use agentee_core::rf::{self, Cx, Matrix, Network};
use agentee_core::sim::{Curve, Reading, SimResult};
use std::f64::consts::PI;

pub enum Attach {
    Measured(Network),
    Series { r: f64, l: f64, c: Option<f64> },
}

pub struct Part {
    pub name: String,
    pub port: usize,
    pub attach: Attach,
}

fn board_s(board: &SimResult, f: f64, z0: f64) -> Option<Matrix> {
    let n = board.ports.len();
    let fr = &board.freqs;
    let at = |k: usize| -> Matrix {
        (0..n)
            .map(|i| (0..n).map(|j| Cx::new(board.s[i][j][k][0], board.s[i][j][k][1])).collect())
            .collect()
    };
    if f >= fr[0] {
        let k = fr.partition_point(|x| *x < f);
        if k >= fr.len() {
            return None;
        }
        if k == 0 || fr[k] == f {
            return Some(at(k));
        }
        let t = (f - fr[k - 1]) / (fr[k] - fr[k - 1]);
        let (a, b) = (at(k - 1), at(k));
        return Some(
            (0..n).map(|i| (0..n).map(|j| a[i][j] * (1.0 - t) + b[i][j] * t).collect()).collect(),
        );
    }
    let z = to_z(&at(0), z0)?;
    let scale = f / fr[0];
    let zf: Matrix =
        z.iter().map(|row| row.iter().map(|c| Cx::new(c.re, c.im * scale)).collect()).collect();
    to_s(&zf, z0)
}

fn identity(n: usize) -> Matrix {
    (0..n).map(|i| (0..n).map(|j| if i == j { Cx::ONE } else { Cx::ZERO }).collect()).collect()
}

pub fn to_z(s: &Matrix, z0: f64) -> Option<Matrix> {
    let n = s.len();
    let i = identity(n);
    let plus: Matrix = (0..n).map(|a| (0..n).map(|b| i[a][b] + s[a][b]).collect()).collect();
    let minus: Matrix = (0..n).map(|a| (0..n).map(|b| i[a][b] - s[a][b]).collect()).collect();
    let x = rf::solve(minus, identity(n))?;
    Some(rf::mul(&plus, &x).into_iter().map(|r| r.into_iter().map(|c| c * z0).collect()).collect())
}

pub fn to_s(z: &Matrix, z0: f64) -> Option<Matrix> {
    let n = z.len();
    let zn: Matrix = z.iter().map(|r| r.iter().map(|c| *c * (1.0 / z0)).collect()).collect();
    let i = identity(n);
    let minus: Matrix = (0..n).map(|a| (0..n).map(|b| zn[a][b] - i[a][b]).collect()).collect();
    let plus: Matrix = (0..n).map(|a| (0..n).map(|b| zn[a][b] + i[a][b]).collect()).collect();
    let inv = rf::solve(plus, identity(n))?;
    Some(rf::mul(&minus, &inv))
}

fn gamma(z: Cx, z0: f64) -> Cx {
    (z - Cx::new(z0, 0.0)) / (z + Cx::new(z0, 0.0))
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    name: &str,
    board: &SimResult,
    z0: f64,
    sinks: &[usize],
    parts: &[Part],
    freqs: &[f64],
    target: Option<f64>,
    spec_hash: u64,
) -> Result<SimResult, String> {
    let started = std::time::Instant::now();
    let nb = board.ports.len();
    for p in 0..nb {
        if !sinks.contains(&p) && !parts.iter().any(|x| x.port == p) {
            return Err(format!("port {} is neither a sink nor carries a part", board.ports[p]));
        }
    }
    let mut kept = Vec::new();
    let mut zs: Vec<Vec<f64>> = vec![Vec::new(); sinks.len()];
    let mut out_s: Vec<Matrix> = Vec::new();
    for &f in freqs {
        let Some(b) = board_s(board, f, z0) else { continue };
        let w = 2.0 * PI * f;
        let mut blocks: Vec<Matrix> = vec![b];
        let mut pairs = Vec::new();
        let mut ok = true;
        for (k, p) in parts.iter().enumerate() {
            let g = match &p.attach {
                Attach::Measured(net) => match net.at(f) {
                    Some(m) => m[0][0],
                    None => {
                        ok = false;
                        break;
                    }
                },
                Attach::Series { r, l, c } => {
                    let mut z = Cx::new(*r, w * l);
                    if let Some(c) = c {
                        z = z + Cx::new(0.0, -1.0 / (w * c));
                    }
                    gamma(z, z0)
                }
            };
            blocks.push(vec![vec![g]]);
            pairs.push((p.port, nb + k));
        }
        if !ok {
            continue;
        }
        let refs: Vec<&Matrix> = blocks.iter().collect();
        let all = rf::block_diag(&refs);
        let (s, ext) = rf::connect(&all, &pairs).ok_or("the connection is singular")?;
        let order: Vec<usize> =
            sinks.iter().map(|p| ext.iter().position(|e| e == p).unwrap()).collect();
        let s: Matrix = order.iter().map(|i| order.iter().map(|j| s[*i][*j]).collect()).collect();
        let z = to_z(&s, z0).ok_or("the sink matrix is singular")?;
        for (i, row) in zs.iter_mut().enumerate() {
            row.push(z[i][i].abs());
        }
        kept.push(f);
        out_s.push(s);
    }
    if kept.is_empty() {
        return Err("no frequency has data for every part".into());
    }
    let n = sinks.len();
    let mut curves: Vec<Curve> = sinks
        .iter()
        .zip(&zs)
        .map(|(p, v)| Curve {
            name: format!("|Z| at {}", board.ports[*p]),
            unit: "ohm".into(),
            values: v.iter().map(|x| Some(*x)).collect(),
        })
        .collect();
    let mut readings = Vec::new();
    if let Some(t) = target {
        curves.push(Curve {
            name: "target".into(),
            unit: "ohm".into(),
            values: vec![Some(t); kept.len()],
        });
    }
    let mhz = |f: f64| {
        if f >= 1e9 { format!("{:.2} GHz", f / 1e9) } else { format!("{:.3} MHz", f / 1e6) }
    };
    for (i, p) in sinks.iter().enumerate() {
        let v = &zs[i];
        let (k, zmax) =
            v.iter().enumerate().fold((0, 0.0f64), |a, (k, x)| if *x > a.1 { (k, *x) } else { a });
        let peaks: Vec<String> = (1..v.len().saturating_sub(1))
            .filter(|&k| v[k] > v[k - 1] && v[k] >= v[k + 1])
            .map(|k| (v[k], kept[k]))
            .collect::<Vec<_>>()
            .into_iter()
            .fold(Vec::<(f64, f64)>::new(), |mut acc, x| {
                acc.push(x);
                acc.sort_by(|a, b| b.0.total_cmp(&a.0));
                acc.truncate(3);
                acc
            })
            .into_iter()
            .map(|(z, f)| format!("{:.1} mohm at {}", z * 1e3, mhz(f)))
            .collect();
        let over = target.map(|t| v.iter().filter(|x| **x > t).count());
        readings.push(Reading {
            label: format!("peak |Z| at {}", board.ports[*p]),
            value: zmax * 1e3,
            unit: "mohm".into(),
            detail: match (target, over) {
                (Some(t), Some(o)) if o > 0 => format!(
                    "{:.1}x the {:.1} mohm target at {}; {} of {} points over",
                    zmax / t,
                    t * 1e3,
                    mhz(kept[k]),
                    o,
                    v.len()
                ),
                (Some(t), _) => format!("under the {:.1} mohm target everywhere", t * 1e3),
                _ => format!("at {}", mhz(kept[k])),
            },
        });
        if !peaks.is_empty() {
            readings.push(Reading {
                label: format!("anti-resonances at {}", board.ports[*p]),
                value: peaks.len() as f64,
                unit: String::new(),
                detail: peaks.join(", "),
            });
        }
    }
    let s = (0..n)
        .map(|i| (0..n).map(|j| out_s.iter().map(|m| [m[i][j].re, m[i][j].im]).collect()).collect())
        .collect();
    Ok(SimResult {
        name: name.into(),
        freqs: kept,
        ports: sinks.iter().map(|p| board.ports[*p].clone()).collect(),
        s,
        excited: vec![true; n],
        cells: 0,
        grid: [0; 3],
        steps: Vec::new(),
        dt: 0.0,
        seconds: started.elapsed().as_secs_f64(),
        device: "cpu".into(),
        spec_hash,
        maps: Vec::new(),
        readings,
        curves,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn junction(ports: usize, freqs: &[f64]) -> SimResult {
        let v = 2.0 / ports as f64;
        let s = (0..ports)
            .map(|i| {
                (0..ports)
                    .map(|j| vec![[if i == j { v - 1.0 } else { v }, 0.0]; freqs.len()])
                    .collect()
            })
            .collect();
        SimResult {
            name: "j".into(),
            freqs: freqs.to_vec(),
            ports: (0..ports).map(|k| format!("P{k}")).collect(),
            s,
            excited: vec![true; ports],
            cells: 0,
            grid: [0; 3],
            steps: vec![],
            dt: 0.0,
            seconds: 0.0,
            device: String::new(),
            spec_hash: 0,
            maps: vec![],
            readings: vec![],
            curves: vec![],
        }
    }

    #[test]
    fn a_decap_beside_a_vrm_matches_the_parallel_impedance() {
        let fb = vec![1e3, 1e10];
        let board = junction(3, &fb);
        let parts = vec![
            Part {
                name: "vrm".into(),
                port: 1,
                attach: Attach::Series { r: 2e-3, l: 20e-9, c: None },
            },
            Part {
                name: "c".into(),
                port: 2,
                attach: Attach::Series { r: 5e-3, l: 0.5e-9, c: Some(10e-6) },
            },
        ];
        let freqs: Vec<f64> = (0..60).map(|k| 1e4 * 10f64.powf(k as f64 / 12.0)).collect();
        let r = run("t", &board, 1.0, &[0], &parts, &freqs, Some(0.01), 0).unwrap();
        for (k, f) in r.freqs.iter().enumerate() {
            let w = 2.0 * PI * f;
            let a = Cx::new(2e-3, w * 20e-9);
            let b = Cx::new(5e-3, w * 0.5e-9 - 1.0 / (w * 10e-6));
            let want = (a * b / (a + b)).abs();
            let got = r.curves[0].values[k].unwrap();
            assert!((got - want).abs() / want < 1e-6, "{f}: {got} {want}");
        }
        assert!(r.readings[0].detail.contains("target"));
    }
}

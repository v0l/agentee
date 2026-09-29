use agentee_core::rf::{self, Cx, Matrix, Network};
use agentee_core::sim::{Reading, SimResult};

pub struct Placed {
    pub name: String,
    pub net: Network,
    pub ports: Vec<usize>,
}

fn board_at(r: &SimResult, k: usize) -> Matrix {
    let n = r.ports.len();
    (0..n).map(|i| (0..n).map(|j| Cx::new(r.s[i][j][k][0], r.s[i][j][k][1])).collect()).collect()
}

fn mhz(f: f64) -> String {
    if f >= 1e9 { format!("{:.3} GHz", f / 1e9) } else { format!("{:.0} MHz", f / 1e6) }
}

pub fn run(
    name: &str,
    board: &SimResult,
    z0: &[f64],
    devices: &[Placed],
    spec_hash: u64,
) -> Result<SimResult, String> {
    let started = std::time::Instant::now();
    let nb = board.ports.len();
    for d in devices {
        for &p in &d.ports {
            if (z0[p] - d.net.z0).abs() > 1e-9 {
                return Err(format!(
                    "{} is referenced to {} ohm and board port {} to {} ohm",
                    d.name, d.net.z0, board.ports[p], z0[p]
                ));
            }
        }
    }
    let mut freqs = Vec::new();
    let mut out: Vec<Matrix> = Vec::new();
    let mut ext = Vec::new();
    for (k, f) in board.freqs.iter().enumerate() {
        let Some(parts) = devices.iter().map(|d| d.net.at(*f)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let b = board_at(board, k);
        let mut blocks: Vec<&Matrix> = vec![&b];
        blocks.extend(parts.iter());
        let all = rf::block_diag(&blocks);
        let mut pairs = Vec::new();
        let mut offset = nb;
        for d in devices {
            for (q, &p) in d.ports.iter().enumerate() {
                pairs.push((p, offset + q));
            }
            offset += d.ports.len();
        }
        let (s, e) = rf::connect(&all, &pairs)
            .ok_or_else(|| format!("the connection is singular at {}", mhz(*f)))?;
        ext = e;
        freqs.push(*f);
        out.push(s);
    }
    if freqs.is_empty() {
        return Err("the devices have no data inside the board's band".into());
    }
    let ne = ext.len();
    let s: Vec<Vec<Vec<[f64; 2]>>> = (0..ne)
        .map(|i| (0..ne).map(|j| out.iter().map(|m| [m[i][j].re, m[i][j].im]).collect()).collect())
        .collect();
    let mut readings = Vec::new();
    if ne == 2 {
        readings = two_port_readings(&freqs, &out);
    }
    if freqs.len() < board.freqs.len() {
        readings.push(Reading {
            label: "band".into(),
            value: (freqs.len() as f64) / board.freqs.len() as f64 * 100.0,
            unit: "%".into(),
            detail: format!("devices cover {} to {}", mhz(freqs[0]), mhz(*freqs.last().unwrap())),
        });
    }
    Ok(SimResult {
        name: name.into(),
        ports: ext.iter().map(|&p| board.ports[p].clone()).collect(),
        s,
        excited: vec![true; ne],
        freqs,
        cells: 0,
        grid: [0; 3],
        steps: Vec::new(),
        dt: 0.0,
        seconds: started.elapsed().as_secs_f64(),
        device: "cpu".into(),
        spec_hash,
        maps: Vec::new(),
        readings,
    })
}

fn two_port_readings(freqs: &[f64], s: &[Matrix]) -> Vec<Reading> {
    let pick = |f: &dyn Fn(&Matrix) -> f64, max: bool| {
        let mut best = (if max { f64::MIN } else { f64::MAX }, 0usize);
        for (k, m) in s.iter().enumerate() {
            let v = f(m);
            if (max && v > best.0) || (!max && v < best.0) {
                best = (v, k);
            }
        }
        (best.0, freqs[best.1])
    };
    let mut r = Vec::new();
    let (gmax, fmax) = pick(&|m| m[1][0].db(), true);
    let (gmin, fmin) = pick(&|m| m[1][0].db(), false);
    r.push(Reading {
        label: "gain peak".into(),
        value: gmax,
        unit: "dB".into(),
        detail: format!("S21 at {}", mhz(fmax)),
    });
    r.push(Reading {
        label: "gain low".into(),
        value: gmin,
        unit: "dB".into(),
        detail: format!("S21 at {}", mhz(fmin)),
    });
    let (s11, f11) = pick(&|m| m[0][0].db(), true);
    r.push(Reading {
        label: "input match".into(),
        value: s11,
        unit: "dB".into(),
        detail: format!("S11 at {}", mhz(f11)),
    });
    let (s22, f22) = pick(&|m| m[1][1].db(), true);
    r.push(Reading {
        label: "output match".into(),
        value: s22,
        unit: "dB".into(),
        detail: format!("S22 at {}", mhz(f22)),
    });
    let (iso, fi) = pick(&|m| m[0][1].db(), true);
    r.push(Reading {
        label: "isolation".into(),
        value: iso,
        unit: "dB".into(),
        detail: format!("S12 at {}", mhz(fi)),
    });
    let (mu, fmu) = pick(&|m| rf::stability(m).mu, false);
    let (k, fk) = pick(&|m| rf::stability(m).k, false);
    r.push(Reading {
        label: "stability mu".into(),
        value: mu,
        unit: String::new(),
        detail: format!("{} at {}", if mu > 1.0 { "lowest" } else { "below 1" }, mhz(fmu)),
    });
    r.push(Reading {
        label: "Rollett K".into(),
        value: k,
        unit: String::new(),
        detail: format!("at {}", mhz(fk)),
    });
    r
}

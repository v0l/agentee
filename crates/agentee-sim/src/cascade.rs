use agentee_core::rf::{self, Cx, Matrix, Network, NoiseParams};
use agentee_core::sim::{Curve, DatasheetRow, Reading, SimResult, StageFile, datasheet_at};

pub struct Placed {
    pub name: String,
    pub net: Network,
    pub ports: Vec<usize>,
    pub datasheet: Vec<DatasheetRow>,
}

pub struct Budget {
    pub kelvin: f64,
    pub bandwidth: Option<f64>,
    pub report: Vec<f64>,
    pub after: Option<StageFile>,
}

fn board_at(r: &SimResult, k: usize) -> Matrix {
    let n = r.ports.len();
    (0..n).map(|i| (0..n).map(|j| Cx::new(r.s[i][j][k][0], r.s[i][j][k][1])).collect()).collect()
}

fn mhz(f: f64) -> String {
    if f >= 1e9 { format!("{:.3} GHz", f / 1e9) } else { format!("{:.0} MHz", f / 1e6) }
}

fn is_passive(s: &Matrix) -> bool {
    s.iter().flatten().all(|c| c.abs() <= 1.0 + 1e-6)
}

fn device_noise(d: &Placed, s: &Matrix, f: f64, kelvin: f64) -> Option<Matrix> {
    if is_passive(s) {
        return Some(rf::passive_noise(s, kelvin));
    }
    if d.net.ports != 2 {
        return None;
    }
    let np = d.net.noise_at(f).or_else(|| {
        datasheet_at(&d.datasheet, f, |r| r.nf).map(|nf| NoiseParams {
            fmin: 10f64.powf(nf / 10.0),
            gamma_opt: Cx::ZERO,
            rn: 0.0,
        })
    })?;
    rf::active_noise(s, np, d.net.z0)
}

struct Point {
    s: Matrix,
    nf: Option<f64>,
    iip3: Option<f64>,
    ip1: Option<f64>,
}

fn solve_point(
    board: &Matrix,
    parts: &[Matrix],
    devices: &[Placed],
    f: f64,
    budget: &Budget,
) -> Result<Point, String> {
    let nb = board.len();
    let mut blocks: Vec<&Matrix> = vec![board];
    blocks.extend(parts.iter());
    let all = rf::block_diag(&blocks);
    let mut pairs = Vec::new();
    let mut offset = nb;
    let mut outputs = Vec::new();
    for d in devices {
        for (q, &p) in d.ports.iter().enumerate() {
            pairs.push((p, offset + q));
        }
        if d.ports.len() == 2 {
            outputs.push(offset + 1);
        } else {
            outputs.push(usize::MAX);
        }
        offset += d.ports.len();
    }
    let j = rf::join(&all, &pairs)
        .ok_or_else(|| format!("the connection is singular at {}", mhz(f)))?;
    let mut noise_blocks = vec![rf::passive_noise(board, budget.kelvin)];
    let mut known = true;
    for (d, s) in devices.iter().zip(parts) {
        match device_noise(d, s, f, budget.kelvin) {
            Some(c) => noise_blocks.push(c),
            None => {
                known = false;
                noise_blocks.push(vec![vec![Cx::ZERO; s.len()]; s.len()]);
            }
        }
    }
    let refs: Vec<&Matrix> = noise_blocks.iter().collect();
    let cs = rf::block_diag(&refs);
    let nf = (known && j.s.len() == 2).then(|| {
        let c = rf::mul(&rf::mul(&j.transfer, &cs), &rf::adjoint(&j.transfer));
        rf::noise_figure(j.s[1][0], c[1][1].re)
    });
    let (mut inv_ip3, mut inv_p1, mut have3, mut have1) = (0.0, 0.0, false, false);
    if j.s.len() == 2 {
        for (d, &out) in devices.iter().zip(&outputs) {
            let Some(k) = j.internal.iter().position(|x| *x == out) else { continue };
            let g = j.response[k][0].norm2();
            if let Some(oip3) = datasheet_at(&d.datasheet, f, |r| r.oip3) {
                inv_ip3 += g / 10f64.powf(oip3 / 10.0);
                have3 = true;
            }
            if let Some(p1) = datasheet_at(&d.datasheet, f, |r| r.p1db) {
                inv_p1 += g / 10f64.powf(p1 / 10.0);
                have1 = true;
            }
        }
    }
    Ok(Point {
        s: j.s,
        nf,
        iip3: have3.then(|| 10.0 * (1.0 / inv_ip3).log10()),
        ip1: have1.then(|| 10.0 * (1.0 / inv_p1).log10() + 1.0),
    })
}

fn with_after(p: &Point, after: &Option<StageFile>) -> (Option<f64>, Option<f64>) {
    let Some(a) = after else { return (p.nf, p.iip3) };
    let g = p.s[1][0].norm2();
    let nf = p.nf.map(|f1| f1 + (10f64.powf(a.nf / 10.0) - 1.0) / g);
    let iip3 = match (p.iip3, a.iip3) {
        (Some(x), Some(y)) => {
            let inv = 1.0 / 10f64.powf(x / 10.0) + g / 10f64.powf(y / 10.0);
            Some(10.0 * (1.0 / inv).log10())
        }
        (x, None) => x,
        (None, Some(y)) => Some(y - 10.0 * g.log10()),
    };
    (nf, iip3)
}

pub fn run(
    name: &str,
    board: &SimResult,
    z0: &[f64],
    devices: &[Placed],
    budget: &Budget,
    spec_hash: u64,
) -> Result<SimResult, String> {
    let started = std::time::Instant::now();
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
    let mut points: Vec<Point> = Vec::new();
    for (k, f) in board.freqs.iter().enumerate() {
        let Some(parts) = devices.iter().map(|d| d.net.at(*f)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        points.push(solve_point(&board_at(board, k), &parts, devices, *f, budget)?);
        freqs.push(*f);
    }
    if freqs.is_empty() {
        return Err("the devices have no data inside the board's band".into());
    }
    let ext = {
        let parts: Vec<Matrix> = devices.iter().map(|d| d.net.at(freqs[0]).unwrap()).collect();
        let nb = board.ports.len();
        let mut pairs = Vec::new();
        let mut offset = nb;
        for d in devices {
            for (q, &p) in d.ports.iter().enumerate() {
                pairs.push((p, offset + q));
            }
            offset += d.ports.len();
        }
        let b = board_at(board, 0);
        let mut blocks: Vec<&Matrix> = vec![&b];
        blocks.extend(parts.iter());
        rf::connect(&rf::block_diag(&blocks), &pairs).map(|x| x.1).unwrap_or_default()
    };
    let out: Vec<Matrix> = points.iter().map(|p| p.s.clone()).collect();
    let ne = ext.len();
    let s: Vec<Vec<Vec<[f64; 2]>>> = (0..ne)
        .map(|i| (0..ne).map(|j| out.iter().map(|m| [m[i][j].re, m[i][j].im]).collect()).collect())
        .collect();
    let mut readings = Vec::new();
    let mut curves = Vec::new();
    if ne == 2 {
        readings = two_port_readings(&freqs, &out);
        let system: Vec<(Option<f64>, Option<f64>)> =
            points.iter().map(|p| with_after(p, &budget.after)).collect();
        let nf_db: Vec<Option<f64>> =
            system.iter().map(|(f, _)| f.map(|v| 10.0 * v.log10())).collect();
        curves.push(Curve {
            name: "noise figure".into(),
            unit: "dB".into(),
            values: nf_db.clone(),
        });
        curves.push(Curve {
            name: "stability mu".into(),
            unit: String::new(),
            values: out.iter().map(|m| Some(rf::stability(m).mu)).collect(),
        });
        let known: Vec<(f64, f64)> =
            freqs.iter().zip(&nf_db).filter_map(|(f, v)| v.map(|v| (*f, v))).collect();
        if let (Some(lo), Some(hi)) = (
            known.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1)),
            known.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1)),
        ) {
            readings.push(Reading {
                label: "noise figure".into(),
                value: lo.1,
                unit: "dB".into(),
                detail: format!("best at {}, worst {:.2} dB at {}", mhz(lo.0), hi.1, mhz(hi.0)),
            });
        }
        let report: Vec<f64> = if budget.report.is_empty() {
            let mut r: Vec<f64> =
                devices.iter().flat_map(|d| d.datasheet.iter().map(|x| x.freq)).collect();
            r.sort_by(f64::total_cmp);
            r.dedup();
            r
        } else {
            budget.report.clone()
        };
        for f in report {
            let Some(k) = (0..freqs.len())
                .min_by(|a, b| (freqs[*a] - f).abs().total_cmp(&(freqs[*b] - f).abs()))
            else {
                continue;
            };
            if (freqs[k] - f).abs() > 0.02 * f {
                continue;
            }
            let p = &points[k];
            let g = p.s[1][0].db();
            let (nf, iip3) = system[k];
            let mut parts = vec![format!("gain {g:.1} dB")];
            if let Some(nf) = nf {
                let nf_db = 10.0 * nf.log10();
                parts.push(format!("NF {nf_db:.2} dB"));
                let floor = 10.0 * (rf::K_B * rf::T0 * 1e3).log10() + nf_db;
                match budget.bandwidth {
                    Some(bw) => {
                        let floor = floor + 10.0 * bw.log10();
                        parts.push(format!("floor {floor:.1} dBm"));
                        if let Some(iip3) = iip3 {
                            parts.push(format!("SFDR {:.1} dB", 2.0 / 3.0 * (iip3 - floor)));
                        }
                    }
                    None => parts.push(format!("floor {floor:.1} dBm/Hz")),
                }
            }
            if let Some(iip3) = iip3 {
                parts.push(format!("IIP3 {iip3:.1} dBm, OIP3 {:.1}", iip3 + g));
            }
            if let Some(ip1) = p.ip1 {
                parts.push(format!("P1dB out {:.1} dBm", ip1 + g - 1.0));
            }
            readings.push(Reading {
                label: format!("at {}", mhz(freqs[k])),
                value: g,
                unit: "dB".into(),
                detail: parts.join(", "),
            });
        }
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
        curves,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn board(freqs: &[f64], loss_db: f64) -> SimResult {
        let a = 10f64.powf(-loss_db / 20.0);
        let mut s = vec![vec![vec![[0.0, 0.0]; freqs.len()]; 4]; 4];
        for (i, j, v) in [(0, 1, a), (1, 0, a), (2, 3, 1.0), (3, 2, 1.0)] {
            s[i][j].iter_mut().for_each(|c| *c = [v, 0.0]);
        }
        SimResult {
            name: "b".into(),
            freqs: freqs.to_vec(),
            ports: ["IN", "A", "B", "OUT"].map(String::from).to_vec(),
            s,
            excited: vec![true; 4],
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
    fn a_pad_ahead_of_an_amplifier_follows_friis_and_refers_ip3() {
        let freqs = vec![1e9, 2e9];
        let amp = vec![vec![Cx::ZERO, Cx::ZERO], vec![Cx::new(10.0, 0.0), Cx::ZERO]];
        let net = Network {
            ports: 2,
            z0: 50.0,
            freqs: freqs.clone(),
            s: vec![amp.clone(), amp],
            noise: vec![],
        };
        let row = |f: f64| DatasheetRow {
            freq: f,
            nf: Some(10.0 * 2f64.log10()),
            oip3: Some(30.0),
            p1db: Some(20.0),
        };
        let dev = Placed {
            name: "amp".into(),
            net,
            ports: vec![1, 2],
            datasheet: vec![row(1e9), row(2e9)],
        };
        let pad_db = 10.0 * 2f64.log10();
        let budget = Budget { kelvin: rf::T0, bandwidth: None, report: vec![], after: None };
        let r = run("t", &board(&freqs, pad_db), &[50.0; 4], &[dev], &budget, 0).unwrap();
        let nf = r.curves[0].values[0].unwrap();
        assert!((nf - 10.0 * 4f64.log10()).abs() < 1e-9, "{nf}");
        let at = r.readings.iter().find(|x| x.label.starts_with("at 1.000")).unwrap();
        assert!(at.detail.contains("IIP3 13.0 dBm"), "{}", at.detail);
        assert!(at.detail.contains("P1dB out 20.0 dBm"), "{}", at.detail);
    }
}

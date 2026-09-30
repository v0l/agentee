use agentee_core::rf::Cx;
use agentee_core::sim::{ChannelResult, Eye, Reading};
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct Ctle {
    pub dc_db: f64,
    pub zero: f64,
    pub poles: Vec<f64>,
}

impl Ctle {
    pub fn at(&self, f: f64) -> Cx {
        let jf = |c: f64| Cx::new(1.0, f / c);
        let mut h = jf(self.zero);
        for p in &self.poles {
            h = h / jf(*p);
        }
        h * 10f64.powf(self.dc_db / 20.0)
    }
}

#[derive(Clone, Debug)]
pub struct Params {
    pub bit_rate: f64,
    pub rise: f64,
    pub swing: f64,
    pub prbs: u32,
    pub ctle: Option<Ctle>,
    pub dfe_taps: usize,
}

pub const PHASES: usize = 128;
pub const BINS: usize = 160;

pub fn prbs(order: u32) -> Vec<bool> {
    let taps: (u32, u32) = match order {
        7 => (7, 6),
        9 => (9, 5),
        11 => (11, 9),
        15 => (15, 14),
        _ => (7, 6),
    };
    let n = taps.0;
    let mut state: u32 = (1 << n) - 1;
    let len = (1usize << n) - 1;
    (0..len)
        .map(|_| {
            let bit = ((state >> (taps.0 - 1)) ^ (state >> (taps.1 - 1))) & 1;
            state = ((state << 1) | bit) & ((1 << n) - 1);
            bit == 1
        })
        .collect()
}

pub fn run(name: &str, freqs: &[f64], h: &[Cx], p: &Params, spec_hash: u64) -> ChannelResult {
    let started = std::time::Instant::now();
    let fmax = *freqs.last().unwrap();
    let ui = 1.0 / p.bit_rate;
    let mut warnings = Vec::new();
    let floor = 1.3 / fmax;
    let rise = if p.rise < floor {
        warnings.push(format!(
            "the {:.0} ps edge needs data past {:.1} GHz, run at {:.0} ps instead",
            p.rise * 1e12,
            fmax / 1e9,
            floor * 1e12
        ));
        floor
    } else {
        p.rise
    };
    let eq: Vec<Cx> = freqs
        .iter()
        .zip(h)
        .map(|(f, c)| match &p.ctle {
            Some(ct) => *c * ct.at(*f),
            None => *c,
        })
        .collect();
    let per_ui = 64usize;
    let dt = ui / per_ui as f64;
    let spacing = freqs.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min).min(freqs[0]);
    let period = 1.0 / spacing;
    let span_ui = ((0.9 * period / ui).floor() as usize).saturating_sub(2).clamp(4, 64);
    if (span_ui as f64 + 2.0) * ui > 0.9 * period {
        warnings.push(format!(
            "the S-parameters step {:.1} MHz, so the pulse can only be followed for {:.0} ns before it wraps",
            spacing / 1e6,
            period * 1e9
        ));
    }
    let st = agentee_core::sparam::step_with(
        freqs,
        &eq,
        rise,
        Some((span_ui + 2) as f64 * ui),
        Some(dt),
    );
    let n = st.value.len();
    let pulse: Vec<f64> = (0..n)
        .map(|m| st.value[m] - if m >= per_ui { st.value[m - per_ui] } else { 0.0 })
        .collect();
    let peak = (0..n).max_by(|a, b| pulse[*a].total_cmp(&pulse[*b])).unwrap_or(0);
    let top = pulse[peak] * 0.999;
    let first = (0..=peak).rev().take_while(|m| pulse[*m] >= top).last().unwrap_or(peak);
    let last = (peak..n).take_while(|m| pulse[*m] >= top).last().unwrap_or(peak);
    let main = (first + last) / 2;
    let amp = p.swing / 2.0;
    let sample = |idx: isize| -> f64 {
        if idx < 0 || idx as usize >= n { 0.0 } else { pulse[idx as usize] }
    };
    let pre = main / per_ui;
    let post = (n - main) / per_ui;
    let cursors = |phase: isize| -> Vec<f64> {
        (-(pre as isize)..=post as isize)
            .map(|k| sample(main as isize + phase + k * per_ui as isize))
            .collect()
    };
    let centre = cursors(0);
    let taps: Vec<f64> =
        (1..=p.dfe_taps).map(|k| centre.get(pre + k).copied().unwrap_or(0.0)).collect();
    let worst = |phase: isize| -> f64 {
        let c = cursors(phase);
        let main = c[pre];
        let isi: f64 = c
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != pre)
            .map(|(k, v)| {
                let tap = if k > pre && k - pre <= taps.len() { taps[k - pre - 1] } else { 0.0 };
                (v - tap).abs()
            })
            .sum();
        2.0 * amp * (main - isi)
    };
    let bits = prbs(p.prbs);
    let symbols: Vec<f64> = bits.iter().map(|b| if *b { amp } else { -amp }).collect();
    let nb = symbols.len();
    let mut ones_min = vec![f64::MAX; per_ui];
    let mut zeros_max = vec![f64::MIN; per_ui];
    let phase_of = |j: usize| j as isize - (per_ui / 2) as isize;
    let all: Vec<Vec<f64>> = (0..per_ui).map(|j| cursors(phase_of(j))).collect();
    let mut samples = vec![0.0f64; nb * per_ui];
    for bit in 0..nb {
        for (j, c) in all.iter().enumerate() {
            let mut v = 0.0;
            for (k, ck) in c.iter().enumerate() {
                let shift = k as isize - pre as isize;
                v += symbols[(bit as isize - shift).rem_euclid(nb as isize) as usize] * ck;
            }
            for (t, tap) in taps.iter().enumerate() {
                v -=
                    symbols[(bit as isize - 1 - t as isize).rem_euclid(nb as isize) as usize] * tap;
            }
            if symbols[bit] > 0.0 {
                ones_min[j] = ones_min[j].min(v);
            } else {
                zeros_max[j] = zeros_max[j].max(v);
            }
            samples[bit * per_ui + j] = v;
        }
    }
    let reach = samples.iter().fold(amp, |a, v| a.max(v.abs())) * 1.15;
    let (v_lo, v_hi) = (-reach, reach);
    let mut counts = vec![0u32; PHASES * BINS];
    for (i, v) in samples.iter().enumerate() {
        let j = i % per_ui;
        let row = ((v_hi - v) / (v_hi - v_lo) * BINS as f64).floor();
        if row < 0.0 || row as usize >= BINS {
            continue;
        }
        for half in 0..2 {
            let col = (half * per_ui + j) * PHASES / (2 * per_ui);
            counts[row as usize * PHASES + col] += 1;
        }
    }
    let heights: Vec<f64> = (0..per_ui).map(|j| ones_min[j] - zeros_max[j]).collect();
    let best = (0..per_ui).max_by(|a, b| heights[*a].total_cmp(&heights[*b])).unwrap_or(per_ui / 2);
    let height = heights[best].max(0.0);
    let width = heights.iter().filter(|h| **h > 0.0).count() as f64 * dt;
    let pda: f64 = (0..per_ui).map(|j| worst(phase_of(j))).fold(f64::MIN, f64::max).max(0.0);
    let nyquist = p.bit_rate / 2.0;
    let il = {
        let k = freqs.partition_point(|f| *f < nyquist).min(freqs.len() - 1);
        20.0 * h[k].abs().max(1e-12).log10()
    };
    let mv = |v: f64| v * 1e3;
    let mut readings = vec![
        Reading {
            label: "eye height".into(),
            value: mv(height),
            unit: "mV".into(),
            detail: format!("PRBS{} at the best phase, {} mV swing", p.prbs, mv(p.swing)),
        },
        Reading {
            label: "eye width".into(),
            value: width * 1e12,
            unit: "ps".into(),
            detail: format!("{:.2} UI", width / ui),
        },
        Reading {
            label: "worst-case height".into(),
            value: mv(pda),
            unit: "mV".into(),
            detail: "peak distortion, every cursor against the main".into(),
        },
        Reading {
            label: "loss at Nyquist".into(),
            value: il,
            unit: "dB".into(),
            detail: format!("{:.2} GHz, before CTLE", nyquist / 1e9),
        },
        Reading {
            label: "main cursor".into(),
            value: mv(2.0 * amp * centre[pre]),
            unit: "mV".into(),
            detail: format!("{} pre and {} post cursors", pre, post),
        },
    ];
    for w in &warnings {
        readings.push(Reading {
            label: "note".into(),
            value: 0.0,
            unit: String::new(),
            detail: w.clone(),
        });
    }
    let t0 = st.time_ps[main];
    let from = main.saturating_sub(4 * per_ui);
    let to = (main + 24 * per_ui).min(n);
    ChannelResult {
        name: name.into(),
        kind: "channel".into(),
        spec_hash,
        layout_hash: None,
        bit_rate: p.bit_rate,
        ui_ps: ui * 1e12,
        rise_ps: rise * 1e12,
        pulse_time_ps: st.time_ps[from..to].iter().map(|t| t - t0).collect(),
        pulse_mv: pulse[from..to].iter().map(|v| mv(2.0 * amp * v)).collect(),
        eye: Eye { phases: PHASES, bins: BINS, v_min_mv: mv(v_lo), v_max_mv: mv(v_hi), counts },
        readings,
        seconds: started.elapsed().as_secs_f64(),
    }
}

pub fn ideal_rc(freqs: &[f64], fc: f64) -> Vec<Cx> {
    freqs.iter().map(|f| Cx::ONE / Cx::new(1.0, f / fc)).collect()
}

pub fn tau(fc: f64) -> f64 {
    1.0 / (2.0 * PI * fc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn freqs() -> Vec<f64> {
        (1..=4000).map(|k| k as f64 * 10e6).collect()
    }

    #[test]
    fn a_lossless_channel_opens_the_eye_to_the_full_swing() {
        let f = freqs();
        let h = vec![Cx::ONE; f.len()];
        let p =
            Params { bit_rate: 2e9, rise: 40e-12, swing: 0.8, prbs: 7, ctle: None, dfe_taps: 0 };
        let r = run("t", &f, &h, &p, 0);
        let eh = r.readings[0].value;
        assert!((eh - 800.0).abs() < 8.0, "{eh}");
        assert!(r.readings[1].value > 0.8 * 500.0, "{}", r.readings[1].value);
    }

    #[test]
    fn an_rc_channel_matches_the_closed_form_worst_case() {
        let f: Vec<f64> = (1..=10000).map(|k| k as f64 * 10e6).collect();
        let (ui, t) = (500e-12, 250e-12);
        let fc = 1.0 / (2.0 * PI * t);
        let h = ideal_rc(&f, fc);
        let p = Params {
            bit_rate: 1.0 / ui,
            rise: 13e-12,
            swing: 1.0,
            prbs: 9,
            ctle: None,
            dfe_taps: 0,
        };
        let r = run("t", &f, &h, &p, 0);
        let q = (-ui / t).exp();
        let want = 1000.0 * (1.0 - 2.0 * q);
        let pda = r.readings[2].value;
        eprintln!(
            "worst case {pda:.1} mV, closed form {want:.1} mV, PRBS {:.1} mV, main {:.1}",
            r.readings[0].value, r.readings[4].value
        );
        assert!((pda - want).abs() / want < 0.015, "{pda} {want}");
        let with_dfe = run("t", &f, &h, &Params { dfe_taps: 12, ..p.clone() }, 0);
        let main = 1000.0 * (1.0 - q);
        assert!(
            (with_dfe.readings[2].value - main).abs() / main < 0.015,
            "{}",
            with_dfe.readings[2].value
        );
    }

    #[test]
    fn prbs7_has_its_period_and_balance() {
        let b = prbs(7);
        assert_eq!(b.len(), 127);
        assert_eq!(b.iter().filter(|x| **x).count(), 64);
    }
}

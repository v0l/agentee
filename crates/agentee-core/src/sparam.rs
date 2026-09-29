use crate::rf::{Cx, Matrix};
use serde::Serialize;
use std::f64::consts::PI;

fn resample(freqs: &[f64], s: &[Cx], df: f64, n: usize) -> Vec<Cx> {
    let (f0, f1) = (freqs[0], freqs.get(1).copied().unwrap_or(freqs[0] * 2.0));
    let dc = if freqs.len() >= 2 {
        let slope = (s[1].re - s[0].re) / (f1 - f0);
        Cx::new((s[0].re - slope * f0).clamp(-1.0, 1.0), 0.0)
    } else {
        Cx::new(s[0].re, 0.0)
    };
    (0..n)
        .map(|k| {
            let f = k as f64 * df;
            if f <= f0 {
                let t = f / f0;
                return dc * (1.0 - t) + s[0] * t;
            }
            let i = freqs.partition_point(|x| *x < f).min(freqs.len() - 1);
            if i == 0 {
                return s[0];
            }
            let t = ((f - freqs[i - 1]) / (freqs[i] - freqs[i - 1])).clamp(0.0, 1.0);
            s[i - 1] * (1.0 - t) + s[i] * t
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct Step {
    pub time_ps: Vec<f64>,
    pub value: Vec<f64>,
    pub rise_ps: f64,
    pub warning: Option<String>,
}

pub fn step(freqs: &[f64], s: &[Cx], rise: f64, span: Option<f64>) -> Step {
    step_with(freqs, s, rise, span, None)
}

pub fn step_with(freqs: &[f64], s: &[Cx], rise: f64, span: Option<f64>, dt: Option<f64>) -> Step {
    let fmax = *freqs.last().unwrap();
    let spacing = freqs.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min);
    let df = freqs[0].min(spacing).max(fmax / 20000.0);
    let n = (fmax / df).floor() as usize + 1;
    let data = resample(freqs, s, df, n);
    let sigma = rise / 2.563;
    let h: Vec<f64> =
        (0..n).map(|k| (-2.0 * PI * PI * sigma * sigma * (k as f64 * df).powi(2)).exp()).collect();
    let warning = (h[n - 1] > 0.01).then(|| {
        format!(
            "a {:.0} ps rise needs data past {:.1} GHz, the result rings",
            rise * 1e12,
            fmax / 1e9
        )
    });
    let window = span.unwrap_or(0.5 / df);
    let dt = dt.unwrap_or((rise / 8.0).min(1.0 / (8.0 * fmax)));
    let steps = ((window / dt).ceil() as usize).min(20000);
    let (mut acc, mut time, mut value) =
        (0.0, Vec::with_capacity(steps), Vec::with_capacity(steps));
    for m in 0..steps {
        let t = (m as f64 - (4.0 * sigma / dt).ceil()) * dt;
        let mut x = data[0].re * h[0];
        for k in 1..n {
            let ph = 2.0 * PI * k as f64 * df * t;
            let e = Cx::new(ph.cos(), ph.sin());
            x += 2.0 * (data[k] * e).re * h[k];
        }
        acc += x * df * dt;
        time.push(t * 1e12);
        value.push(acc);
    }
    Step { time_ps: time, value, rise_ps: rise * 1e12, warning }
}

pub fn tdr_impedance(step: &Step, z0: f64) -> Vec<f64> {
    step.value.iter().map(|r| z0 * (1.0 + r) / (1.0 - r).max(1e-9)).collect()
}

pub fn mixed_mode(s: &Matrix, pos: [usize; 2], neg: [usize; 2]) -> [[Cx; 4]; 4] {
    let n = s.len();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let mut m = vec![vec![Cx::ZERO; n]; 4];
    m[0][pos[0]] = Cx::new(h, 0.0);
    m[0][neg[0]] = Cx::new(-h, 0.0);
    m[1][pos[1]] = Cx::new(h, 0.0);
    m[1][neg[1]] = Cx::new(-h, 0.0);
    m[2][pos[0]] = Cx::new(h, 0.0);
    m[2][neg[0]] = Cx::new(h, 0.0);
    m[3][pos[1]] = Cx::new(h, 0.0);
    m[3][neg[1]] = Cx::new(h, 0.0);
    let ms = crate::rf::mul(&m, s);
    let mt = crate::rf::adjoint(&m);
    let out = crate::rf::mul(&ms, &mt);
    let mut r = [[Cx::ZERO; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            r[i][j] = out[i][j];
        }
    }
    r
}

pub fn passivity(s: &Matrix) -> f64 {
    let ss = crate::rf::mul(&crate::rf::adjoint(s), s);
    let n = ss.len();
    let mut v: Vec<Cx> = (0..n).map(|i| Cx::new(1.0 + i as f64 * 0.01, 0.0)).collect();
    let mut lambda = 0.0;
    for _ in 0..200 {
        let w: Vec<Cx> =
            (0..n).map(|i| (0..n).fold(Cx::ZERO, |a, j| a + ss[i][j] * v[j])).collect();
        let norm = w.iter().map(|c| c.norm2()).sum::<f64>().sqrt();
        if norm < 1e-300 {
            return 0.0;
        }
        lambda = norm;
        v = w.into_iter().map(|c| c * (1.0 / norm)).collect();
    }
    lambda.sqrt()
}

pub fn reciprocity(s: &Matrix) -> f64 {
    let n = s.len();
    (0..n)
        .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
        .map(|(i, j)| (s[i][j] - s[j][i]).abs())
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_s(z: f64, z0: f64, theta: f64) -> (Cx, Cx) {
        let (c, s) = (theta.cos(), theta.sin());
        let a = Cx::new(c, 0.0);
        let b = Cx::new(0.0, z * s);
        let cc = Cx::new(0.0, s / z);
        let d = Cx::new(c, 0.0);
        let r = Cx::new(z0, 0.0);
        let den = a + b / r + cc * r + d;
        ((a + b / r - cc * r - d) / den, Cx::new(2.0, 0.0) / den)
    }

    #[test]
    fn tdr_sees_a_75_ohm_section_between_50_ohm_ports() {
        let freqs: Vec<f64> = (1..=2000).map(|k| k as f64 * 10e6).collect();
        let delay = 200e-12;
        let s11: Vec<Cx> =
            freqs.iter().map(|f| line_s(75.0, 50.0, 2.0 * PI * f * delay).0).collect();
        let st = step(&freqs, &s11, 70e-12, Some(600e-12));
        let z = tdr_impedance(&st, 50.0);
        let at = |ps: f64| z[st.time_ps.iter().position(|t| *t >= ps).unwrap()];
        assert!((at(-100.0) - 50.0).abs() < 0.5, "{}", at(-100.0));
        assert!((at(200.0) - 75.0).abs() < 0.75, "{}", at(200.0));
        assert!(st.warning.is_none());
    }

    #[test]
    fn uncoupled_lines_have_no_mode_conversion() {
        let (r, t) = line_s(55.0, 50.0, 1.1);
        let z = Cx::ZERO;
        let s = vec![vec![r, z, t, z], vec![z, r, z, t], vec![t, z, r, z], vec![z, t, z, r]];
        let m = mixed_mode(&s, [0, 2], [1, 3]);
        assert!((m[1][0] - t).abs() < 1e-12);
        assert!((m[3][2] - t).abs() < 1e-12);
        assert!(m[3][0].abs() < 1e-12 && m[1][2].abs() < 1e-12);
        assert!((passivity(&s) - 1.0).abs() < 1e-9);
        assert!(reciprocity(&s) < 1e-12);
    }
}

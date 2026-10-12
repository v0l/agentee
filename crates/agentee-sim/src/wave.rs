pub struct Window {
    ts: Vec<f64>,
    ys: Vec<f64>,
}

impl Window {
    pub fn new(ts: &[f64], ys: &[f64], from: f64, to: f64) -> Option<Window> {
        let lerp = |t: f64| -> f64 {
            let k = ts.partition_point(|&x| x < t).clamp(1, ts.len() - 1);
            let span = ts[k] - ts[k - 1];
            let u = if span > 0.0 { ((t - ts[k - 1]) / span).clamp(0.0, 1.0) } else { 0.0 };
            ys[k - 1] + (ys[k] - ys[k - 1]) * u
        };
        if ts.len() < 2 {
            return None;
        }
        let from = from.max(ts[0]);
        let to = to.min(ts[ts.len() - 1]);
        if to <= from {
            return None;
        }
        let mut wt = vec![from];
        let mut wy = vec![lerp(from)];
        for (&t, &y) in ts.iter().zip(ys) {
            if t > from && t < to {
                wt.push(t);
                wy.push(y);
            }
        }
        wt.push(to);
        wy.push(lerp(to));
        Some(Window { ts: wt, ys: wy })
    }

    pub fn peak(&self) -> f64 {
        self.ys.iter().copied().fold(f64::MIN, f64::max)
    }

    pub fn trough(&self) -> f64 {
        self.ys.iter().copied().fold(f64::MAX, f64::min)
    }

    fn integral(&self, f: impl Fn(f64) -> f64) -> f64 {
        self.ts
            .windows(2)
            .zip(self.ys.windows(2))
            .map(|(t, y)| (t[1] - t[0]) * (f(y[0]) + f(y[1])) / 2.0)
            .sum()
    }

    fn span(&self) -> f64 {
        self.ts[self.ts.len() - 1] - self.ts[0]
    }

    pub fn mean(&self) -> f64 {
        self.integral(|y| y) / self.span()
    }

    pub fn rms(&self) -> f64 {
        let m = self.mean();
        (self.integral(|y| (y - m) * (y - m)) / self.span()).max(0.0).sqrt()
    }

    fn rising(&self) -> Vec<f64> {
        let m = self.mean();
        let mut out = Vec::new();
        for (t, y) in self.ts.windows(2).zip(self.ys.windows(2)) {
            if y[0] < m && y[1] >= m && y[1] > y[0] {
                out.push(t[0] + (t[1] - t[0]) * (m - y[0]) / (y[1] - y[0]));
            }
        }
        out
    }

    pub fn freq(&self) -> Option<f64> {
        let r = self.rising();
        (r.len() >= 2).then(|| (r.len() - 1) as f64 / (r[r.len() - 1] - r[0]))
    }

    pub fn thd(&self) -> Option<f64> {
        let r = self.rising();
        if r.len() < 2 {
            return None;
        }
        let (a, b) = (r[0], r[r.len() - 1]);
        let cycles = r.len() - 1;
        let n = (cycles * 256).clamp(1024, 1 << 16);
        let at = |t: f64| {
            let k = self.ts.partition_point(|&x| x < t).clamp(1, self.ts.len() - 1);
            let span = self.ts[k] - self.ts[k - 1];
            let u = if span > 0.0 { ((t - self.ts[k - 1]) / span).clamp(0.0, 1.0) } else { 0.0 };
            self.ys[k - 1] + (self.ys[k] - self.ys[k - 1]) * u
        };
        let samples: Vec<f64> = (0..n).map(|i| at(a + (b - a) * i as f64 / n as f64)).collect();
        let amp = |h: usize| {
            let w = std::f64::consts::TAU * (h * cycles) as f64 / n as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, s) in samples.iter().enumerate() {
                re += s * (w * i as f64).cos();
                im -= s * (w * i as f64).sin();
            }
            2.0 * re.hypot(im) / n as f64
        };
        let fundamental = amp(1);
        if fundamental <= 0.0 {
            return None;
        }
        let harmonics: f64 = (2..=10).map(|h| amp(h).powi(2)).sum();
        Some(100.0 * harmonics.sqrt() / fundamental)
    }
}

pub fn spectrum(ts: &[f64], ys: &[f64], from: f64, n: usize) -> Option<(Vec<f64>, Vec<f64>)> {
    let w = Window::new(ts, ys, from, f64::INFINITY)?;
    let n = n.next_power_of_two().max(64);
    let (a, b) = (w.ts[0], w.ts[w.ts.len() - 1]);
    let m = w.mean();
    let mut re: Vec<f64> = (0..n)
        .map(|i| {
            let t = a + (b - a) * i as f64 / n as f64;
            let k = w.ts.partition_point(|&x| x < t).clamp(1, w.ts.len() - 1);
            let span = w.ts[k] - w.ts[k - 1];
            let u = if span > 0.0 { ((t - w.ts[k - 1]) / span).clamp(0.0, 1.0) } else { 0.0 };
            let y = w.ys[k - 1] + (w.ys[k] - w.ys[k - 1]) * u - m;
            let hann = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            y * hann
        })
        .collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    let df = 1.0 / (b - a);
    let freqs = (1..n / 2).map(|k| k as f64 * df).collect();
    let dbv = (1..n / 2)
        .map(|k| {
            let amp = 4.0 * re[k].hypot(im[k]) / n as f64;
            20.0 * amp.max(1e-15).log10()
        })
        .collect();
    Some((freqs, dbv))
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn sampled(f: impl Fn(f64) -> f64, t_end: f64, n: usize) -> (Vec<f64>, Vec<f64>) {
        let ts: Vec<f64> = (0..=n).map(|i| t_end * i as f64 / n as f64).collect();
        let ys = ts.iter().map(|&t| f(t)).collect();
        (ts, ys)
    }

    #[test]
    fn a_clean_sine_reads_its_amplitude_frequency_and_no_distortion() {
        let (ts, ys) = sampled(|t| 2.5 + (TAU * 1000.0 * t).sin(), 0.01, 20_000);
        let w = Window::new(&ts, &ys, 0.0, 1.0).unwrap();
        assert!((w.peak() - 3.5).abs() < 1e-3);
        assert!((w.trough() - 1.5).abs() < 1e-3);
        assert!((w.rms() - 1.0 / 2f64.sqrt()).abs() < 2e-3, "{}", w.rms());
        assert!((w.freq().unwrap() - 1000.0).abs() < 0.5, "{:?}", w.freq());
        assert!(w.thd().unwrap() < 0.05, "{:?}", w.thd());
    }

    #[test]
    fn a_third_harmonic_shows_as_its_ratio() {
        let (ts, ys) = sampled(
            |t| (TAU * 441.0 * t).sin() + 0.1 * (TAU * 3.0 * 441.0 * t).sin(),
            0.05,
            40_000,
        );
        let thd = Window::new(&ts, &ys, 0.0, 1.0).unwrap().thd().unwrap();
        assert!((thd - 10.0).abs() < 0.3, "{thd}");
    }

    #[test]
    fn clipping_a_sine_raises_its_thd() {
        let (ts, ys) = sampled(|t| (TAU * 1000.0 * t).sin().clamp(-0.6, 0.6), 0.01, 20_000);
        let thd = Window::new(&ts, &ys, 0.0, 1.0).unwrap().thd().unwrap();
        assert!(thd > 10.0, "{thd}");
    }

    #[test]
    fn the_window_cuts_the_start_up() {
        let (ts, ys) =
            sampled(|t| if t < 0.005 { 9.0 } else { (TAU * 1000.0 * t).sin() }, 0.01, 20_000);
        let all = Window::new(&ts, &ys, 0.0, 1.0).unwrap();
        let late = Window::new(&ts, &ys, 0.006, 1.0).unwrap();
        assert!((all.peak() - 9.0).abs() < 1e-9);
        assert!((late.peak() - 1.0).abs() < 1e-3);
        assert!(Window::new(&ts, &ys, 0.02, 1.0).is_none());
    }

    #[test]
    fn the_spectrum_peaks_at_the_tone_and_its_harmonic() {
        let (ts, ys) =
            sampled(|t| (TAU * 1000.0 * t).sin() + 0.01 * (TAU * 3000.0 * t).sin(), 0.02, 40_000);
        let (f, db) = spectrum(&ts, &ys, 0.0, 4096).unwrap();
        let at = |hz: f64| {
            let k = f.iter().position(|&x| x >= hz).unwrap();
            db[k.saturating_sub(1)..=k + 1].iter().copied().fold(f64::MIN, f64::max)
        };
        assert!(at(1000.0).abs() < 0.5, "{}", at(1000.0));
        assert!((at(3000.0) + 40.0).abs() < 1.0, "{}", at(3000.0));
        assert!(at(2000.0) < -80.0, "{}", at(2000.0));
    }
}

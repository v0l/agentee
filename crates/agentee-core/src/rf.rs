use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

impl Cx {
    pub const ZERO: Cx = Cx { re: 0.0, im: 0.0 };
    pub const ONE: Cx = Cx { re: 1.0, im: 0.0 };

    pub fn new(re: f64, im: f64) -> Cx {
        Cx { re, im }
    }

    pub fn polar(mag: f64, deg: f64) -> Cx {
        let a = deg.to_radians();
        Cx::new(mag * a.cos(), mag * a.sin())
    }

    pub fn norm2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    pub fn abs(self) -> f64 {
        self.norm2().sqrt()
    }

    pub fn conj(self) -> Cx {
        Cx::new(self.re, -self.im)
    }

    pub fn db(self) -> f64 {
        10.0 * self.norm2().max(1e-30).log10()
    }
}

impl Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for Cx {
    type Output = Cx;
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
}

impl Mul for Cx {
    type Output = Cx;
    fn mul(self, o: Cx) -> Cx {
        Cx::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

impl Mul<f64> for Cx {
    type Output = Cx;
    fn mul(self, k: f64) -> Cx {
        Cx::new(self.re * k, self.im * k)
    }
}

impl Div for Cx {
    type Output = Cx;
    fn div(self, o: Cx) -> Cx {
        let d = o.norm2();
        Cx::new((self.re * o.re + self.im * o.im) / d, (self.im * o.re - self.re * o.im) / d)
    }
}

impl Neg for Cx {
    type Output = Cx;
    fn neg(self) -> Cx {
        Cx::new(-self.re, -self.im)
    }
}

pub type Matrix = Vec<Vec<Cx>>;

#[derive(Clone, Debug)]
pub struct Network {
    pub ports: usize,
    pub z0: f64,
    pub freqs: Vec<f64>,
    pub s: Vec<Matrix>,
    pub noise: Vec<(f64, NoiseParams)>,
}

impl Network {
    pub fn noise_at(&self, f: f64) -> Option<NoiseParams> {
        let (first, last) = (self.noise.first()?.0, self.noise.last()?.0);
        if f < first * (1.0 - 1e-9) || f > last * (1.0 + 1e-9) {
            return None;
        }
        let k = self.noise.partition_point(|x| x.0 < f).min(self.noise.len() - 1);
        if k == 0 || self.noise[k].0 == f {
            return Some(self.noise[k].1);
        }
        let ((f0, a), (f1, b)) = (self.noise[k - 1], self.noise[k]);
        let t = (f - f0) / (f1 - f0);
        Some(NoiseParams {
            fmin: a.fmin * (1.0 - t) + b.fmin * t,
            gamma_opt: a.gamma_opt * (1.0 - t) + b.gamma_opt * t,
            rn: a.rn * (1.0 - t) + b.rn * t,
        })
    }

    pub fn at(&self, f: f64) -> Option<Matrix> {
        let (first, last) = (*self.freqs.first()?, *self.freqs.last()?);
        if f < first * (1.0 - 1e-9) || f > last * (1.0 + 1e-9) {
            return None;
        }
        let k = self.freqs.partition_point(|x| *x < f).min(self.freqs.len() - 1);
        if k == 0 || self.freqs[k] == f {
            return Some(self.s[k].clone());
        }
        let (f0, f1) = (self.freqs[k - 1], self.freqs[k]);
        let t = (f - f0) / (f1 - f0);
        let (a, b) = (&self.s[k - 1], &self.s[k]);
        Some(
            (0..self.ports)
                .map(|i| (0..self.ports).map(|j| a[i][j] * (1.0 - t) + b[i][j] * t).collect())
                .collect(),
        )
    }
}

pub fn parse_touchstone(text: &str, ports: usize) -> Result<Network, String> {
    let (mut scale, mut format, mut z0) = (1e9, "MA".to_string(), 50.0);
    let mut numbers = Vec::new();
    let mut seen_option = false;
    for line in text.lines() {
        let line = line.split('!').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(opt) = line.strip_prefix('#') {
            if seen_option {
                continue;
            }
            seen_option = true;
            let words: Vec<String> = opt.split_whitespace().map(|w| w.to_uppercase()).collect();
            let mut i = 0;
            while i < words.len() {
                match words[i].as_str() {
                    "HZ" => scale = 1.0,
                    "KHZ" => scale = 1e3,
                    "MHZ" => scale = 1e6,
                    "GHZ" => scale = 1e9,
                    "MA" | "DB" | "RI" => format = words[i].clone(),
                    "S" => {}
                    "Y" | "Z" | "H" | "G" => {
                        return Err(format!("only S parameters are read, not {}", words[i]));
                    }
                    "R" => {
                        z0 = words
                            .get(i + 1)
                            .and_then(|w| w.parse().ok())
                            .ok_or("the option line has R without a number")?;
                        i += 1;
                    }
                    w => return Err(format!("unknown option `{w}` in the # line")),
                }
                i += 1;
            }
            continue;
        }
        if line.starts_with('[') {
            return Err("Touchstone 2.0 keyword files are not read yet".into());
        }
        for w in line.split_whitespace() {
            numbers.push(w.parse::<f64>().map_err(|_| format!("cannot read `{w}` as a number"))?);
        }
    }
    let per = 1 + 2 * ports * ports;
    let mut net = Network { ports, z0, freqs: Vec::new(), s: Vec::new(), noise: Vec::new() };
    let mut pos = 0;
    while pos < numbers.len() {
        let f = numbers[pos] * scale;
        if net.freqs.last().is_some_and(|last| f <= *last) {
            break;
        }
        let rec = &numbers[pos..(pos + per).min(numbers.len())];
        pos += per;
        if rec.len() < per {
            return Err(format!("the record at {} Hz is short", f));
        }
        let pair = |k: usize| {
            let (a, b) = (rec[1 + 2 * k], rec[2 + 2 * k]);
            match format.as_str() {
                "RI" => Cx::new(a, b),
                "DB" => Cx::polar(10f64.powf(a / 20.0), b),
                _ => Cx::polar(a, b),
            }
        };
        let m: Matrix = (0..ports)
            .map(|i| {
                (0..ports)
                    .map(|j| pair(if ports == 2 { j * 2 + i } else { i * ports + j }))
                    .collect()
            })
            .collect();
        net.freqs.push(f);
        net.s.push(m);
    }
    if net.freqs.is_empty() {
        return Err("no frequency points".into());
    }
    if ports == 2 {
        for rec in numbers[pos.min(numbers.len())..].chunks(5) {
            if rec.len() < 5 {
                return Err("the noise block ends in the middle of a record".into());
            }
            net.noise.push((
                rec[0] * scale,
                NoiseParams {
                    fmin: 10f64.powf(rec[1] / 10.0),
                    gamma_opt: Cx::polar(rec[2], rec[3]),
                    rn: rec[4] * z0,
                },
            ));
        }
    }
    Ok(net)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mount {
    Series,
    Shunt,
}

pub fn impedance(s: &Matrix, z0: f64, mount: Mount) -> Cx {
    let s11 = (s[0][0] + s[1][1]) * 0.5;
    let s21 = (s[1][0] + s[0][1]) * 0.5;
    let z0c = Cx::new(z0, 0.0);
    match mount {
        Mount::Series => z0c * 2.0 * s11 / s21,
        Mount::Shunt => -(z0c * s21) / (s11 * 2.0),
    }
}

pub fn as_one_port(net: &Network, mount: Mount, z_port: f64) -> (Network, Vec<f64>) {
    let mut clamped = Vec::new();
    let s = net
        .s
        .iter()
        .zip(&net.freqs)
        .map(|(m, f)| {
            let mut z = impedance(m, net.z0, mount);
            if z.re < 0.0 {
                clamped.push(*f);
                z.re = 0.0;
            }
            let r = Cx::new(z_port, 0.0);
            vec![vec![(z - r) / (z + r)]]
        })
        .collect();
    (Network { ports: 1, z0: z_port, freqs: net.freqs.clone(), s, noise: Vec::new() }, clamped)
}

pub fn ports_from_path(path: &std::path::Path) -> Option<usize> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    ext.strip_prefix('s')?.strip_suffix('p')?.parse().ok()
}

pub fn solve(mut a: Matrix, mut b: Matrix) -> Option<Matrix> {
    let n = a.len();
    for c in 0..n {
        let p = (c..n).max_by(|x, y| a[*x][c].norm2().total_cmp(&a[*y][c].norm2()))?;
        if a[p][c].norm2() < 1e-300 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in 0..n {
            if r == c {
                continue;
            }
            let k = a[r][c] / a[c][c];
            if k == Cx::ZERO {
                continue;
            }
            let (pivot_a, pivot_b) = (a[c].clone(), b[c].clone());
            for (x, v) in a[r].iter_mut().zip(&pivot_a).skip(c) {
                *x = *x - k * *v;
            }
            for (x, v) in b[r].iter_mut().zip(&pivot_b) {
                *x = *x - k * *v;
            }
        }
    }
    for (r, row) in b.iter_mut().enumerate() {
        let d = a[r][r];
        row.iter_mut().for_each(|x| *x = *x / d);
    }
    Some(b)
}

pub struct Joined {
    pub s: Matrix,
    pub external: Vec<usize>,
    pub transfer: Matrix,
    pub internal: Vec<usize>,
    pub response: Matrix,
}

pub fn connect(s: &Matrix, pairs: &[(usize, usize)]) -> Option<(Matrix, Vec<usize>)> {
    join(s, pairs).map(|j| (j.s, j.external))
}

pub fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let (r, k, c) = (a.len(), b.len(), b.first().map(|x| x.len()).unwrap_or(0));
    (0..r)
        .map(|i| (0..c).map(|j| (0..k).fold(Cx::ZERO, |acc, t| acc + a[i][t] * b[t][j])).collect())
        .collect()
}

pub fn adjoint(a: &Matrix) -> Matrix {
    let (r, c) = (a.len(), a.first().map(|x| x.len()).unwrap_or(0));
    (0..c).map(|j| (0..r).map(|i| a[i][j].conj()).collect()).collect()
}

pub const K_B: f64 = 1.380649e-23;
pub const T0: f64 = 290.0;

pub fn passive_noise(s: &Matrix, kelvin: f64) -> Matrix {
    let ss = mul(s, &adjoint(s));
    let n = s.len();
    (0..n)
        .map(|i| {
            (0..n)
                .map(|j| (if i == j { Cx::ONE } else { Cx::ZERO } - ss[i][j]) * (K_B * kelvin))
                .collect()
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct NoiseParams {
    pub fmin: f64,
    pub gamma_opt: Cx,
    pub rn: f64,
}

pub fn active_noise(s: &Matrix, np: NoiseParams, z0: f64) -> Option<Matrix> {
    let (s11, s12, s21, s22) = (s[0][0], s[0][1], s[1][0], s[1][1]);
    let two = s21 * 2.0;
    let a = ((Cx::ONE + s11) * (Cx::ONE - s22) + s12 * s21) / two;
    let b = ((Cx::ONE + s11) * (Cx::ONE + s22) - s12 * s21) * z0 / two;
    let c = ((Cx::ONE - s11) * (Cx::ONE - s22) - s12 * s21) / (two * z0);
    let d = ((Cx::ONE - s11) * (Cx::ONE + s22) + s12 * s21) / two;
    let z = Cx::new(z0, 0.0);
    let yopt = (Cx::ONE - np.gamma_opt) / ((Cx::ONE + np.gamma_opt) * z0);
    let k4 = 4.0 * K_B * T0;
    let cvi = (Cx::new((np.fmin - 1.0) / 2.0, 0.0) - yopt.conj() * np.rn) * k4;
    let ca = vec![
        vec![Cx::new(np.rn * k4, 0.0), cvi],
        vec![cvi.conj(), Cx::new(np.rn * yopt.norm2() * k4, 0.0)],
    ];
    let mut t = vec![vec![Cx::ZERO; 2]; 2];
    for src in 0..2 {
        let m = vec![
            vec![Cx::ONE, z, Cx::ZERO, Cx::ZERO],
            vec![Cx::ONE, Cx::ZERO, -a, -b],
            vec![Cx::ZERO, Cx::ONE, -c, -d],
            vec![Cx::ZERO, Cx::ZERO, Cx::ONE, -z],
        ];
        let mut rhs = vec![vec![Cx::ZERO]; 4];
        rhs[1 + src][0] = Cx::ONE;
        let x = solve(m, rhs)?;
        let (v1, i1, v2, i2) = (x[0][0], x[1][0], x[2][0], x[3][0]);
        let scale = 1.0 / (2.0 * z0.sqrt());
        t[0][src] = (v1 - z * i1) * scale;
        t[1][src] = (v2 + z * i2) * scale;
    }
    Some(mul(&mul(&t, &ca), &adjoint(&t)))
}

pub fn noise_figure(s21: Cx, c22: f64) -> f64 {
    1.0 + c22 / (K_B * T0 * s21.norm2())
}

pub fn join(s: &Matrix, pairs: &[(usize, usize)]) -> Option<Joined> {
    let n = s.len();
    let internal: Vec<usize> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let external: Vec<usize> = (0..n).filter(|p| !internal.contains(p)).collect();
    let ni = internal.len();
    let pos = |p: usize| internal.iter().position(|x| *x == p).unwrap();
    let mut gamma = vec![vec![Cx::ZERO; ni]; ni];
    for (a, b) in pairs {
        gamma[pos(*a)][pos(*b)] = Cx::ONE;
        gamma[pos(*b)][pos(*a)] = Cx::ONE;
    }
    let sub = |rows: &[usize], cols: &[usize]| -> Matrix {
        rows.iter().map(|r| cols.iter().map(|c| s[*r][*c]).collect()).collect()
    };
    let (see, sei, sie, sii) = (
        sub(&external, &external),
        sub(&external, &internal),
        sub(&internal, &external),
        sub(&internal, &internal),
    );
    let sg = mul(&sii, &gamma);
    let lhs: Matrix = (0..ni)
        .map(|i| (0..ni).map(|j| if i == j { Cx::ONE - sg[i][j] } else { -sg[i][j] }).collect())
        .collect();
    let ne = external.len();
    let mut rhs = sie.clone();
    for (r, row) in rhs.iter_mut().enumerate() {
        row.extend((0..ni).map(|c| if c == r { Cx::ONE } else { Cx::ZERO }));
    }
    let x = solve(lhs, rhs)?;
    let bi: Matrix = x.iter().map(|r| r[..ne].to_vec()).collect();
    let inv: Matrix = x.iter().map(|r| r[ne..].to_vec()).collect();
    let sg_e = mul(&sei, &gamma);
    let tail = mul(&sg_e, &bi);
    let out = (0..ne).map(|i| (0..ne).map(|j| see[i][j] + tail[i][j]).collect()).collect();
    let via = mul(&sg_e, &inv);
    let mut transfer = vec![vec![Cx::ZERO; n]; ne];
    for (i, row) in transfer.iter_mut().enumerate() {
        row[external[i]] = Cx::ONE;
        for (k, p) in internal.iter().enumerate() {
            row[*p] = via[i][k];
        }
    }
    Some(Joined { s: out, external, transfer, internal, response: bi })
}

pub fn block_diag(parts: &[&Matrix]) -> Matrix {
    let n: usize = parts.iter().map(|m| m.len()).sum();
    let mut out = vec![vec![Cx::ZERO; n]; n];
    let mut o = 0;
    for m in parts {
        for i in 0..m.len() {
            for j in 0..m.len() {
                out[o + i][o + j] = m[i][j];
            }
        }
        o += m.len();
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Stability {
    pub k: f64,
    pub delta: f64,
    pub mu: f64,
    pub mu_prime: f64,
}

pub fn stability(s: &Matrix) -> Stability {
    let (s11, s12, s21, s22) = (s[0][0], s[0][1], s[1][0], s[1][1]);
    let delta = s11 * s22 - s12 * s21;
    let loop_gain = (s12 * s21).abs();
    let k = (1.0 - s11.norm2() - s22.norm2() + delta.norm2()) / (2.0 * loop_gain.max(1e-30));
    let mu = (1.0 - s11.norm2()) / ((s22 - delta * s11.conj()).abs() + loop_gain);
    let mu_prime = (1.0 - s22.norm2()) / ((s11 - delta * s22.conj()).abs() + loop_gain);
    Stability { k, delta: delta.abs(), mu, mu_prime }
}

pub fn max_gain_db(s: &Matrix) -> Option<f64> {
    let st = stability(s);
    if st.k < 1.0 {
        return None;
    }
    let ratio = s[1][0].abs() / s[0][1].abs().max(1e-30);
    Some(10.0 * (ratio * (st.k - (st.k * st.k - 1.0).sqrt())).log10())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Cx, b: Cx) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn touchstone_two_port_order_is_s11_s21_s12_s22() {
        let t =
            "! x\n# MHZ S DB R 50\n100 -10 0  20 90  -30 0  -15 45\n200 -11 0 19 80 -31 0 -16 40\n";
        let n = parse_touchstone(t, 2).unwrap();
        assert_eq!(n.freqs, vec![1e8, 2e8]);
        assert!((n.s[0][1][0].db() - 20.0).abs() < 1e-9);
        assert!((n.s[0][0][1].db() + 30.0).abs() < 1e-9);
        assert!(close(n.s[0][1][0], Cx::polar(10.0, 90.0)));
        let mid = n.at(1.5e8).unwrap();
        assert!(close(mid[0][0], (n.s[0][0][0] + n.s[1][0][0]) * 0.5));
        assert!(n.at(3e8).is_none());
    }

    #[test]
    fn touchstone_reads_the_noise_block() {
        let t = "# GHZ S RI R 50\n1 0 0 1 0 1 0 0 0\n2 0 0 1 0 1 0 0 0\n1 1.5 0.3 45 0.2\n2 2.5 0.4 90 0.3\n";
        let n = parse_touchstone(t, 2).unwrap();
        assert_eq!(n.freqs.len(), 2);
        assert_eq!(n.noise.len(), 2);
        let mid = n.noise_at(1.5e9).unwrap();
        assert!((10.0 * mid.fmin.log10() - 2.0).abs() < 0.05);
        assert!((mid.rn - 12.5).abs() < 1e-9);
    }

    #[test]
    fn series_and_shunt_fixtures_give_back_the_part() {
        let z0 = 50.0;
        for z in
            [Cx::new(0.05, 0.3), Cx::new(2.0, -40.0), Cx::new(30.0, 900.0), Cx::new(5000.0, 2.0)]
        {
            let r = Cx::new(z0, 0.0);
            let series_s11 = z / (z + r * 2.0);
            let series_s21 = r * 2.0 / (z + r * 2.0);
            let series = vec![vec![series_s11, series_s21], vec![series_s21, series_s11]];
            let shunt_s11 = -r / (z * 2.0 + r);
            let shunt_s21 = z * 2.0 / (z * 2.0 + r);
            let shunt = vec![vec![shunt_s11, shunt_s21], vec![shunt_s21, shunt_s11]];
            for (m, mount) in [(series, Mount::Series), (shunt, Mount::Shunt)] {
                let back = impedance(&m, z0, mount);
                assert!((back - z).abs() / z.abs() < 1e-9, "{mount:?} {z:?} {back:?}");
            }
        }
    }

    #[test]
    fn fixture_line_length_on_both_sides_cancels() {
        let z = Cx::new(40.0, 700.0);
        let r = Cx::new(50.0, 0.0);
        let line = Cx::polar(1.0, -2.0 * 37.0);
        let s11 = -r / (z * 2.0 + r) * line;
        let s21 = z * 2.0 / (z * 2.0 + r) * line;
        let m = vec![vec![s11, s21], vec![s21, s11]];
        assert!((impedance(&m, 50.0, Mount::Shunt) - z).abs() < 1e-9);
        let zs = Cx::new(0.3, 5.0);
        let s11 = zs / (zs + r * 2.0) * line;
        let s21 = r * 2.0 / (zs + r * 2.0) * line;
        let m = vec![vec![s11, s21], vec![s21, s11]];
        assert!((impedance(&m, 50.0, Mount::Series) - zs).abs() < 1e-9);
    }

    #[test]
    fn thru_lines_hand_the_device_through_unchanged() {
        let thru = vec![vec![Cx::ZERO, Cx::ONE], vec![Cx::ONE, Cx::ZERO]];
        let dev = vec![
            vec![Cx::polar(0.3, -40.0), Cx::polar(0.05, 20.0)],
            vec![Cx::polar(8.0, 120.0), Cx::polar(0.4, -70.0)],
        ];
        let all = block_diag(&[&thru, &dev, &thru]);
        let (s, ext) = connect(&all, &[(1, 2), (3, 4)]).unwrap();
        assert_eq!(ext, vec![0, 5]);
        for i in 0..2 {
            for j in 0..2 {
                assert!(close(s[i][j], dev[i][j]), "{i}{j}");
            }
        }
    }

    fn noise_out(s: &Matrix, cs: &Matrix, pairs: &[(usize, usize)]) -> f64 {
        let j = join(s, pairs).unwrap();
        let c = mul(&mul(&j.transfer, cs), &adjoint(&j.transfer));
        noise_figure(j.s[1][0], c[1][1].re)
    }

    #[test]
    fn a_matched_attenuator_at_t0_has_its_loss_as_noise_figure() {
        let a = 10f64.powf(-6.0 / 20.0);
        let pad = vec![vec![Cx::ZERO, Cx::new(a, 0.0)], vec![Cx::new(a, 0.0), Cx::ZERO]];
        let cs = passive_noise(&pad, T0);
        let f = noise_figure(pad[1][0], cs[1][1].re);
        assert!((10.0 * f.log10() - 6.0).abs() < 1e-9);
        let both = block_diag(&[&pad, &pad]);
        let f2 = noise_out(&both, &passive_noise(&both, T0), &[(1, 2)]);
        assert!((10.0 * f2.log10() - 12.0).abs() < 1e-9);
    }

    #[test]
    fn a_device_behind_a_lossless_mismatch_follows_the_noise_parameter_formula() {
        let dev = vec![
            vec![Cx::polar(0.35, -60.0), Cx::polar(0.04, 30.0)],
            vec![Cx::polar(6.0, 110.0), Cx::polar(0.3, -40.0)],
        ];
        let np = NoiseParams { fmin: 10f64.powf(0.06), gamma_opt: Cx::polar(0.45, 70.0), rn: 12.0 };
        let cd = active_noise(&dev, np, 50.0).unwrap();
        let f50 = noise_figure(dev[1][0], cd[1][1].re);
        let want50 =
            np.fmin + 4.0 * np.rn / 50.0 * np.gamma_opt.norm2() / (Cx::ONE + np.gamma_opt).norm2();
        assert!((f50 - want50).abs() < 1e-9, "{f50} {want50}");
        for (b, deg) in [(0.4, 30.0), (1.2, -80.0), (0.05, 170.0)] {
            let y = Cx::new(0.0, b);
            let den = y + Cx::new(2.0, 0.0);
            let (s11, s21) = (-y / den, Cx::new(2.0, 0.0) / den);
            let shunt = vec![vec![s11, s21], vec![s21, s11]];
            let line =
                vec![vec![Cx::ZERO, Cx::polar(1.0, deg)], vec![Cx::polar(1.0, deg), Cx::ZERO]];
            let (m, _) = connect(&block_diag(&[&shunt, &line]), &[(1, 2)]).unwrap();
            let all = block_diag(&[&m, &dev]);
            let mut cs = vec![vec![Cx::ZERO; 4]; 4];
            for i in 0..2 {
                for j in 0..2 {
                    cs[2 + i][2 + j] = cd[i][j];
                }
            }
            let f = noise_out(&all, &cs, &[(1, 2)]);
            let gs = m[1][1];
            let want = np.fmin
                + 4.0 * np.rn / 50.0 * (gs - np.gamma_opt).norm2()
                    / ((1.0 - gs.norm2()) * (Cx::ONE + np.gamma_opt).norm2());
            assert!((f - want).abs() < 1e-9, "{f} {want} at {gs:?}");
        }
    }

    #[test]
    fn friis_holds_for_an_amplifier_then_a_pad() {
        let dev = vec![vec![Cx::ZERO, Cx::ZERO], vec![Cx::new(10.0, 0.0), Cx::ZERO]];
        let np = NoiseParams { fmin: 2.0, gamma_opt: Cx::ZERO, rn: 5.0 };
        let a = 0.5f64.sqrt();
        let pad = vec![vec![Cx::ZERO, Cx::new(a, 0.0)], vec![Cx::new(a, 0.0), Cx::ZERO]];
        let all = block_diag(&[&dev, &pad]);
        let mut cs = passive_noise(&all, T0);
        let cd = active_noise(&dev, np, 50.0).unwrap();
        for i in 0..2 {
            for j in 0..2 {
                cs[i][j] = cd[i][j];
            }
        }
        let f = noise_out(&all, &cs, &[(1, 2)]);
        let want = 2.0 + (2.0 - 1.0) / 100.0;
        assert!((f - want).abs() < 1e-9, "{f} {want}");
    }

    #[test]
    fn two_matched_three_db_pads_make_six() {
        let a = 10f64.powf(-3.0 / 20.0);
        let pad = vec![vec![Cx::ZERO, Cx::new(a, 0.0)], vec![Cx::new(a, 0.0), Cx::ZERO]];
        let (s, _) = connect(&block_diag(&[&pad, &pad]), &[(1, 2)]).unwrap();
        assert!((s[1][0].db() + 6.0).abs() < 1e-9);
        assert!(s[0][0].abs() < 1e-12);
    }

    #[test]
    fn a_mismatch_between_two_lines_matches_the_series_formula() {
        let g = Cx::new(0.2, 0.1);
        let t = Cx::polar(0.9, -30.0);
        let step = vec![vec![g, t], vec![t, -g]];
        let (s, _) = connect(&block_diag(&[&step, &step]), &[(1, 2)]).unwrap();
        let s21 = t * t / (Cx::ONE - (-g) * g);
        let s11 = g + t * g * t / (Cx::ONE - (-g) * g);
        assert!(close(s[1][0], s21));
        assert!(close(s[0][0], s11));
    }

    #[test]
    fn mu_above_one_exactly_when_k_above_one_and_delta_below_one() {
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut both = [0usize; 2];
        for _ in 0..20000 {
            let mut c = |m: f64| Cx::polar(m * rnd(), 360.0 * rnd());
            let s = vec![vec![c(1.0), c(0.3)], vec![c(10.0), c(1.0)]];
            let st = stability(&s);
            let rollett = st.k > 1.0 && st.delta < 1.0;
            if (st.k - 1.0).abs() < 1e-9 || (st.mu - 1.0).abs() < 1e-9 {
                continue;
            }
            assert_eq!(rollett, st.mu > 1.0, "{st:?}");
            assert_eq!(st.mu > 1.0, st.mu_prime > 1.0, "{st:?}");
            both[rollett as usize] += 1;
        }
        assert!(both[0] > 100 && both[1] > 100, "{both:?}");
    }
}

use agentee_core::rf::Cx;
use faer::Mat;
use faer::linalg::solvers::SolveLstsq;

#[derive(Clone, Debug, Default)]
pub struct Rational {
    pub d: f64,
    pub e: f64,
    pub real: Vec<f64>,
    pub pairs: Vec<Cx>,
}

#[derive(Clone, Debug, Default)]
pub struct Surface {
    pub scale: f64,
    pub real: Vec<f64>,
    pub pairs: Vec<Cx>,
    pub faces: Vec<Rational>,
}

fn basis(real: &[f64], pairs: &[Cx], s: Cx) -> Vec<Cx> {
    let i = Cx::new(0.0, 1.0);
    let mut out = Vec::with_capacity(real.len() + 2 * pairs.len());
    for p in real {
        out.push(Cx::ONE / (s - Cx::new(*p, 0.0)));
    }
    for q in pairs {
        let (a, b) = (Cx::ONE / (s - *q), Cx::ONE / (s - q.conj()));
        out.push(a + b);
        out.push(i * (a - b));
    }
    out
}

fn lstsq(a: &Mat<f64>, b: &Mat<f64>) -> Vec<f64> {
    let norms: Vec<f64> = (0..a.ncols())
        .map(|j| (0..a.nrows()).map(|i| a[(i, j)] * a[(i, j)]).sum::<f64>().sqrt().max(1e-300))
        .collect();
    let scaled = Mat::<f64>::from_fn(a.nrows(), a.ncols(), |i, j| a[(i, j)] / norms[j]);
    faer::set_global_parallelism(faer::Par::Seq);
    let x = scaled.col_piv_qr().solve_lstsq(b);
    (0..a.ncols()).map(|j| x[(j, 0)] / norms[j]).collect()
}

impl Surface {
    pub fn sheet_table(&self, dt: f64) -> Vec<f32> {
        let (nr, nc) = (self.real.len(), self.pairs.len());
        let mut out = vec![nr as f32, nc as f32, self.faces.len() as f32, 0.0];
        let step = |p: Cx| {
            let q = p * (0.5 * dt);
            let one = Cx::ONE;
            let den = one - q;
            ((one + q) / den, -(q * 2.0) / den, one / den)
        };
        let real: Vec<_> = self.real.iter().map(|p| step(Cx::new(p * self.scale, 0.0))).collect();
        let pairs: Vec<_> = self.pairs.iter().map(|p| step(*p * self.scale)).collect();
        for (alpha, g, _) in &real {
            out.extend_from_slice(&[alpha.re as f32, g.re as f32]);
        }
        for (alpha, g, _) in &pairs {
            out.extend_from_slice(&[alpha.re as f32, alpha.im as f32, g.re as f32, g.im as f32]);
        }
        for f in &self.faces {
            let lt = 2.0 * f.e / (self.scale * dt);
            let mut a = lt + f.d;
            let mut ch = Vec::with_capacity(nr + 2 * nc);
            for ((p, r), (_, g, h)) in self.real.iter().zip(&f.real).zip(&real) {
                let c = -r / p;
                a += c * 0.5 * g.re;
                ch.push((c * h.re) as f32);
            }
            for ((p, r), (_, g, h)) in self.pairs.iter().zip(&f.pairs).zip(&pairs) {
                let c = -(*r / *p);
                a += (c * *g).re;
                let v = c * *h * 2.0;
                ch.extend_from_slice(&[v.re as f32, v.im as f32]);
            }
            out.extend_from_slice(&[a as f32, lt as f32]);
            out.extend(ch);
        }
        out
    }

    pub fn impedance(&self, face: usize, omega: f64) -> Cx {
        self.at(face, Cx::new(0.0, omega / self.scale))
    }

    fn at(&self, face: usize, s: Cx) -> Cx {
        let f = &self.faces[face];
        let mut z = Cx::new(f.d, 0.0) + s * f.e;
        for (p, r) in self.real.iter().zip(&f.real) {
            z = z + Cx::new(*r, 0.0) / (s - Cx::new(*p, 0.0));
        }
        for (q, c) in self.pairs.iter().zip(&f.pairs) {
            z = z + *c / (s - *q) + c.conj() / (s - q.conj());
        }
        z
    }

    pub fn fit(targets: &[Vec<Cx>], omega: &[f64], scale: f64, order: usize) -> Surface {
        let s: Vec<Cx> = omega.iter().map(|w| Cx::new(0.0, w / scale)).collect();
        let (lo, hi) = (omega[0] / scale, omega[omega.len() - 1] / scale);
        let npairs = (order / 2).max(1);
        let mut real: Vec<f64> = Vec::new();
        let mut pairs: Vec<Cx> = (0..npairs)
            .map(|k| {
                let b = lo * (hi / lo).powf(k as f64 / (npairs - 1).max(1) as f64);
                Cx::new(-b / 100.0, b)
            })
            .collect();
        let weights: Vec<Vec<f64>> =
            targets.iter().map(|t| t.iter().map(|v| 1.0 / v.abs().max(1e-300)).collect()).collect();
        let m = targets.len();
        let put = |a: &mut Mat<f64>, r: usize, c: usize, v: Cx| {
            a[(r, c)] = v.re;
            a[(r + 1, c)] = v.im;
        };
        for _ in 0..12 {
            let n = real.len() + 2 * pairs.len();
            let cols = m * (n + 2) + n;
            let mut a = Mat::<f64>::zeros(2 * s.len() * m, cols);
            let mut b = Mat::<f64>::zeros(2 * s.len() * m, 1);
            for (fi, t) in targets.iter().enumerate() {
                for (i, si) in s.iter().enumerate() {
                    let w = weights[fi][i];
                    let r = 2 * (fi * s.len() + i);
                    let off = fi * (n + 2);
                    for (k, v) in basis(&real, &pairs, *si).into_iter().enumerate() {
                        put(&mut a, r, off + k, v * w);
                        put(&mut a, r, m * (n + 2) + k, -(t[i] * v * w));
                    }
                    put(&mut a, r, off + n, Cx::new(w, 0.0));
                    put(&mut a, r, off + n + 1, *si * w);
                    b[(r, 0)] = t[i].re * w;
                    b[(r + 1, 0)] = t[i].im * w;
                }
            }
            let x = lstsq(&a, &b);
            let sigma = &x[m * (n + 2)..];
            let mut h = Mat::<f64>::zeros(n, n);
            for (k, p) in real.iter().enumerate() {
                h[(k, k)] = *p;
                for j in 0..n {
                    h[(k, j)] -= sigma[j];
                }
            }
            for (k, q) in pairs.iter().enumerate() {
                let i0 = real.len() + 2 * k;
                h[(i0, i0)] += q.re;
                h[(i0, i0 + 1)] += q.im;
                h[(i0 + 1, i0)] -= q.im;
                h[(i0 + 1, i0 + 1)] += q.re;
                for j in 0..n {
                    h[(i0, j)] -= 2.0 * sigma[j];
                }
            }
            let Ok(ev) = h.eigenvalues() else { break };
            real.clear();
            pairs.clear();
            for e in ev {
                let re = -e.re.abs();
                if e.im.abs() <= 1e-9 * e.re.abs().max(e.im.abs()) {
                    real.push(re);
                } else if e.im > 0.0 {
                    pairs.push(Cx::new(re, e.im));
                }
            }
            real.sort_by(|a, b| b.total_cmp(a));
            pairs.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
        }
        let faces = targets
            .iter()
            .zip(&weights)
            .map(|(t, w)| Self::residues(&real, &pairs, &s, t, w, lo, hi))
            .collect();
        Surface { scale, real, pairs, faces }
    }

    fn residues(
        real: &[f64],
        pairs: &[Cx],
        s: &[Cx],
        t: &[Cx],
        w: &[f64],
        lo: f64,
        hi: f64,
    ) -> Rational {
        let n = real.len() + 2 * pairs.len();
        let solve = |with_e: bool| {
            let cols = n + 1 + with_e as usize;
            let mut a = Mat::<f64>::zeros(2 * s.len(), cols);
            let mut b = Mat::<f64>::zeros(2 * s.len(), 1);
            for (i, si) in s.iter().enumerate() {
                let r = 2 * i;
                let mut row = basis(real, pairs, *si);
                row.push(Cx::ONE);
                if with_e {
                    row.push(*si);
                }
                for (k, v) in row.into_iter().enumerate() {
                    a[(r, k)] = v.re * w[i];
                    a[(r + 1, k)] = v.im * w[i];
                }
                b[(r, 0)] = t[i].re * w[i];
                b[(r + 1, 0)] = t[i].im * w[i];
            }
            lstsq(&a, &b)
        };
        let mut x = solve(true);
        if x[n + 1] < 0.0 {
            x = solve(false);
            x.push(0.0);
        }
        let mut f = Rational {
            d: x[n],
            e: x[n + 1],
            real: x[..real.len()].to_vec(),
            pairs: (0..pairs.len())
                .map(|k| Cx::new(x[real.len() + 2 * k], x[real.len() + 2 * k + 1]))
                .collect(),
        };
        let probe = Surface {
            scale: 1.0,
            real: real.to_vec(),
            pairs: pairs.to_vec(),
            faces: vec![f.clone()],
        };
        let span = (hi / lo).ln();
        let lowest = (0..=4000)
            .map(|k| lo * 1e-2 * (k as f64 / 4000.0 * (span + 4.0 * std::f64::consts::LN_10)).exp())
            .chain([0.0])
            .map(|v| probe.at(0, Cx::new(0.0, v)).re)
            .fold(f.d, f64::min);
        if lowest < 0.0 {
            f.d -= lowest;
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loss::{Enig, Face, Roughness, surface_impedance};

    #[test]
    fn vector_fitting_follows_the_skin_roughness_and_nickel_impedances() {
        let (f0, f_max) = (20e9, 40e9);
        let w0 = 2.0 * std::f64::consts::PI * f0;
        let (lo, hi) = (w0 / 1000.0, 2.0 * std::f64::consts::PI * f_max * 100.0);
        let omega: Vec<f64> = (0..240).map(|k| lo * (hi / lo).powf(k as f64 / 239.0)).collect();
        let t = 35e-6;
        let faces = [
            Face::Copper(Roughness::default()),
            Face::Copper(Roughness { rms_um: Some(2.8), ..Default::default() }),
            Face::Enig(Enig::default()),
        ];
        let targets: Vec<Vec<Cx>> = faces
            .iter()
            .map(|f| omega.iter().map(|w| surface_impedance(*f, t, Cx::new(0.0, *w))).collect())
            .collect();
        let fit = Surface::fit(&targets, &omega, w0, 24);
        for (k, target) in targets.iter().enumerate() {
            let mut worst = 0.0f64;
            for w in (0..960).map(|j| lo * (hi / lo).powf(j as f64 / 959.0)) {
                let want = surface_impedance(faces[k], t, Cx::new(0.0, w));
                worst = worst.max((fit.impedance(k, w) - want).abs() / want.abs());
            }
            eprintln!("face {k}: worst error {worst:.2e}, {} samples", target.len());
            assert!(worst < 0.01, "face {k}: {worst}");
        }
        assert!(fit.real.iter().all(|p| *p < 0.0) && fit.pairs.iter().all(|q| q.re < 0.0));
    }

    #[test]
    fn the_sheet_table_steps_the_bilinear_image_of_the_fit() {
        let w0 = 2.0 * std::f64::consts::PI * 1e9;
        let (lo, hi) = (w0 / 20.0, w0 * 50.0);
        let omega: Vec<f64> = (0..120).map(|k| lo * (hi / lo).powf(k as f64 / 119.0)).collect();
        let face = Face::Enig(Enig::default());
        let target: Vec<Cx> =
            omega.iter().map(|w| surface_impedance(face, 35e-6, Cx::new(0.0, *w)) * 20.0).collect();
        let fit = Surface::fit(&[target], &omega, w0, 12);
        let dt = 0.02 / w0;
        let table = fit.sheet_table(dt);
        let (nr, nc) = (fit.real.len(), fit.pairs.len());
        let f = 4 + 2 * nr + 4 * nc;
        let t = |k: usize| table[k] as f64;
        for w in [w0, 20.0 * w0] {
            let mut states = vec![0.0f64; nr + 2 * nc];
            let steps = 400_000;
            let period = 2.0 * std::f64::consts::PI / (w * dt);
            let tail = ((steps as f64 * 0.25 / period).floor() * period).round() as usize;
            let (mut v_acc, mut i_acc) = (Cx::ZERO, Cx::ZERO);
            for n in 0..steps {
                let (i0, i1) = ((w * n as f64 * dt).cos(), (w * (n + 1) as f64 * dt).cos());
                let ibar = 0.5 * (i0 + i1);
                let mut hist = t(f + 1) * i0;
                for (k, u) in states[..nr].iter().enumerate() {
                    hist -= t(f + 2 + k) * u;
                }
                for k in 0..nc {
                    let o = f + 2 + nr + 2 * k;
                    let u = nr + 2 * k;
                    hist -= t(o) * states[u] - t(o + 1) * states[u + 1];
                }
                let v = t(f) * ibar - hist;
                for (k, u) in states[..nr].iter_mut().enumerate() {
                    *u = t(4 + 2 * k) * *u + t(5 + 2 * k) * ibar;
                }
                for k in 0..nc {
                    let p = 4 + 2 * nr + 4 * k;
                    let u = nr + 2 * k;
                    let (ur, ui) = (states[u], states[u + 1]);
                    states[u] = t(p) * ur - t(p + 1) * ui + t(p + 2) * ibar;
                    states[u + 1] = t(p) * ui + t(p + 1) * ur + t(p + 3) * ibar;
                }
                if n >= steps - tail {
                    let ph = -w * (n as f64 + 0.5) * dt;
                    let e = Cx::new(ph.cos(), ph.sin());
                    v_acc = v_acc + e * v;
                    i_acc = i_acc + e * ibar;
                }
            }
            let got = v_acc / i_acc;
            let warped = 2.0 / dt * (0.5 * w * dt).tan();
            let want = fit.impedance(0, warped);
            let err = (got - want).abs() / want.abs();
            eprintln!("{:.1} GHz: stepped {got:?}, bilinear fit {want:?}, error {err:.1e}", w / w0);
            assert!(err < 2e-3, "{err}");
        }
    }
}

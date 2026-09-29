use crate::gpu::{Gpu, gpu};
use bytemuck::{Pod, Zeroable};

pub struct Problem {
    pub n: [usize; 3],
    pub gx: Vec<f32>,
    pub gy: Vec<f32>,
    pub gz: Vec<f32>,
    pub g0: Vec<f32>,
    pub reference: f32,
    pub source: Vec<f32>,
    pub fixed: Vec<Option<f32>>,
}

pub struct Solution {
    pub phi: Vec<f32>,
    pub iterations: usize,
    pub residual: f64,
    pub device: String,
}

impl Problem {
    pub fn new(n: [usize; 3]) -> Problem {
        let len = n[0] * n[1] * n[2];
        Problem {
            n,
            gx: vec![0.0; len],
            gy: vec![0.0; len],
            gz: vec![0.0; len],
            g0: vec![0.0; len],
            reference: 0.0,
            source: vec![0.0; len],
            fixed: vec![None; len],
        }
    }

    pub fn len(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        (i * self.n[1] + j) * self.n[2] + k
    }

    fn neighbours(&self, id: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let [nx, ny, nz] = self.n;
        let (sx, sy) = (ny * nz, nz);
        let (i, j, k) = (id / sx, (id / nz) % ny, id % nz);
        [
            (i + 1 < nx).then(|| (id + sx, self.gx[id])),
            (i > 0).then(|| (id - sx, self.gx[id.wrapping_sub(sx)])),
            (j + 1 < ny).then(|| (id + sy, self.gy[id])),
            (j > 0).then(|| (id - sy, self.gy[id.wrapping_sub(sy)])),
            (k + 1 < nz).then(|| (id + 1, self.gz[id])),
            (k > 0).then(|| (id - 1, self.gz[id.wrapping_sub(1)])),
        ]
        .into_iter()
        .flatten()
    }

    fn system(&self) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
        let len = self.len();
        let mut diag = vec![0f32; len];
        let mut free = vec![0u32; len];
        let mut b = vec![0f32; len];
        for id in 0..len {
            if self.fixed[id].is_some() {
                continue;
            }
            let mut d = self.g0[id] as f64;
            let mut rhs = self.source[id] as f64;
            for (nb, g) in self.neighbours(id) {
                d += g as f64;
                if let Some(v) = self.fixed[nb] {
                    rhs += g as f64 * (v - self.reference) as f64;
                }
            }
            if d > 0.0 {
                diag[id] = d as f32;
                free[id] = 1;
                b[id] = rhs as f32;
            }
        }
        (diag, free, b)
    }

    fn finish(&self, x: &[f32]) -> Vec<f32> {
        (0..self.len()).map(|id| self.fixed[id].unwrap_or(x[id] + self.reference)).collect()
    }

    pub fn solve(&self, tol: f64, max_iter: usize) -> Solution {
        match gpu() {
            Some(g) => self.solve_gpu(g, tol, max_iter),
            None => self.solve_cpu(tol, max_iter),
        }
    }

    pub fn solve_cpu(&self, tol: f64, max_iter: usize) -> Solution {
        let (diag, free, b) = self.system();
        let len = self.len();
        let apply = |v: &[f64]| -> Vec<f64> {
            (0..len)
                .map(|id| {
                    if free[id] == 0 {
                        return v[id];
                    }
                    let mut s = diag[id] as f64 * v[id];
                    for (nb, g) in self.neighbours(id) {
                        if free[nb] == 1 {
                            s -= g as f64 * v[nb];
                        }
                    }
                    s
                })
                .collect()
        };
        let pre = |r: &[f64]| -> Vec<f64> {
            (0..len).map(|i| if diag[i] > 0.0 { r[i] / diag[i] as f64 } else { 0.0 }).collect()
        };
        let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
        let mut x = vec![0f64; len];
        let mut r: Vec<f64> = b.iter().map(|v| *v as f64).collect();
        let rr0 = dot(&r, &r).max(1e-300);
        let mut z = pre(&r);
        let mut p = z.clone();
        let mut rz = dot(&r, &z);
        let mut it = 0;
        let mut rr = rr0;
        while it < max_iter && (rr / rr0).sqrt() > tol {
            let ap = apply(&p);
            let alpha = rz / dot(&p, &ap).max(1e-300);
            for i in 0..len {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
            }
            z = pre(&r);
            let rz_new = dot(&r, &z);
            let beta = rz_new / rz.max(1e-300);
            rz = rz_new;
            for i in 0..len {
                p[i] = z[i] + beta * p[i];
            }
            rr = dot(&r, &r);
            it += 1;
        }
        let xs: Vec<f32> = x.iter().map(|v| *v as f32).collect();
        Solution {
            phi: self.finish(&xs),
            iterations: it,
            residual: (rr / rr0).sqrt(),
            device: "cpu".into(),
        }
    }

    pub fn solve_gpu(&self, g: &Gpu, tol: f64, max_iter: usize) -> Solution {
        let (diag, free, b) = self.system();
        let n = self.len();
        let groups = n.div_ceil(256).clamp(1, 1024) as u32;
        let z: Vec<f32> =
            (0..n).map(|i| if diag[i] > 0.0 { b[i] / diag[i] } else { 0.0 }).collect();
        let rz0: f64 = b.iter().zip(&z).map(|(r, z)| *r as f64 * *z as f64).sum();
        let rr0: f64 = b.iter().map(|r| (*r as f64).powi(2)).sum::<f64>().max(1e-300);
        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct Params {
            n: [u32; 8],
        }
        let params = Params {
            n: [self.n[0] as u32, self.n[1] as u32, self.n[2] as u32, n as u32, groups, 0, 0, 0],
        };
        let b_params = g.uniform("nodal", &params);
        let b_gx = g.storage("gx", &self.gx);
        let b_gy = g.storage("gy", &self.gy);
        let b_gz = g.storage("gz", &self.gz);
        let b_diag = g.storage("diag", &diag);
        let b_x = g.zeroed("x", (n * 4) as u64);
        let b_r = g.storage("r", &b);
        let b_p = g.storage("p", &z);
        let b_ap = g.zeroed("ap", (n * 4) as u64);
        let b_partial = g.zeroed("partial", (2 * groups as usize * 4) as u64);
        let b_scalars =
            g.storage("scalars", &[rz0 as f32, 0.0, 0.0, 0.0, rr0 as f32, 0.0, 0.0, 0.0]);
        let b_free = g.storage("free", &free);
        let module = g.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nodal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("nodal.wgsl").into()),
        });
        let all: [(u32, &wgpu::Buffer); 12] = [
            (0, &b_params),
            (1, &b_gx),
            (2, &b_gy),
            (3, &b_gz),
            (4, &b_diag),
            (5, &b_x),
            (6, &b_r),
            (7, &b_p),
            (8, &b_ap),
            (9, &b_partial),
            (10, &b_scalars),
            (11, &b_free),
        ];
        let make = |entry: &str, uses: &[u32]| {
            let pipe = g.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            let entries: Vec<wgpu::BindGroupEntry> = all
                .iter()
                .filter(|(b, _)| uses.contains(b))
                .map(|(b, buf)| wgpu::BindGroupEntry {
                    binding: *b,
                    resource: buf.as_entire_binding(),
                })
                .collect();
            let bg = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(entry),
                layout: &pipe.get_bind_group_layout(0),
                entries: &entries,
            });
            (pipe, bg)
        };
        let wg = n.div_ceil(256) as u32;
        let flat = [wg.min(65535), wg.div_ceil(65535), 1];
        let steps = [
            (make("apply", &[0, 1, 2, 3, 4, 7, 8, 11]), flat),
            (make("dot_pap", &[0, 7, 8, 9]), [groups, 1, 1]),
            (make("reduce_alpha", &[0, 9, 10]), [1, 1, 1]),
            (make("update_xr", &[0, 4, 5, 6, 7, 8, 9, 10]), [groups, 1, 1]),
            (make("reduce_beta", &[0, 9, 10]), [1, 1, 1]),
            (make("update_p", &[0, 4, 6, 7, 10]), flat),
        ];
        let batch = 50;
        let mut it = 0;
        let mut residual = 1.0;
        while it < max_iter {
            let mut enc = g.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                for _ in 0..batch {
                    for ((pipe, bg), d) in &steps {
                        pass.set_pipeline(pipe);
                        pass.set_bind_group(0, bg, &[]);
                        pass.dispatch_workgroups(d[0], d[1], d[2]);
                    }
                }
            }
            g.queue.submit([enc.finish()]);
            it += batch;
            let s: Vec<f32> = g.read(&b_scalars, 8);
            residual = (s[4] as f64 / rr0).max(0.0).sqrt();
            if !residual.is_finite() || residual < tol {
                break;
            }
        }
        let x: Vec<f32> = g.read(&b_x, n);
        Solution { phi: self.finish(&x), iterations: it, residual, device: g.name.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(n: usize) -> Problem {
        let mut p = Problem::new([n, 3, 2]);
        for i in 0..n - 1 {
            for j in 0..3 {
                for k in 0..2 {
                    let id = p.idx(i, j, k);
                    p.gx[id] = 2.0;
                }
            }
        }
        for j in 0..3 {
            for k in 0..2 {
                let (a, b) = (p.idx(0, j, k), p.idx(n - 1, j, k));
                p.fixed[a] = Some(1.0);
                p.fixed[b] = Some(0.0);
            }
        }
        p
    }

    #[test]
    fn a_resistive_bar_divides_linearly() {
        let p = bar(21);
        for s in [p.solve_cpu(1e-8, 10_000), p.solve(1e-6, 10_000)] {
            for i in 0..21 {
                let v = s.phi[p.idx(i, 1, 1)];
                assert!((v - (1.0 - i as f32 / 20.0)).abs() < 1e-4, "{} {i} {v}", s.device);
            }
        }
    }

    #[test]
    fn a_heated_slab_matches_its_conduction_balance() {
        let mut p = Problem::new([30, 30, 4]);
        let (g, h) = (1.0f32, 0.01f32);
        for i in 0..30 {
            for j in 0..30 {
                for k in 0..4 {
                    let id = p.idx(i, j, k);
                    p.gx[id] = if i + 1 < 30 { g } else { 0.0 };
                    p.gy[id] = if j + 1 < 30 { g } else { 0.0 };
                    p.gz[id] = if k + 1 < 4 { g } else { 0.0 };
                    if k == 3 {
                        p.g0[id] = h;
                    }
                }
            }
        }
        let c = p.idx(15, 15, 0);
        p.source[c] = 1.0;
        p.reference = 25.0;
        let a = p.solve_cpu(1e-9, 20_000);
        let b = p.solve(1e-6, 20_000);
        let out: f64 = (0..30)
            .flat_map(|i| (0..30).map(move |j| (i, j)))
            .map(|(i, j)| (a.phi[p.idx(i, j, 3)] as f64 - 25.0) * h as f64)
            .sum();
        assert!((out - 1.0).abs() < 1e-3, "heat out {out}");
        let peak_a = a.phi[c];
        let peak_b = b.phi[c];
        assert!((peak_a - peak_b).abs() / (peak_a - 25.0) < 2e-3, "{peak_a} {peak_b} {}", b.device);
    }
}

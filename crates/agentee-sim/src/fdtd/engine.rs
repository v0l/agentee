pub const C0: f64 = 299_792_458.0;
pub const EPS0: f64 = 8.854_187_812_8e-12;
pub const MU0: f64 = 1.256_637_062_12e-6;
pub const ETA0: f64 = 376.730_313_668;

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub pml: usize,
}

impl Grid {
    pub fn axis(&self, a: usize) -> &[f64] {
        match a {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }

    pub fn dims(&self) -> [usize; 3] {
        [self.x.len(), self.y.len(), self.z.len()]
    }

    pub fn cells(&self) -> usize {
        (self.x.len() - 1) * (self.y.len() - 1) * (self.z.len() - 1)
    }

    pub fn nearest(&self, a: usize, v: f64) -> usize {
        let ax = self.axis(a);
        let i = ax.partition_point(|x| *x < v);
        match (i.checked_sub(1), ax.get(i)) {
            (Some(j), Some(x)) if (v - ax[j]).abs() <= (x - v).abs() => j,
            (Some(j), None) => j,
            _ => i.min(ax.len() - 1),
        }
    }

    pub fn cell_of(&self, a: usize, v: f64) -> usize {
        let ax = self.axis(a);
        ax.partition_point(|x| *x <= v).saturating_sub(1).min(ax.len() - 2)
    }
}

pub fn idx(n: [usize; 3], i: usize, j: usize, k: usize) -> usize {
    (i * n[1] + j) * n[2] + k
}

#[derive(Clone)]
pub struct Axis {
    pub n: usize,
    pub d: Vec<f64>,
    pub dd: Vec<f64>,
    pub inv_d: Vec<f32>,
    pub inv_dd: Vec<f32>,
    pub raw_inv_d: Vec<f32>,
    pub raw_inv_dd: Vec<f32>,
    pub be: Vec<f32>,
    pub ce: Vec<f32>,
    pub bh: Vec<f32>,
    pub ch: Vec<f32>,
    pub slot_e: Vec<i32>,
    pub slot_h: Vec<i32>,
}

fn axis(line: &[f64], pml: usize, dt: f64) -> Axis {
    let n = line.len();
    let d: Vec<f64> = (0..n)
        .map(|i| if i + 1 < n { line[i + 1] - line[i] } else { line[i] - line[i - 1] })
        .collect();
    let dd: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                d[0]
            } else if i + 1 >= n {
                d[n - 2]
            } else {
                0.5 * (line[i + 1] - line[i - 1])
            }
        })
        .collect();
    let (m, kappa_max, alpha_max) = (3.0, 1.0, 0.05);
    let mut ke = vec![1.0; n];
    let mut kh = vec![1.0; n];
    let mut be = vec![0f32; n];
    let mut ce = vec![0f32; n];
    let mut bh = vec![0f32; n];
    let mut ch = vec![0f32; n];
    let mut slot_e = vec![-1i32; n];
    let mut slot_h = vec![-1i32; n];
    if pml > 0 && n > 2 * pml + 2 {
        let left = line[pml];
        let right = line[n - 1 - pml];
        let depth_l = left - line[0];
        let depth_r = line[n - 1] - right;
        let coef = |rho: f64, depth: f64, cell: f64| {
            let r = (rho / depth).clamp(0.0, 1.0);
            let sigma_max = 0.8 * (m + 1.0) / (ETA0 * cell);
            let sigma = sigma_max * r.powf(m);
            let kappa = 1.0 + (kappa_max - 1.0) * r.powf(m);
            let alpha = alpha_max * (1.0 - r) * 2.0 * std::f64::consts::PI * EPS0 * 1e9;
            let b = (-(sigma / kappa + alpha) * dt / EPS0).exp();
            let c = if sigma > 0.0 {
                sigma * (b - 1.0) / (sigma * kappa + kappa * kappa * alpha)
            } else {
                0.0
            };
            (kappa, b, c)
        };
        for i in 0..n {
            let x = line[i];
            let (rho, depth, cell) = if x < left {
                (left - x, depth_l, d[0])
            } else if x > right {
                (x - right, depth_r, d[n - 2])
            } else {
                (0.0, 1.0, 1.0)
            };
            if rho > 0.0 {
                let (k, b, c) = coef(rho, depth, cell);
                ke[i] = k;
                be[i] = b as f32;
                ce[i] = c as f32;
            }
            if i < pml {
                slot_e[i] = i as i32;
            } else if i > n - 1 - pml {
                slot_e[i] = (i + 2 * pml - n) as i32;
            }
            if i + 1 < n {
                let xc = 0.5 * (line[i] + line[i + 1]);
                let (rho, depth, cell) = if xc < left {
                    (left - xc, depth_l, d[0])
                } else if xc > right {
                    (xc - right, depth_r, d[n - 2])
                } else {
                    (0.0, 1.0, 1.0)
                };
                if rho > 0.0 {
                    let (k, b, c) = coef(rho, depth, cell);
                    kh[i] = k;
                    bh[i] = b as f32;
                    ch[i] = c as f32;
                }
                if i < pml {
                    slot_h[i] = i as i32;
                } else if i >= n - 1 - pml {
                    slot_h[i] = (pml + i - (n - 1 - pml)) as i32;
                }
            }
        }
    }
    Axis {
        n,
        inv_d: (0..n).map(|i| (1.0 / (kh[i] * d[i])) as f32).collect(),
        inv_dd: (0..n).map(|i| (1.0 / (ke[i] * dd[i])) as f32).collect(),
        raw_inv_d: d.iter().map(|v| (1.0 / v) as f32).collect(),
        raw_inv_dd: dd.iter().map(|v| (1.0 / v) as f32).collect(),
        d,
        dd,
        be,
        ce,
        bh,
        ch,
        slot_e,
        slot_h,
    }
}

pub fn time_step(grid: &Grid) -> f64 {
    let min = |v: &[f64]| v.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
    let s = 1.0 / min(&grid.x).powi(2) + 1.0 / min(&grid.y).powi(2) + 1.0 / min(&grid.z).powi(2);
    0.99 / (C0 * s.sqrt())
}

pub fn psi_len(n: [usize; 3], axis: usize, pml: usize) -> usize {
    let mut m = n;
    m[axis] = 2 * pml;
    if pml == 0 { 0 } else { m[0] * m[1] * m[2] }
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub comp: usize,
    pub at: [usize; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Element {
    Resistor(f64),
    Capacitor(f64),
    Inductor(f64),
}

#[derive(Clone, Debug)]
pub struct Lumped {
    pub name: String,
    pub edges: Vec<Edge>,
    pub element: Element,
}

#[derive(Clone, Debug)]
pub struct PortDef {
    pub name: String,
    pub columns: Vec<Vec<Edge>>,
    pub r: f64,
}

pub struct Sim {
    pub grid: Grid,
    pub ax: [Axis; 3],
    pub dt: f64,
    pub ca: [Vec<f32>; 3],
    pub cb: [Vec<f32>; 3],
    pub pml: usize,
    pub ports: Vec<PortDef>,
    pub port_src: Vec<Vec<(usize, usize, f32)>>,
    pub inductors: Vec<(usize, usize, f32, f32)>,
    pub sheets: Vec<SheetEdge>,
}

#[derive(Clone, Copy, Debug)]
pub struct SheetEdge {
    pub comp: usize,
    pub id: usize,
    pub g: f64,
    pub x_sig: f64,
    pub c0: f64,
}

pub struct Materials {
    pub eps: Vec<f32>,
    pub sigma: Vec<f32>,
}

impl Materials {
    pub fn new(grid: &Grid) -> Materials {
        let cells = grid.cells();
        Materials { eps: vec![1.0; cells], sigma: vec![0.0; cells] }
    }

    fn at(&self, dims: [usize; 3], c: [isize; 3]) -> (f64, f64) {
        let c = [0, 1, 2].map(|a| c[a].clamp(0, dims[a] as isize - 2) as usize);
        let id = (c[0] * (dims[1] - 1) + c[1]) * (dims[2] - 1) + c[2];
        (self.eps[id] as f64, self.sigma[id] as f64)
    }
}

impl Sim {
    pub fn new(
        grid: Grid,
        mats: &Materials,
        pec: &dyn Fn(usize, [usize; 3]) -> bool,
        lumped: &[Lumped],
        ports: Vec<PortDef>,
        resistive: &[(usize, usize, f64)],
    ) -> Sim {
        let dt = time_step(&grid);
        let pml = grid.pml;
        let ax = [axis(&grid.x, pml, dt), axis(&grid.y, pml, dt), axis(&grid.z, pml, dt)];
        let n = grid.dims();
        let total = n[0] * n[1] * n[2];
        let mut ca = [vec![0f32; total], vec![0f32; total], vec![0f32; total]];
        let mut cb = [vec![0f32; total], vec![0f32; total], vec![0f32; total]];
        let mut eps_edge = [vec![0f64; total], vec![0f64; total], vec![0f64; total]];
        let mut sig_edge = [vec![0f64; total], vec![0f64; total], vec![0f64; total]];
        for c in 0..3 {
            let (u, v) = ((c + 1) % 3, (c + 2) % 3);
            for i in 0..n[0] {
                for j in 0..n[1] {
                    for k in 0..n[2] {
                        let p = [i, j, k];
                        if p[c] + 1 >= n[c]
                            || p[u] == 0
                            || p[u] + 1 >= n[u]
                            || p[v] == 0
                            || p[v] + 1 >= n[v]
                        {
                            continue;
                        }
                        let (mut e_sum, mut s_sum, mut w_sum) = (0.0, 0.0, 0.0);
                        for (ou, ov) in [(-1isize, -1isize), (0, -1), (-1, 0), (0, 0)] {
                            let mut cell = [p[0] as isize, p[1] as isize, p[2] as isize];
                            cell[u] += ou;
                            cell[v] += ov;
                            let w = ax[u].d[cell[u] as usize] * ax[v].d[cell[v] as usize];
                            let (e, s) = mats.at(n, cell);
                            e_sum += e * w;
                            s_sum += s * w;
                            w_sum += w;
                        }
                        let id = idx(n, i, j, k);
                        eps_edge[c][id] = EPS0 * e_sum / w_sum;
                        sig_edge[c][id] = s_sum / w_sum;
                    }
                }
            }
        }
        let area = |e: &Edge| {
            let (u, v) = ((e.comp + 1) % 3, (e.comp + 2) % 3);
            ax[u].dd[e.at[u]] * ax[v].dd[e.at[v]]
        };
        let length = |e: &Edge| ax[e.comp].d[e.at[e.comp]];
        let mut series_r: Vec<(usize, usize, f64)> = Vec::new();
        let mut inductors = Vec::new();
        for l in lumped {
            let count = l.edges.len().max(1) as f64;
            for e in &l.edges {
                let id = idx(n, e.at[0], e.at[1], e.at[2]);
                match l.element {
                    Element::Capacitor(cap) => {
                        eps_edge[e.comp][id] += cap * count * length(e) / area(e)
                    }
                    Element::Resistor(r) => series_r.push((e.comp, id, r / count)),
                    Element::Inductor(_) => {}
                }
            }
        }
        for c in 0..3 {
            for id in 0..total {
                let eps = eps_edge[c][id];
                if eps == 0.0 {
                    continue;
                }
                let x = sig_edge[c][id] * dt / (2.0 * eps);
                ca[c][id] = ((1.0 - x) / (1.0 + x)) as f32;
                cb[c][id] = (dt / eps / (1.0 + x)) as f32;
            }
        }
        for c in 0..3 {
            for i in 0..n[0] {
                for j in 0..n[1] {
                    for k in 0..n[2] {
                        if pec(c, [i, j, k]) {
                            let id = idx(n, i, j, k);
                            ca[c][id] = 0.0;
                            cb[c][id] = 0.0;
                        }
                    }
                }
            }
        }
        let resistor = |ca: &mut [Vec<f32>; 3], cb: &mut [Vec<f32>; 3], e: &Edge, r: f64| -> f64 {
            let id = idx(n, e.at[0], e.at[1], e.at[2]);
            let eps = eps_edge[e.comp][id];
            let x_sig = sig_edge[e.comp][id] * dt / (2.0 * eps);
            let beta = dt * length(e) / (2.0 * r * eps * area(e)) + x_sig;
            ca[e.comp][id] = ((1.0 - beta) / (1.0 + beta)) as f32;
            cb[e.comp][id] = (dt / (eps * (1.0 + beta))) as f32;
            dt / (eps * (1.0 + beta) * r * area(e))
        };
        for (comp, id, r) in &series_r {
            let at = unidx(n, *id);
            resistor(&mut ca, &mut cb, &Edge { comp: *comp, at }, *r);
        }
        let mut sheets = Vec::new();
        for (comp, id, r) in resistive {
            let e = Edge { comp: *comp, at: unidx(n, *id) };
            resistor(&mut ca, &mut cb, &e, *r);
            let eps = eps_edge[*comp][*id];
            sheets.push(SheetEdge {
                comp: *comp,
                id: *id,
                g: dt * length(&e) / (2.0 * r * eps * area(&e)),
                x_sig: sig_edge[*comp][*id] * dt / (2.0 * eps),
                c0: dt / eps,
            });
        }
        let mut port_src = Vec::new();
        for p in &ports {
            let cols = p.columns.len().max(1) as f64;
            let mut v = Vec::new();
            for col in &p.columns {
                let m = col.len().max(1) as f64;
                let re = p.r * cols / m;
                for e in col {
                    let src = resistor(&mut ca, &mut cb, e, re);
                    v.push((e.comp, idx(n, e.at[0], e.at[1], e.at[2]), (src / m) as f32));
                }
            }
            port_src.push(v);
        }
        for l in lumped {
            if let Element::Inductor(ind) = l.element {
                let count = l.edges.len().max(1) as f64;
                for e in &l.edges {
                    let id = idx(n, e.at[0], e.at[1], e.at[2]);
                    let k_e = cb[e.comp][id] as f64 / area(e);
                    let k_i = dt * length(e) / (ind / count);
                    inductors.push((e.comp, id, k_e as f32, k_i as f32));
                }
            }
        }
        Sim { grid, ax, dt, ca, cb, pml, ports, port_src, inductors, sheets }
    }

    pub fn dims(&self) -> [usize; 3] {
        self.grid.dims()
    }
}

pub fn unidx(n: [usize; 3], id: usize) -> [usize; 3] {
    [id / (n[1] * n[2]), (id / n[2]) % n[1], id % n[2]]
}

pub fn pulse_shape(fc: f64) -> (f64, f64) {
    let tau = 10f64.ln().sqrt() / (std::f64::consts::PI * fc);
    (4.0 * tau, tau)
}

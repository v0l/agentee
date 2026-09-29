use crate::gpu::{Gpu, gpu};
use agentee_core::board::{Board, LayerKind};
use serde::Serialize;

pub const ETA0: f64 = 376.730_313_668;

#[derive(Clone, Debug)]
pub struct Grid {
    pub nx: usize,
    pub ny: usize,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub eps: Vec<f32>,
    pub conductor: Vec<i8>,
}

pub const FREE: i8 = 0;
pub const GROUND: i8 = 1;
pub const PLUS: i8 = 2;
pub const MINUS: i8 = 3;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
}

pub fn lines(fixed: &[f64], features: &[(f64, f64)], coarse: f64, ratio: f64) -> Vec<f64> {
    let mut fixed: Vec<f64> = fixed.to_vec();
    fixed.sort_by(f64::total_cmp);
    fixed.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let size = |x: f64| {
        features.iter().map(|(at, s)| s + (ratio - 1.0) * (x - at).abs()).fold(coarse, f64::min)
    };
    let mut out = vec![fixed[0]];
    for w in fixed.windows(2) {
        let (a, b) = (w[0], w[1]);
        let steps = 256;
        let mut acc = vec![0.0; steps + 1];
        for k in 0..steps {
            let x = a + (b - a) * (k as f64 + 0.5) / steps as f64;
            acc[k + 1] = acc[k] + (b - a) / steps as f64 / size(x);
        }
        let n = acc[steps].ceil().max(1.0) as usize;
        for c in 1..n {
            let target = acc[steps] * c as f64 / n as f64;
            let k = acc.partition_point(|v| *v < target).clamp(1, steps);
            let t = (target - acc[k - 1]) / (acc[k] - acc[k - 1]).max(1e-30);
            out.push(a + (b - a) * ((k - 1) as f64 + t) / steps as f64);
        }
        out.push(b);
    }
    out
}

impl Grid {
    pub fn mesh(xs: Vec<f64>, ys: Vec<f64>) -> Grid {
        let (nx, ny) = (xs.len(), ys.len());
        Grid { nx, ny, xs, ys, eps: vec![1.0; (nx - 1) * (ny - 1)], conductor: vec![FREE; nx * ny] }
    }

    fn index(v: &[f64], x: f64) -> usize {
        v.iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map(|a| a.0)
            .unwrap_or(0)
    }

    pub fn dielectric(&mut self, r: Rect, er: f64) {
        for j in 0..self.ny - 1 {
            let yc = 0.5 * (self.ys[j] + self.ys[j + 1]);
            if yc < r.y0 || yc > r.y1 {
                continue;
            }
            for i in 0..self.nx - 1 {
                let xc = 0.5 * (self.xs[i] + self.xs[i + 1]);
                if xc >= r.x0 && xc <= r.x1 {
                    self.eps[j * (self.nx - 1) + i] = er as f32;
                }
            }
        }
    }

    pub fn metal(&mut self, r: Rect, what: i8) {
        let (i0, i1) = (Self::index(&self.xs, r.x0), Self::index(&self.xs, r.x1));
        let (j0, j1) = (Self::index(&self.ys, r.y0), Self::index(&self.ys, r.y1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                self.conductor[j * self.nx + i] = what;
            }
        }
    }

    fn cell_eps(&self, i: isize, j: isize, vacuum: bool) -> f64 {
        let outside = i < 0 || j < 0 || i >= self.nx as isize - 1 || j >= self.ny as isize - 1;
        match (outside, vacuum) {
            (true, _) => 0.0,
            (false, true) => 1.0,
            (false, false) => self.eps[j as usize * (self.nx - 1) + i as usize] as f64,
        }
    }

    fn step(v: &[f64], i: isize) -> f64 {
        if i < 0 || i as usize + 1 >= v.len() { 0.0 } else { v[i as usize + 1] - v[i as usize] }
    }

    fn faces(&self, vacuum: bool) -> (Vec<f32>, Vec<f32>) {
        let (nx, ny) = (self.nx, self.ny);
        let mut gx = vec![0.0f32; nx * ny];
        let mut gy = vec![0.0f32; nx * ny];
        for j in 0..ny as isize {
            for i in 0..nx as isize {
                let k = j as usize * nx + i as usize;
                if (i as usize) + 1 < nx {
                    let dx = Self::step(&self.xs, i);
                    let (up, dn) = (Self::step(&self.ys, j - 1), Self::step(&self.ys, j));
                    let e = self.cell_eps(i, j - 1, vacuum) * up / 2.0
                        + self.cell_eps(i, j, vacuum) * dn / 2.0;
                    gx[k] = (e / dx) as f32;
                }
                if (j as usize) + 1 < ny {
                    let dy = Self::step(&self.ys, j);
                    let (l, r) = (Self::step(&self.xs, i - 1), Self::step(&self.xs, i));
                    let e = self.cell_eps(i - 1, j, vacuum) * l / 2.0
                        + self.cell_eps(i, j, vacuum) * r / 2.0;
                    gy[k] = (e / dy) as f32;
                }
            }
        }
        (gx, gy)
    }

    fn initial(&self) -> Vec<f32> {
        self.conductor
            .iter()
            .map(|c| match *c {
                PLUS => 1.0,
                MINUS => -1.0,
                _ => 0.0,
            })
            .collect()
    }

    fn energy(&self, phi: &[f32], ex: &[f32], ey: &[f32]) -> f64 {
        let (nx, ny) = (self.nx, self.ny);
        let mut s = 0.0f64;
        for j in 0..ny {
            for i in 0..nx {
                let k = j * nx + i;
                if i + 1 < nx {
                    let d = (phi[k + 1] - phi[k]) as f64;
                    s += ex[k] as f64 * d * d;
                }
                if j + 1 < ny {
                    let d = (phi[k + nx] - phi[k]) as f64;
                    s += ey[k] as f64 * d * d;
                }
            }
        }
        s
    }
}

pub struct Solve {
    pub phi: Vec<f32>,
    pub sum: f64,
    pub iterations: usize,
}

fn omega(g: &Grid) -> f32 {
    let n = g.nx.max(g.ny) as f64;
    (2.0 / (1.0 + (std::f64::consts::PI / n).sin())) as f32
}

pub fn solve_cpu(g: &Grid, vacuum: bool, tol: f64) -> Solve {
    let (ex, ey) = g.faces(vacuum);
    let mut phi = g.initial();
    let w = omega(g);
    let (nx, ny) = (g.nx, g.ny);
    let mut last = f64::MAX;
    let batch = 200;
    let mut iterations = 0;
    loop {
        for _ in 0..batch {
            for color in 0..2 {
                for j in 0..ny {
                    for i in ((j + color) % 2..nx).step_by(2) {
                        let k = j * nx + i;
                        if g.conductor[k] != FREE {
                            continue;
                        }
                        let (mut num, mut den) = (0.0f32, 0.0f32);
                        if i > 0 {
                            num += ex[k - 1] * phi[k - 1];
                            den += ex[k - 1];
                        }
                        if i + 1 < nx {
                            num += ex[k] * phi[k + 1];
                            den += ex[k];
                        }
                        if j > 0 {
                            num += ey[k - nx] * phi[k - nx];
                            den += ey[k - nx];
                        }
                        if j + 1 < ny {
                            num += ey[k] * phi[k + nx];
                            den += ey[k];
                        }
                        if den > 0.0 {
                            phi[k] += w * (num / den - phi[k]);
                        }
                    }
                }
            }
        }
        iterations += batch;
        let s = g.energy(&phi, &ex, &ey);
        if ((s - last) / s).abs() < tol || iterations > 200_000 {
            return Solve { phi, sum: s, iterations };
        }
        last = s;
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    nx: u32,
    ny: u32,
    color: u32,
    omega: f32,
}

pub fn solve_gpu(gpu: &Gpu, g: &Grid, vacuum: bool, tol: f64) -> Solve {
    let (ex, ey) = g.faces(vacuum);
    let phi0 = g.initial();
    let fixed: Vec<u32> = g.conductor.iter().map(|c| (*c != FREE) as u32).collect();
    let pipeline = gpu.pipeline(include_str!("laplace.wgsl"), "sor");
    let phi = gpu.storage("phi", &phi0);
    let exb = gpu.storage("ex", &ex);
    let eyb = gpu.storage("ey", &ey);
    let fixb = gpu.storage("fixed", &fixed);
    let w = omega(g);
    let params: Vec<wgpu::Buffer> = (0..2)
        .map(|c| {
            gpu.uniform("params", &Params { nx: g.nx as u32, ny: g.ny as u32, color: c, omega: w })
        })
        .collect();
    let groups: Vec<wgpu::BindGroup> =
        params.iter().map(|u| gpu.bind(&pipeline, &[&phi, &exb, &eyb, &fixb, u])).collect();
    let (wx, wy) = (g.nx.div_ceil(16) as u32, g.ny.div_ceil(16) as u32);
    let batch = 500;
    let mut iterations = 0;
    let mut last = f64::MAX;
    loop {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            for _ in 0..batch {
                for bg in &groups {
                    pass.set_bind_group(0, bg, &[]);
                    pass.dispatch_workgroups(wx, wy, 1);
                }
            }
        }
        gpu.queue.submit([enc.finish()]);
        iterations += batch;
        let out: Vec<f32> = gpu.read(&phi, g.nx * g.ny);
        let s = g.energy(&out, &ex, &ey);
        if ((s - last) / s).abs() < tol || iterations > 400_000 {
            return Solve { phi: out, sum: s, iterations };
        }
        last = s;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Gpu,
    Cpu,
}

#[derive(Clone, Debug, Serialize)]
pub struct Line {
    pub z0: f64,
    pub eeff: f64,
    pub c_pf_per_m: f64,
    pub l_nh_per_m: f64,
    pub delay_ps_per_mm: f64,
    pub differential: bool,
    pub nx: usize,
    pub ny: usize,
    pub smallest_um: f64,
    pub iterations: usize,
    pub backend: Backend,
    pub device: String,
    #[serde(skip)]
    pub phi: Vec<f32>,
    #[serde(skip)]
    pub grid: Option<Grid>,
}

pub fn solve(g: Grid, differential: bool, prefer_gpu: bool, tol: f64) -> Line {
    let device = if prefer_gpu { gpu() } else { None };
    let (a, b, backend, name) = match device {
        Some(d) => (
            solve_gpu(d, &g, false, tol),
            solve_gpu(d, &g, true, tol),
            Backend::Gpu,
            d.name.clone(),
        ),
        None => (solve_cpu(&g, false, tol), solve_cpu(&g, true, tol), Backend::Cpu, "cpu".into()),
    };
    let scale = if differential { 0.5 } else { 1.0 };
    let (s, s0) = (a.sum * scale, b.sum * scale);
    let z_line = ETA0 / (s * s0).sqrt();
    let eeff = s / s0;
    let c = 8.854_187_812_8e-12 * s;
    let v = 299_792_458.0 / eeff.sqrt();
    Line {
        z0: if differential { 2.0 * z_line } else { z_line },
        eeff,
        c_pf_per_m: c * 1e12,
        l_nh_per_m: z_line * z_line * c * 1e9,
        delay_ps_per_mm: 1e9 / v,
        differential,
        nx: g.nx,
        ny: g.ny,
        smallest_um: g
            .xs
            .windows(2)
            .chain(g.ys.windows(2))
            .map(|w| w[1] - w[0])
            .fold(f64::MAX, f64::min)
            * 1000.0,
        iterations: a.iterations + b.iterations,
        backend,
        device: name,
        phi: a.phi,
        grid: Some(g),
    }
}

#[derive(Clone, Debug)]
pub struct Stack {
    pub above: Vec<(f64, f64)>,
    pub below: Vec<(f64, f64)>,
    pub plane_above: bool,
    pub plane_below: bool,
    pub copper: f64,
    pub fill_er: f64,
    pub mask: Option<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub width: f64,
    pub diff_gap: Option<f64>,
    pub coplanar_gap: Option<f64>,
}

impl Stack {
    pub fn from_board(board: &Board, layer: &str, mask: bool) -> Result<Stack, String> {
        let layers = &board.stackup.layers;
        let i = layers
            .iter()
            .position(|l| l.name == layer && l.kind == LayerKind::Copper)
            .ok_or_else(|| {
                format!(
                    "`{layer}` is not a copper layer of {} ({})",
                    board.name,
                    board.stackup.copper_names().join(", ")
                )
            })?;
        let collect = |range: Box<dyn Iterator<Item = usize>>| {
            let mut v = Vec::new();
            let mut plane = false;
            let mut mask_layer = None;
            for k in range {
                let l = &layers[k];
                match l.kind {
                    LayerKind::Copper => {
                        plane = true;
                        break;
                    }
                    LayerKind::Core | LayerKind::Prepreg => v.push((l.thickness.to_mm(), l.er)),
                    LayerKind::Mask => mask_layer = Some((l.thickness.to_mm().max(0.005), l.er)),
                    _ => {}
                }
            }
            (v, plane, mask_layer)
        };
        let (above, plane_above, mask_above) = collect(Box::new((0..i).rev()));
        let (below, plane_below, mask_below) = collect(Box::new(i + 1..layers.len()));
        if above.is_empty() && below.is_empty() {
            return Err(format!("`{layer}` has no dielectric next to it"));
        }
        let fill_er = above.first().or(below.first()).map(|x| x.1).unwrap_or(1.0);
        let outer_mask = if above.is_empty() { mask_above } else { mask_below };
        Ok(Stack {
            above,
            below,
            plane_above,
            plane_below,
            copper: layers[i].thickness.to_mm(),
            fill_er,
            mask: if mask && (!plane_above || !plane_below) { outer_mask } else { None },
        })
    }
}

pub struct Resolution {
    pub edge_cells: f64,
    pub ratio: f64,
    pub reach: f64,
}

impl Resolution {
    pub const FAST: Resolution = Resolution { edge_cells: 40.0, ratio: 1.12, reach: 30.0 };
    pub const FINE: Resolution = Resolution { edge_cells: 160.0, ratio: 1.04, reach: 50.0 };
}

pub fn build(stack: &Stack, trace: &Trace, res: &Resolution) -> Result<(Grid, bool), String> {
    build_mode(stack, trace, res, false)
}

pub fn build_mode(
    stack: &Stack,
    trace: &Trace,
    res: &Resolution,
    even: bool,
) -> Result<(Grid, bool), String> {
    let mut stack = stack.clone();
    if stack.above.len() > stack.below.len() && !stack.plane_below {
        std::mem::swap(&mut stack.above, &mut stack.below);
        std::mem::swap(&mut stack.plane_above, &mut stack.plane_below);
    }
    let h_below: f64 = stack.below.iter().map(|x| x.0).sum();
    let h_above: f64 = stack.above.iter().map(|x| x.0).sum();
    let h = if stack.plane_above { h_above.min(h_below) } else { h_below };
    let (w, t) = (trace.width, stack.copper);
    let span = match trace.diff_gap {
        Some(g) => 2.0 * w + g,
        None => w,
    };
    let mut small = w.min(h);
    if let Some(g) = trace.diff_gap {
        small = small.min(g);
    }
    if let Some(g) = trace.coplanar_gap {
        small = small.min(g);
    }
    let fine = small / res.edge_cells;
    let reach = res.reach * (h_above + h_below).max(span);
    let top = if stack.plane_above { -(h_above) } else { -(reach) };
    let bottom = t + h_below;
    let half = span / 2.0 + reach;
    let x0 = -span / 2.0;
    let strips: Vec<(f64, f64, i8)> = match trace.diff_gap {
        Some(gap) => vec![
            (x0, x0 + w, PLUS),
            (x0 + w + gap, x0 + 2.0 * w + gap, if even { PLUS } else { MINUS }),
        ],
        None => vec![(x0, x0 + w, PLUS)],
    };
    let grounds: Vec<(f64, f64)> = match trace.coplanar_gap {
        Some(gap) => vec![(-half, x0 - gap), (-x0 + gap, half)],
        None => Vec::new(),
    };
    let mask = stack.mask.filter(|_| !stack.plane_above);
    let tm = mask.map(|m| m.0).unwrap_or(0.0);

    let mut fx = vec![-half, half];
    let mut featx = Vec::new();
    for (a, b, _) in &strips {
        fx.extend([*a, *b]);
        featx.extend([(*a, fine), (*b, fine)]);
        if tm > 0.0 {
            fx.extend([a - tm, b + tm]);
        }
    }
    for (a, b) in &grounds {
        for e in [*a, *b] {
            if e.abs() < half - 1e-9 {
                fx.push(e);
                featx.push((e, fine));
                if tm > 0.0 {
                    fx.push(if e < 0.0 { e + tm } else { e - tm });
                }
            }
        }
    }
    let mut fy = vec![top, bottom, 0.0, t];
    let mut feat_y = vec![(0.0, fine), (t, fine)];
    let mut y = 0.0;
    for (th, _) in &stack.above {
        y -= th;
        fy.push(y);
        feat_y.push((y, (th / 4.0).min(fine * 4.0)));
    }
    let mut y = t;
    for (th, _) in &stack.below {
        y += th;
        fy.push(y);
        feat_y.push((y, (th / 4.0).min(fine * 4.0)));
    }
    if tm > 0.0 {
        fy.extend([-tm, t - tm]);
        feat_y.push((-tm, tm / 2.0));
    }
    let coarse_x = (half / 8.0).max(fine);
    let coarse_y = ((bottom - top) / 12.0).max(fine);
    let xs = lines(&fx, &featx, coarse_x, res.ratio);
    let ys = lines(&fy, &feat_y, coarse_y.min(h / 10.0).max(fine), res.ratio);
    let mut g = Grid::mesh(xs, ys);

    let full = |y0: f64, y1: f64| Rect { x0: -half, x1: half, y0, y1 };
    let mut y = 0.0;
    for (th, er) in &stack.above {
        g.dielectric(full(y - th, y), *er);
        y -= th;
    }
    if stack.plane_above {
        g.dielectric(full(0.0, t), stack.fill_er);
    }
    let mut y = t;
    for (th, er) in &stack.below {
        g.dielectric(full(y, y + th), *er);
        y += th;
    }
    if let Some((tm, er)) = mask {
        g.dielectric(full(t - tm, t), er);
        for (a, b, _) in &strips {
            g.dielectric(Rect { x0: a - tm, x1: b + tm, y0: -tm, y1: t }, er);
        }
        for (a, b) in &grounds {
            g.dielectric(Rect { x0: a - tm, x1: b + tm, y0: -tm, y1: t }, er);
        }
    }
    for (a, b, what) in &strips {
        g.metal(Rect { x0: *a, x1: *b, y0: 0.0, y1: t }, *what);
    }
    for (a, b) in &grounds {
        g.metal(Rect { x0: *a, x1: *b, y0: 0.0, y1: t }, GROUND);
    }
    if stack.plane_above {
        g.metal(full(top, top), GROUND);
    }
    if stack.plane_below {
        g.metal(full(bottom, bottom), GROUND);
    }
    Ok((g, trace.diff_gap.is_some()))
}

pub fn line_raw(w: f64, h: f64, t: f64, er: f64) -> Result<Line, String> {
    let stack = Stack {
        above: vec![],
        below: vec![(h, er)],
        plane_above: false,
        plane_below: true,
        copper: t,
        fill_er: er,
        mask: None,
    };
    let (g, d) =
        build(&stack, &Trace { width: w, diff_gap: None, coplanar_gap: None }, &Resolution::FAST)?;
    Ok(solve(g, d, true, 1e-8))
}

pub fn line(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
) -> Result<Line, String> {
    let stack = Stack::from_board(board, layer, mask)?;
    let (g, diff) = build(&stack, trace, if fine { &Resolution::FINE } else { &Resolution::FAST })?;
    Ok(solve(g, diff, true, if fine { 1e-8 } else { 1e-7 }))
}

#[derive(Clone, Debug, Serialize)]
pub struct Pair {
    pub z_diff: f64,
    pub z_common: f64,
    pub z_odd: f64,
    pub z_even: f64,
    pub coupling: f64,
    pub next_long_line: f64,
    pub eeff_odd: f64,
    pub eeff_even: f64,
    pub delay_odd_ps_per_mm: f64,
    pub delay_even_ps_per_mm: f64,
    pub device: String,
}

pub fn pair_of(odd: &Line, even: &Line) -> Pair {
    let (zo, ze) = (odd.z0 / 2.0, even.z0 / 2.0);
    let k = (ze - zo) / (ze + zo);
    Pair {
        z_diff: 2.0 * zo,
        z_common: ze / 2.0,
        z_odd: zo,
        z_even: ze,
        coupling: k,
        next_long_line: k / 2.0,
        eeff_odd: odd.eeff,
        eeff_even: even.eeff,
        delay_odd_ps_per_mm: odd.delay_ps_per_mm,
        delay_even_ps_per_mm: even.delay_ps_per_mm,
        device: odd.device.clone(),
    }
}

pub fn pair(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
) -> Result<Pair, String> {
    if trace.diff_gap.is_none() {
        return Err("a pair needs a gap".into());
    }
    let stack = Stack::from_board(board, layer, mask)?;
    let res = if fine { &Resolution::FINE } else { &Resolution::FAST };
    let tol = if fine { 1e-8 } else { 1e-7 };
    let (go, _) = build_mode(&stack, trace, res, false)?;
    let (ge, _) = build_mode(&stack, trace, res, true)?;
    Ok(pair_of(&solve(go, true, true, tol), &solve(ge, true, true, tol)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentee_core::calc;
    use std::f64::consts::PI;

    fn k(m: f64) -> f64 {
        let (mut a, mut b) = (1.0f64, (1.0 - m * m).sqrt());
        for _ in 0..60 {
            let (x, y) = ((a + b) / 2.0, (a * b).sqrt());
            a = x;
            b = y;
        }
        PI / (2.0 * a)
    }

    fn cohn(w: f64, b: f64, er: f64) -> f64 {
        let m = 1.0 / (PI * w / (2.0 * b)).cosh();
        30.0 * PI / er.sqrt() * k(m) / k((1.0 - m * m).sqrt())
    }

    fn stripline(h: f64, er: f64) -> Stack {
        Stack {
            above: vec![(h, er)],
            below: vec![(h, er)],
            plane_above: true,
            plane_below: true,
            copper: 0.0,
            fill_er: er,
            mask: None,
        }
    }

    fn microstrip(h: f64, er: f64) -> Stack {
        Stack {
            above: vec![],
            below: vec![(h, er)],
            plane_above: false,
            plane_below: true,
            copper: 0.0,
            fill_er: er,
            mask: None,
        }
    }

    fn run(s: &Stack, w: f64) -> Line {
        let (g, d) =
            build(s, &Trace { width: w, diff_gap: None, coplanar_gap: None }, &Resolution::FINE)
                .unwrap();
        solve(g, d, true, 1e-9)
    }

    #[test]
    fn zero_thickness_stripline_matches_cohn_exactly() {
        for (w, h, er) in [(0.2, 0.2, 1.0), (0.4, 0.2, 4.4), (1.0, 0.25, 3.0)] {
            let r = run(&stripline(h, er), w);
            let exact = cohn(w, 2.0 * h, er);
            let err = (r.z0 - exact) / exact;
            eprintln!(
                "stripline w {w} b {}: {:.3} vs {exact:.3} ({:+.3}%) {}x{} {}",
                2.0 * h,
                r.z0,
                err * 100.0,
                r.nx,
                r.ny,
                r.device
            );
            assert!(err.abs() < 0.005, "{err}");
            assert!((r.eeff - er).abs() / er < 0.001);
        }
    }

    #[test]
    fn edge_coupled_stripline_matches_cohn() {
        let cohn = |w: f64, s: f64, b: f64, er: f64| {
            let a = (PI * w / (2.0 * b)).tanh();
            let c = (PI * (w + s) / (2.0 * b)).tanh();
            let z = |m: f64| 30.0 * PI / er.sqrt() * k((1.0 - m * m).sqrt()) / k(m);
            (z(a * c), z(a / c))
        };
        for (w, s, h, er) in [(0.15, 0.15, 0.2, 4.0), (0.1, 0.25, 0.15, 3.5), (0.3, 0.1, 0.25, 1.0)]
        {
            let stack = stripline(h, er);
            let trace = Trace { width: w, diff_gap: Some(s), coplanar_gap: None };
            let (go, _) = build_mode(&stack, &trace, &Resolution::FINE, false).unwrap();
            let (ge, _) = build_mode(&stack, &trace, &Resolution::FINE, true).unwrap();
            let p = pair_of(&solve(go, true, false, 1e-9), &solve(ge, true, false, 1e-9));
            let (ze, zo) = cohn(w, s, 2.0 * h, er);
            eprintln!(
                "w {w} s {s}: even {:.3} vs {ze:.3}, odd {:.3} vs {zo:.3}",
                p.z_even, p.z_odd
            );
            assert!((p.z_even - ze).abs() / ze < 0.01, "even {} vs {ze}", p.z_even);
            assert!((p.z_odd - zo).abs() / zo < 0.01, "odd {} vs {zo}", p.z_odd);
        }
    }

    #[test]
    fn zero_thickness_microstrip_matches_hammerstad_jensen() {
        for (w, h, er) in
            [(0.1, 0.2, 1.0), (0.2, 0.2, 1.0), (0.6, 0.2, 1.0), (0.37, 0.21, 4.4), (2.9, 1.51, 4.5)]
        {
            let r = run(&microstrip(h, er), w);
            let hj = calc::microstrip_z0(w, 0.0, h, er);
            let err = (r.z0 - hj) / hj;
            eprintln!(
                "microstrip w {w} h {h} er {er}: {:.3} vs {hj:.3} ({:+.3}%) eeff {:.4} {}x{}",
                r.z0,
                err * 100.0,
                r.eeff,
                r.nx,
                r.ny
            );
            assert!(err.abs() < 0.005, "{err}");
        }
    }

    #[test]
    fn gpu_agrees_with_cpu() {
        let Some(d) = gpu() else { return };
        let (g, _) = build(
            &microstrip(0.2, 4.4),
            &Trace { width: 0.3, diff_gap: None, coplanar_gap: None },
            &Resolution::FAST,
        )
        .unwrap();
        let a = solve_cpu(&g, false, 1e-8);
        let b = solve_gpu(d, &g, false, 1e-8);
        assert!((a.sum - b.sum).abs() / a.sum < 1e-3, "{} {}", a.sum, b.sum);
    }
}

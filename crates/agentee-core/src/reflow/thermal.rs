use super::defaults::Family;

pub const BODY_DENSITY_G_MM3: f64 = 0.0020;
pub const BODY_CP_J_G_K: f64 = 0.8;
pub const FR4_DENSITY_G_MM3: f64 = 0.00185;
pub const FR4_CP_J_G_K: f64 = 0.9;
pub const COPPER_DENSITY_G_MM3: f64 = 0.00896;
pub const COPPER_CP_J_G_K: f64 = 0.385;
pub const REFLOW_OVEN_H_W_M2K: f64 = 60.0;
pub const TOASTER_OVEN_H_W_M2K: f64 = 25.0;
pub const HOTPLATE_H_W_M2K: f64 = 40.0;
pub const BOARD_COPPER_FILL: f64 = 0.5;
pub const TAU_FLOOR_S: f64 = 1.0;
pub const PAD_CONDUCTANCE_W_M2K: f64 = 500.0;
pub const SURFACE_TENSION_G_PER_MM2: f64 = 30.0 / 645.16;

#[derive(Clone, Debug)]
pub struct Board {
    pub thickness_mm: f64,
    pub copper_mm: f64,
    pub copper_fill: f64,
    pub h_eff: f64,
}

impl Board {
    pub fn tau(&self) -> f64 {
        let per_mm2 = self.thickness_mm * FR4_DENSITY_G_MM3 * FR4_CP_J_G_K
            + self.copper_mm * self.copper_fill * COPPER_DENSITY_G_MM3 * COPPER_CP_J_G_K;
        (per_mm2 / (2.0 * self.h_eff * 1e-6)).max(TAU_FLOOR_S)
    }
}

#[derive(Clone, Debug)]
pub struct Geometry {
    pub family: Family,
    pub body: Option<[f64; 3]>,
    pub footprint: [f64; 2],
    pub pad_area_mm2: f64,
    pub mass_g: Option<f64>,
    pub local_copper_fill: Option<f64>,
}

pub fn body_mass_g(g: &Geometry) -> f64 {
    let [l, w] = g.footprint;
    g.mass_g.unwrap_or_else(|| match g.body {
        Some([l, w, h]) => l * w * h * BODY_DENSITY_G_MM3,
        None => l * w * BODY_DENSITY_G_MM3,
    })
}

pub fn own_tau(b: &Board, g: &Geometry) -> f64 {
    let [l, w] = g.footprint;
    let h = g.body.map(|x| x[2]).unwrap_or(1.0);
    let mut c = body_mass_g(g) * BODY_CP_J_G_K;
    if let Some(local) = g.local_copper_fill {
        let excess = (local - b.copper_fill).max(0.0);
        c += l * w * b.copper_mm * excess * COPPER_DENSITY_G_MM3 * COPPER_CP_J_G_K;
    }
    let area = l * w + 2.0 * (l + w) * h;
    let conductance = (b.h_eff * area + PAD_CONDUCTANCE_W_M2K * g.pad_area_mm2) * 1e-6;
    (c / conductance).max(TAU_FLOOR_S)
}

pub fn oven_for_board(board: &[f64], tau: f64, dt: f64, ambient: f64) -> Vec<f64> {
    let n = board.len();
    (0..n)
        .map(|i| {
            let next = board[(i + 1).min(n - 1)];
            let prev = board[i.saturating_sub(1)];
            let slope = (next - prev) / (dt * if i == 0 || i + 1 == n { 1.0 } else { 2.0 });
            (board[i] + tau * slope).max(ambient)
        })
        .collect()
}

pub fn follow(drive: &[f64], tau: f64, dt: f64, start: f64) -> Vec<f64> {
    let k = 1.0 - (-dt / tau).exp();
    let mut t = start;
    drive
        .iter()
        .map(|&d| {
            t += (d - t) * k;
            t
        })
        .collect()
}

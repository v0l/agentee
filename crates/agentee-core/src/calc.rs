use serde::Serialize;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceGeometry {
    Microstrip { h_mm: f64, er: f64, t_mm: f64 },
    Stripline { h1_mm: f64, h2_mm: f64, er: f64, t_mm: f64 },
}

impl TraceGeometry {
    pub fn is_external(&self) -> bool {
        matches!(self, TraceGeometry::Microstrip { .. })
    }

    pub fn copper_mm(&self) -> f64 {
        match *self {
            TraceGeometry::Microstrip { t_mm, .. } | TraceGeometry::Stripline { t_mm, .. } => t_mm,
        }
    }

    pub fn single_ended(&self, w_mm: f64) -> f64 {
        match *self {
            TraceGeometry::Microstrip { h_mm, er, t_mm } => microstrip_z0(w_mm, t_mm, h_mm, er),
            TraceGeometry::Stripline { h1_mm, h2_mm, er, t_mm } => {
                stripline_z0(w_mm, t_mm, h1_mm, h2_mm, er)
            }
        }
    }

    pub fn differential(&self, w_mm: f64, gap_mm: f64) -> f64 {
        let z0 = self.single_ended(w_mm);
        match *self {
            TraceGeometry::Microstrip { h_mm, .. } => {
                2.0 * z0 * (1.0 - 0.48 * (-0.96 * gap_mm / h_mm).exp())
            }
            TraceGeometry::Stripline { h1_mm, h2_mm, t_mm, .. } => {
                let b = h1_mm + h2_mm + t_mm;
                2.0 * z0 * (1.0 - 0.347 * (-2.9 * gap_mm / b).exp())
            }
        }
    }

    pub fn impedance(&self, w_mm: f64, gap_mm: Option<f64>) -> f64 {
        match gap_mm {
            Some(g) => self.differential(w_mm, g),
            None => self.single_ended(w_mm),
        }
    }

    pub fn width_for(&self, target: f64, gap_mm: Option<f64>) -> Option<f64> {
        solve_decreasing(|w| self.impedance(w, gap_mm), target, 0.01, 20.0)
    }
}

fn hj_z01(u: f64) -> f64 {
    let f = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
    60.0 * (f / u + (1.0 + 4.0 / (u * u)).sqrt()).ln()
}

fn hj_eeff(u: f64, er: f64) -> f64 {
    let a = 1.0
        + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0
        + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b)
}

pub fn microstrip_z0(w: f64, t: f64, h: f64, er: f64) -> f64 {
    let u = w / h;
    let ur = if t > 0.0 {
        let th = t / h;
        let coth = 1.0 / (6.517 * u).sqrt().tanh();
        let du1 = th / PI * (1.0 + 4.0 * std::f64::consts::E / (th * coth * coth)).ln();
        u + 0.5 * (1.0 + 1.0 / (er - 1.0).max(0.0).sqrt().cosh()) * du1
    } else {
        u
    };
    hj_z01(ur) / hj_eeff(ur, er).sqrt()
}

fn symmetric_stripline_z0(w: f64, t: f64, b: f64, er: f64) -> f64 {
    let x = t / b;
    let dw = if x > 0.0 {
        let m = 2.0 / (1.0 + 2.0 / 3.0 * x / (1.0 - x));
        x / (PI * (1.0 - x))
            * (1.0
                - 0.5 * ((x / (2.0 - x)).powi(2) + (0.0796 * x / (w / b + 1.1 * x)).powf(m)).ln())
    } else {
        0.0
    };
    let k = 4.0 / (PI * (w / (b - t) + dw));
    30.0 / er.sqrt() * (1.0 + k * (2.0 * k + ((2.0 * k).powi(2) + 6.27).sqrt())).ln()
}

pub fn stripline_z0(w: f64, t: f64, h1: f64, h2: f64, er: f64) -> f64 {
    let z1 = symmetric_stripline_z0(w, t, 2.0 * h1 + t, er);
    let z2 = symmetric_stripline_z0(w, t, 2.0 * h2 + t, er);
    2.0 * z1 * z2 / (z1 + z2)
}

const IPC2221_EXTERNAL: f64 = 0.048;
const IPC2221_INTERNAL: f64 = 0.024;

pub fn ipc2221_width(amps: f64, rise_c: f64, copper_mm: f64, external: bool) -> f64 {
    let k = if external { IPC2221_EXTERNAL } else { IPC2221_INTERNAL };
    let area_mil2 = (amps / (k * rise_c.powf(0.44))).powf(1.0 / 0.725);
    let t_mil = copper_mm / crate::units::MM_PER_MIL;
    area_mil2 / t_mil * crate::units::MM_PER_MIL
}

pub fn ipc2221_current(width_mm: f64, rise_c: f64, copper_mm: f64, external: bool) -> f64 {
    let k = if external { IPC2221_EXTERNAL } else { IPC2221_INTERNAL };
    let area_mil2 = (width_mm / crate::units::MM_PER_MIL) * (copper_mm / crate::units::MM_PER_MIL);
    k * rise_c.powf(0.44) * area_mil2.powf(0.725)
}

fn solve_decreasing(f: impl Fn(f64) -> f64, target: f64, lo: f64, hi: f64) -> Option<f64> {
    let (mut lo, mut hi) = (lo, hi);
    if f(lo) < target || f(hi) > target {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microstrip_on_jlc_7628_is_about_fifty_ohm_at_a_third_of_a_mm() {
        let z = microstrip_z0(0.35, 0.035, 0.2104, 4.4);
        assert!((46.0..54.0).contains(&z), "{z}");
    }

    #[test]
    fn microstrip_on_1_6mm_fr4_is_about_fifty_ohm_at_three_mm() {
        let z = microstrip_z0(2.9, 0.035, 1.51, 4.5);
        assert!((47.0..54.0).contains(&z), "{z}");
    }

    #[test]
    fn symmetric_stripline_matches_the_ipc_form() {
        let z = stripline_z0(0.15, 0.0152, 0.2, 0.2, 4.4);
        let direct = symmetric_stripline_z0(0.15, 0.0152, 0.4152, 4.4);
        assert!((z - direct).abs() < 1e-9);
    }

    #[test]
    fn width_solver_inverts_impedance() {
        let g = TraceGeometry::Microstrip { h_mm: 0.2104, er: 4.4, t_mm: 0.035 };
        let w = g.width_for(50.0, None).unwrap();
        assert!((g.single_ended(w) - 50.0).abs() < 0.01);
        let wd = g.width_for(90.0, Some(0.15)).unwrap();
        assert!((g.differential(wd, 0.15) - 90.0).abs() < 0.01);
    }

    #[test]
    fn ipc2221_round_trips() {
        let w = ipc2221_width(1.0, 10.0, 0.035, true);
        assert!((0.25..0.45).contains(&w), "{w}");
        assert!((ipc2221_current(w, 10.0, 0.035, true) - 1.0).abs() < 1e-6);
        assert!(ipc2221_width(1.0, 10.0, 0.035, false) > w * 2.0);
    }
}

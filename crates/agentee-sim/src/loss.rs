use crate::gpu::gpu;
use crate::xsection::{self, Grid, Resolution, Stack, Trace};
use agentee_core::board::Board;
use agentee_core::rf::Cx;
use serde::Serialize;
use std::f64::consts::PI;

pub const EPS0: f64 = 8.854_187_812_8e-12;
pub const MU0: f64 = 1.256_637_062_12e-6;
pub const C0: f64 = 299_792_458.0;
pub const COPPER: f64 = 1.7241e-8;
pub const GOLD: f64 = 2.44e-8;
pub const NICKEL: f64 = 1.0e-7;
pub const REFERENCE_HZ: f64 = 1e9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Roughness {
    pub rms_um: Option<f64>,
    pub huray_radius_um: Option<f64>,
    pub huray_ratio: Option<f64>,
}

impl Roughness {
    pub fn factor(&self, skin_m: f64) -> f64 {
        if let (Some(a), Some(sr)) = (self.huray_radius_um, self.huray_ratio) {
            let a = a * 1e-6;
            let d = skin_m;
            return 1.0 + 1.5 * sr / (1.0 + d / a + d * d / (2.0 * a * a));
        }
        match self.rms_um {
            Some(r) => 1.0 + 2.0 / PI * (1.4 * (r * 1e-6 / skin_m).powi(2)).atan(),
            None => 1.0,
        }
    }

    pub fn causal(&self, s: Cx) -> Cx {
        if let (Some(a), Some(sr)) = (self.huray_radius_um, self.huray_ratio) {
            let a = a * 1e-6;
            let q = csqrt(s * (MU0 * a * a / COPPER));
            return Cx::ONE + q / (Cx::ONE + q) * (1.5 * sr);
        }
        match self.rms_um {
            Some(r) => {
                let r = r * 1e-6;
                let x = s * (0.7 * MU0 * r * r / COPPER);
                let q = csqrt(x);
                let k = cln(Cx::ONE + q * 2.0 / (Cx::ONE + x)) + catan(q) * 2.0;
                Cx::ONE + k * (1.0 / PI)
            }
            None => Cx::ONE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Enig {
    pub nickel_um: f64,
    pub gold_um: f64,
}

impl Default for Enig {
    fn default() -> Self {
        Enig { nickel_um: 4.5, gold_um: 0.075 }
    }
}

pub fn nickel_permeability(s: Cx) -> Cx {
    let (low, high, f0, damping) = (6.0, 2.0, 2.6e9, 0.18);
    let w0 = 2.0 * PI * f0;
    let g = damping * w0;
    let num = Cx::new(w0 * w0, 0.0) + s * g;
    let den = Cx::new(w0 * w0, 0.0) + s * (2.0 * g) + s * s;
    Cx::new(high, 0.0) + num / den * (low - high)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    Copper(Roughness),
    Enig(Enig),
}

pub fn surface_impedance(face: Face, copper_m: f64, s: Cx) -> Cx {
    let slab = metal_layer(None, COPPER, Cx::ONE, copper_m, s);
    match face {
        Face::Copper(r) => slab * r.causal(s),
        Face::Enig(e) => {
            let ni = metal_layer(Some(slab), NICKEL, nickel_permeability(s), e.nickel_um * 1e-6, s);
            metal_layer(Some(ni), GOLD, Cx::ONE, e.gold_um * 1e-6, s)
        }
    }
}

fn metal_layer(load: Option<Cx>, rho: f64, mu: Cx, d: f64, s: Cx) -> Cx {
    let eta = csqrt(s * mu * (MU0 * rho));
    let t = ctanh(csqrt(s * mu * (MU0 / rho)) * d);
    match load {
        None => eta / t,
        Some(z) => eta * (z + eta * t) / (eta + z * t),
    }
}

pub fn csqrt(z: Cx) -> Cx {
    let m = z.abs();
    let re = ((m + z.re) / 2.0).max(0.0).sqrt();
    let im = ((m - z.re) / 2.0).max(0.0).sqrt();
    Cx::new(re, if z.im < 0.0 { -im } else { im })
}

fn cln(z: Cx) -> Cx {
    Cx::new(z.abs().ln(), z.im.atan2(z.re))
}

fn ctanh(z: Cx) -> Cx {
    let m = (-2.0 * z.re).exp();
    let e = Cx::new(m * (-2.0 * z.im).cos(), m * (-2.0 * z.im).sin());
    (Cx::ONE - e) / (Cx::ONE + e)
}

fn catan(z: Cx) -> Cx {
    let i = Cx::new(0.0, 1.0);
    (cln(Cx::ONE - i * z) - cln(Cx::ONE + i * z)) * i * 0.5
}

pub fn djordjevic_sarkar(er: f64, tan: f64, f_ref: f64, f: f64) -> (f64, f64) {
    if tan <= 0.0 {
        return (er, 0.0);
    }
    let (w1, w2) = (2.0 * PI * 1e3, 2.0 * PI * 1e12);
    let span = (w2 / w1).log10();
    let g = |f: f64| {
        let w = 2.0 * PI * f;
        let num = (w2 * w2 + w * w).sqrt() / (w1 * w1 + w * w).sqrt();
        let ang = (w / w2).atan() - (w / w1).atan();
        (num.ln() / std::f64::consts::LN_10 / span, ang / std::f64::consts::LN_10 / span)
    };
    let (gr, gi) = g(f_ref);
    let delta = -er * tan / gi;
    let inf = er - delta * gr;
    let (fr, fi) = g(f);
    let real = inf + delta * fr;
    (real, -delta * fi / real)
}

#[derive(Clone, Debug, Serialize)]
pub struct Point {
    pub freq_hz: f64,
    pub r_ohm_per_m: f64,
    pub l_nh_per_m: f64,
    pub g_ms_per_m: f64,
    pub c_pf_per_m: f64,
    pub z0_ohm: f64,
    pub alpha_db_per_m: f64,
    pub conductor_db_per_m: f64,
    pub dielectric_db_per_m: f64,
    pub db_per_inch: f64,
    pub eeff: f64,
    pub delay_ps_per_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sweep {
    pub mode: String,
    pub rdc_ohm_per_m: f64,
    pub surface_factor_per_m: f64,
    pub points: Vec<Point>,
    #[serde(skip)]
    pub materials: Vec<(f32, f32, f64)>,
    #[serde(skip)]
    pub c0_per_line: f64,
}

pub struct Model {
    pub resistivity: f64,
    pub roughness: Roughness,
    pub f_ref: f64,
}

impl Default for Model {
    fn default() -> Self {
        Model { resistivity: COPPER, roughness: Roughness::default(), f_ref: REFERENCE_HZ }
    }
}

pub fn inductance(g: &Grid, pair: bool, tol: f64) -> f64 {
    let b = match gpu() {
        Some(d) => xsection::solve_gpu(d, g, true, tol),
        None => xsection::solve_cpu(g, true, tol),
    };
    let scale = if pair { 0.5 } else { 1.0 };
    1.0 / (C0 * C0 * EPS0 * b.sum * scale)
}

pub fn wheeler(
    stack: &Stack,
    trace: &Trace,
    res: &Resolution,
    even: bool,
    tol: f64,
) -> Result<f64, String> {
    let pair = trace.diff_gap.is_some();
    let dn = [
        trace.width,
        stack.copper.max(1e-9),
        trace.diff_gap.unwrap_or(1.0),
        trace.coplanar_gap.unwrap_or(1.0),
    ]
    .into_iter()
    .fold(f64::MAX, f64::min)
        / 40.0;
    let receded = |k: f64| -> Result<f64, String> {
        let d = dn * k;
        let mut st = stack.clone();
        st.copper = (stack.copper - 2.0 * d).max(0.0);
        if st.plane_above
            && let Some(l) = st.above.last_mut()
        {
            l.0 += 2.0 * d;
        }
        if st.plane_below
            && let Some(l) = st.below.last_mut()
        {
            l.0 += 2.0 * d;
        }
        let tr = Trace {
            width: trace.width - 2.0 * d,
            diff_gap: trace.diff_gap.map(|g| g + 2.0 * d),
            coplanar_gap: trace.coplanar_gap.map(|g| g + 2.0 * d),
        };
        let (g, _) = xsection::build_mode(&st, &tr, res, even)?;
        Ok(inductance(&g, pair, tol))
    };
    let (l0, l1, l2) = (receded(0.0)?, receded(1.0)?, receded(2.0)?);
    Ok((4.0 * (l1 - l0) - (l2 - l0)) / (2.0 * dn * 1e-3) / MU0)
}

#[allow(clippy::too_many_arguments)]
pub fn sweep(
    g: &Grid,
    pair: bool,
    strip_mm2: f64,
    geom: f64,
    model: &Model,
    freqs: &[f64],
    mode: &str,
    tol: f64,
) -> Sweep {
    let (a, b) = match gpu() {
        Some(d) => (xsection::solve_gpu(d, g, false, tol), xsection::solve_gpu(d, g, true, tol)),
        None => (xsection::solve_cpu(g, false, tol), xsection::solve_cpu(g, true, tol)),
    };
    let scale = if pair { 0.5 } else { 1.0 };
    let materials = g.materials(&a.phi);
    let norm = a.sum / materials.iter().map(|m| m.0 as f64 * m.2).sum::<f64>().max(1e-30);
    let c0 = EPS0 * b.sum * scale;
    let l_ext = 1.0 / (C0 * C0 * c0);
    let rdc = model.resistivity / (strip_mm2 * 1e-6);
    let points = freqs
        .iter()
        .map(|&f| {
            let w = 2.0 * PI * f;
            let skin = (model.resistivity / (PI * f * MU0)).sqrt();
            let rs = (PI * f * MU0 * model.resistivity).sqrt();
            let r_smooth = rs * geom;
            let rac = r_smooth * model.roughness.factor(skin);
            let r = (rdc * rdc + rac * rac).sqrt();
            let l = l_ext + r_smooth / w;
            let (mut cs, mut gs) = (0.0, 0.0);
            for (er, tan, part) in &materials {
                let (e, t) = djordjevic_sarkar(*er as f64, *tan as f64, model.f_ref, f);
                cs += part * norm * e;
                gs += part * norm * e * t;
            }
            let c = EPS0 * cs * scale;
            let gg = w * EPS0 * gs * scale;
            let z = num::sqrt_div((r, w * l), (gg, w * c));
            let gamma = num::sqrt_mul((r, w * l), (gg, w * c));
            let zmag = (z.0 * z.0 + z.1 * z.1).sqrt();
            let np_db = 20.0 / std::f64::consts::LN_10;
            let alpha_c = rac / (2.0 * zmag);
            let alpha_d = gg * zmag / 2.0;
            let eeff = (gamma.1 / (w / C0)).powi(2);
            Point {
                freq_hz: f,
                r_ohm_per_m: r,
                l_nh_per_m: l * 1e9,
                g_ms_per_m: gg * 1e3,
                c_pf_per_m: c * 1e12,
                z0_ohm: zmag,
                alpha_db_per_m: gamma.0 * np_db,
                conductor_db_per_m: alpha_c * np_db,
                dielectric_db_per_m: alpha_d * np_db,
                db_per_inch: gamma.0 * np_db * 0.0254,
                eeff,
                delay_ps_per_mm: gamma.1 / w * 1e9,
            }
        })
        .collect();
    Sweep {
        mode: mode.into(),
        rdc_ohm_per_m: rdc,
        surface_factor_per_m: geom,
        points,
        materials,
        c0_per_line: c0,
    }
}

mod num {
    fn sqrt(z: (f64, f64)) -> (f64, f64) {
        let m = (z.0 * z.0 + z.1 * z.1).sqrt();
        let re = ((m + z.0) / 2.0).max(0.0).sqrt();
        let im = ((m - z.0) / 2.0).max(0.0).sqrt();
        (re, if z.1 < 0.0 { -im } else { im })
    }

    pub fn sqrt_mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        sqrt((a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0))
    }

    pub fn sqrt_div(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        let d = b.0 * b.0 + b.1 * b.1;
        sqrt(((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d))
    }
}

pub fn line(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
    model: &Model,
    freqs: &[f64],
) -> Result<Vec<Sweep>, String> {
    let stack = Stack::from_board(board, layer, mask)?;
    let res = if fine { &Resolution::FINE } else { &Resolution::FAST };
    let tol = if fine { 1e-8 } else { 1e-7 };
    let area = trace.width * stack.copper;
    let modes: &[(bool, &str)] = if trace.diff_gap.is_some() {
        &[(false, "odd"), (true, "even")]
    } else {
        &[(false, "single")]
    };
    modes
        .iter()
        .map(|(even, name)| {
            let (g, _) = xsection::build_mode(&stack, trace, res, *even)?;
            let geom = wheeler(&stack, trace, res, *even, tol)?;
            Ok(sweep(&g, trace.diff_gap.is_some(), area, geom, model, freqs, name, tol))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripline(h: f64, t: f64, er: f64, tan: f64) -> Stack {
        Stack {
            above: vec![(h, er, tan)],
            below: vec![(h, er, tan)],
            plane_above: true,
            plane_below: true,
            copper: t,
            fill_er: er,
            mask: None,
        }
    }

    #[test]
    fn roughness_factors_have_their_published_limits() {
        let r = Roughness { rms_um: Some(1.0), ..Default::default() };
        assert!((r.factor(1e-6) - (1.0 + 2.0 / PI * 1.4f64.atan())).abs() < 1e-12);
        assert!((r.factor(1e-9) - 2.0).abs() < 1e-3);
        assert!((r.factor(1e-3) - 1.0).abs() < 1e-3);
        let h =
            Roughness { huray_radius_um: Some(0.5), huray_ratio: Some(2.0), ..Default::default() };
        assert!((h.factor(1e-12) - 4.0).abs() < 1e-3);
        assert!((h.factor(1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn causal_roughness_matches_dmitriev_zdorov_and_simonovich_table_1() {
        let rms = 1.0;
        let r = Roughness { rms_um: Some(rms), ..Default::default() };
        for x in [0.01f64, 0.3, 1.0, 2.0, 10.0, 300.0] {
            let w = x * COPPER / (0.7 * MU0 * (rms * 1e-6).powi(2));
            let k = r.causal(Cx::new(0.0, w)) - Cx::ONE;
            let loss = 2.0 / PI * x.atan();
            let q = (2.0 * x).sqrt();
            let inductance = (((1.0 + q + x) / (1.0 - q + x)).ln() + 2.0 * q.atan2(1.0 - x)
                - 2.0 * x.atan())
                / PI;
            assert!((k.re - k.im - loss).abs() < 1e-12, "{x}: {} vs {loss}", k.re - k.im);
            assert!((k.re + k.im - inductance).abs() < 1e-12, "{x}: {} {inductance}", k.re + k.im);
            let skin = (2.0 * COPPER / (w * MU0)).sqrt();
            let smooth = Cx::new(1.0, 1.0) * r.causal(Cx::new(0.0, w));
            assert!((smooth.re - r.factor(skin)).abs() < 1e-12);
        }
        let h =
            Roughness { huray_radius_um: Some(0.5), huray_ratio: Some(2.0), ..Default::default() };
        for f in [1e8, 1e9, 1e10, 1e11] {
            let w = 2.0 * PI * f;
            let skin = (2.0 * COPPER / (w * MU0)).sqrt();
            let got = (Cx::new(1.0, 1.0) * h.causal(Cx::new(0.0, w))).re;
            assert!((got - h.factor(skin)).abs() < 1e-12, "{f}: {got} {}", h.factor(skin));
        }
    }

    #[test]
    fn enig_face_reduces_to_its_metals_in_the_limits() {
        let s = |f: f64| Cx::new(0.0, 2.0 * PI * f);
        for f in [1e8, 2.6e9, 4e10] {
            let cu = csqrt(s(f) * (MU0 * COPPER));
            let bare = Enig { nickel_um: 0.0, gold_um: 0.0 };
            let z = surface_impedance(Face::Enig(bare), 1e-3, s(f));
            assert!((z - cu).abs() / cu.abs() < 1e-9, "{f}");
            let thick = Enig { nickel_um: 1000.0, gold_um: 0.0 };
            let ni = csqrt(s(f) * nickel_permeability(s(f)) * (MU0 * NICKEL));
            let z = surface_impedance(Face::Enig(thick), 1e-3, s(f));
            assert!((z - ni).abs() / ni.abs() < 1e-9, "{f}");
            let rough = Roughness { rms_um: Some(2.0), ..Default::default() };
            let z = surface_impedance(Face::Copper(rough), 1e-3, s(f));
            assert!((z - cu * rough.causal(s(f))).abs() / cu.abs() < 1e-9);
        }
        let dc = surface_impedance(Face::Copper(Roughness::default()), 35e-6, s(1.0));
        assert!((dc.re / (COPPER / 35e-6) - 1.0).abs() < 1e-6, "{dc:?}");
        let mu = nickel_permeability(Cx::ZERO);
        assert!((mu.re - 6.0).abs() < 1e-12 && mu.im.abs() < 1e-12);
        let mu = nickel_permeability(s(1e14));
        assert!((mu.re - 2.0).abs() < 1e-3);
    }

    #[test]
    fn djordjevic_sarkar_returns_the_reference_values_and_stays_causal() {
        let (e, t) = djordjevic_sarkar(4.3, 0.02, 1e9, 1e9);
        assert!((e - 4.3).abs() < 1e-12 && (t - 0.02).abs() < 1e-12);
        let (e_lo, _) = djordjevic_sarkar(4.3, 0.02, 1e9, 1e6);
        let (e_hi, _) = djordjevic_sarkar(4.3, 0.02, 1e9, 20e9);
        assert!(e_lo > 4.3 && e_hi < 4.3, "{e_lo} {e_hi}");
    }

    #[test]
    fn homogeneous_stripline_has_the_tem_dielectric_loss() {
        let (er, tan) = (4.0, 0.02);
        let stack = stripline(0.2, 0.0, er, tan);
        let trace = Trace { width: 0.15, diff_gap: None, coplanar_gap: None };
        let (g, _) = xsection::build(&stack, &trace, &Resolution::FAST).unwrap();
        let model = Model { resistivity: 1e-30, ..Model::default() };
        let s = sweep(&g, false, 0.15 * 0.035, 0.0, &model, &[1e9], "single", 1e-9);
        let want = PI * 1e9 * er.sqrt() * tan / C0 * 20.0 / std::f64::consts::LN_10;
        let got = s.points[0].dielectric_db_per_m;
        assert!((got - want).abs() / want < 0.002, "{got} vs {want}");
    }

    #[test]
    fn thick_stripline_matches_the_wheeler_formula_in_pozar() {
        let (h, t, w) = (0.2, 0.035, 0.15);
        let trace = Trace { width: w, diff_gap: None, coplanar_gap: None };
        let stack = stripline(h, t, 1.0, 0.0);
        let (g, _) = xsection::build(&stack, &trace, &Resolution::FINE).unwrap();
        let z0 = inductance(&g, false, 1e-10) * C0;
        let ours = wheeler(&stack, &trace, &Resolution::FINE, false, 1e-10).unwrap();
        let (wm, tm, b) = (w * 1e-3, t * 1e-3, (2.0 * h + t) * 1e-3);
        let a = 1.0 + 2.0 * wm / (b - tm) + (b + tm) / (b - tm) / PI * ((2.0 * b - tm) / tm).ln();
        let pozar = 2.0 * z0 * 2.7e-3 * z0 / (30.0 * PI * (b - tm)) * a;
        eprintln!("Z0 {z0:.2}, R/Rs {ours:.0} /m, Pozar {pozar:.0} /m");
        assert!(z0 < 120.0);
        assert!((ours / pozar - 1.0).abs() < 0.03, "{ours} {pozar}");
    }

    #[test]
    fn wide_stripline_approaches_the_parallel_plate_resistance() {
        let (h, t, w) = (0.1, 0.035, 8.0);
        let trace = Trace { width: w, diff_gap: None, coplanar_gap: None };
        let stack = stripline(h, t, 1.0, 0.0);
        let wh = wheeler(&stack, &trace, &Resolution::FAST, false, 1e-9).unwrap();
        let plates = 1.0 / (w * 1e-3);
        eprintln!("wide strip {wh:.1} /m, parallel plates {plates:.1} /m");
        assert!((wh / plates - 1.0).abs() < 0.03, "{wh} {plates}");
    }
}

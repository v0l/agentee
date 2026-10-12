use agentee_core::circuit::{
    At, CircuitResult, CircuitSpec, Curve, ElemKind, Measure, Node, Reading, Wave, fmt_si,
};
use faer::c64;
use faer::linalg::solvers::Solve;
use faer::sparse::{SparseColMat, Triplet};
use std::collections::BTreeMap;

const GMIN: f64 = 1e-12;
const BOLTZMANN: f64 = 1.380_649e-23;
const CHARGE: f64 = 1.602_176_634e-19;
const EPS_SOFT: f64 = 0.02;
const ZENER_KNEE: f64 = 1e-3;

type Ix = Option<usize>;

#[derive(Clone, Debug)]
enum Unit {
    R { a: Ix, b: Ix, g: f64 },
    C { a: Ix, b: Ix, c: f64 },
    L { a: Ix, b: Ix, l: f64, j: usize },
    D { a: Ix, k: Ix, is: f64, n: f64, bv: Option<f64>, mem: usize },
    Q { c: Ix, b: Ix, e: Ix, pnp: bool, is: f64, bf: f64, br: f64, va: Option<f64>, mem: usize },
    M { d: Ix, g: Ix, s: Ix, p: bool, vth: f64, k: f64, lambda: f64, mem: usize },
    Gm { p: Ix, n: Ix, cp: Ix, cn: Ix, gm: f64, offset: f64 },
    Buffer { out: Ix, input: Ix, vcc: Ix, vee: Ix, rails: bool, headroom: f64, j: usize },
    Ldo { vin: Ix, g: Ix, out: Ix, vout: f64, dropout: f64, iq: f64, j: usize },
    Vcvs { p: Ix, n: Ix, cp: Ix, cn: Ix, gain: f64, j: usize },
    Src { p: Ix, n: Ix, j: Option<usize>, drive: usize },
}

struct NoiseSrc {
    a: Ix,
    b: Ix,
    psd: Psd,
}

enum Psd {
    Thermal(f64),
    Shot(usize, u8),
    Channel(usize),
    Fixed(f64),
}

pub struct Net {
    size: usize,
    nodes: usize,
    units: Vec<Unit>,
    owner: Vec<Option<usize>>,
    noise: Vec<NoiseSrc>,
    mems: usize,
    vt: f64,
    integrators: Vec<Integrator>,
}

struct Integrator {
    node: usize,
    vcc: Ix,
    vee: Ix,
    headroom: f64,
}

impl Integrator {
    fn limit(&self, new: f64, old: f64, x: &[f64]) -> f64 {
        let (hi, lo) = (v(x, self.vcc), v(x, self.vee));
        let half = (hi - lo) / 2.0 - self.headroom;
        if half <= 1e-3 {
            return old + (new - old).clamp(-10.0, 10.0);
        }
        let mid = (hi + lo) / 2.0;
        let (uo, un) = ((old - mid) / half, (new - mid) / half);
        let u = if uo.abs() <= WINDUP_SPAN {
            uo + (un - uo).clamp(-1.0, 1.0)
        } else if un.signum() != uo.signum() || un.abs() < WINDUP_SPAN {
            WINDUP_SPAN * uo.signum()
        } else {
            un
        };
        mid + u * half
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Dc { scale: f64, gmin: f64 },
    Tran { t: f64, h: f64, trap: bool },
}

#[derive(Clone, Default)]
struct State {
    cap: Vec<(f64, f64)>,
    ind: Vec<(f64, f64)>,
}

struct Sys {
    a: BTreeMap<(usize, usize), f64>,
    b: Vec<f64>,
    limited: bool,
}

impl Sys {
    fn new(m: usize) -> Sys {
        Sys { a: BTreeMap::new(), b: vec![0.0; m], limited: false }
    }

    fn add(&mut self, r: Ix, c: Ix, v: f64) {
        if let (Some(r), Some(c)) = (r, c) {
            *self.a.entry((r, c)).or_insert(0.0) += v;
        }
    }

    fn rhs(&mut self, r: Ix, v: f64) {
        if let Some(r) = r {
            self.b[r] += v;
        }
    }

    fn cond(&mut self, a: Ix, b: Ix, g: f64) {
        self.add(a, a, g);
        self.add(b, b, g);
        self.add(a, b, -g);
        self.add(b, a, -g);
    }

    fn cur(&mut self, a: Ix, b: Ix, i: f64) {
        self.rhs(a, -i);
        self.rhs(b, i);
    }

    fn branch(&mut self, p: Ix, n: Ix, j: usize) {
        self.add(p, Some(j), 1.0);
        self.add(n, Some(j), -1.0);
        self.add(Some(j), p, 1.0);
        self.add(Some(j), n, -1.0);
    }

    fn ctrl(&mut self, k: Ix, constant: f64, terms: &[(Ix, Ix, f64)]) {
        self.rhs(k, -constant);
        for &(p, n, c) in terms {
            self.add(k, p, c);
            self.add(k, n, -c);
        }
    }
}

fn v(x: &[f64], i: Ix) -> f64 {
    i.map_or(0.0, |i| x[i])
}

fn pnjlim(new: f64, old: f64, nvt: f64, vcrit: f64) -> f64 {
    if new > vcrit && (new - old).abs() > 2.0 * nvt {
        if old > 0.0 {
            let arg = 1.0 + (new - old) / nvt;
            if arg > 0.0 { old + nvt * arg.ln() } else { vcrit }
        } else {
            nvt * (new / nvt).max(1e-30).ln().max(0.0)
        }
    } else {
        new
    }
}

fn vcrit(is: f64, nvt: f64) -> f64 {
    nvt * (nvt / (std::f64::consts::SQRT_2 * is)).ln()
}

fn expo(x: f64) -> f64 {
    if x > 80.0 { 80f64.exp() * (1.0 + x - 80.0) } else { x.exp() }
}

fn dexpo(x: f64) -> f64 {
    if x > 80.0 { 80f64.exp() } else { x.exp() }
}

fn diode(vd: f64, is: f64, n: f64, bv: Option<f64>, vt: f64) -> (f64, f64) {
    let nvt = n * vt;
    let mut i = is * (expo(vd / nvt) - 1.0);
    let mut g = is * dexpo(vd / nvt) / nvt;
    if let Some(bv) = bv {
        let x = -(vd + bv) / vt;
        i -= ZENER_KNEE * expo(x);
        g += ZENER_KNEE * dexpo(x) / vt;
    }
    (i + GMIN * vd, g + GMIN)
}

fn mos(vgs: f64, vds: f64, vth: f64, k: f64, lambda: f64, vt: f64) -> (f64, f64, f64) {
    let nvt = 2.0 * 1.5 * vt;
    let u = (vgs - vth) / nvt;
    let (vov, dvov) =
        if u > 40.0 { (vgs - vth, 1.0) } else { (nvt * u.exp().ln_1p(), 1.0 / (1.0 + (-u).exp())) };
    let cl = 1.0 + lambda * vds;
    if vds >= vov {
        let id = k / 2.0 * vov * vov * cl;
        (id, k * vov * cl * dvov, k / 2.0 * vov * vov * lambda)
    } else {
        let core = vov * vds - vds * vds / 2.0;
        (k * core * cl, k * vds * cl * dvov, k * (vov - vds) * cl + k * core * lambda)
    }
}

const WINDUP_SPAN: f64 = 3.0;
const WINDUP_G: f64 = 1.0;
const WINDUP_KNEE: f64 = 0.1;

fn hinge(e: f64) -> (f64, f64) {
    if e <= 0.0 {
        (0.0, 0.0)
    } else if e < WINDUP_KNEE {
        (e * e / (2.0 * WINDUP_KNEE), e / WINDUP_KNEE)
    } else {
        (e - WINDUP_KNEE / 2.0, 1.0)
    }
}

fn soft_min(a: f64, b: f64) -> (f64, f64) {
    let r = ((a - b).powi(2) + EPS_SOFT * EPS_SOFT).sqrt();
    ((a + b - r) / 2.0, (1.0 - (b - a) / r) / 2.0)
}

fn soft_pos(a: f64) -> (f64, f64) {
    let r = (a * a + EPS_SOFT * EPS_SOFT).sqrt();
    ((a + r) / 2.0, (1.0 + a / r) / 2.0)
}

impl Net {
    pub fn build(spec: &CircuitSpec) -> Net {
        let mut size = spec.nets.len();
        let node = |size: &mut usize| {
            let k = *size;
            *size += 1;
            Some(k)
        };
        let mut units = Vec::new();
        let mut owner = Vec::new();
        let mut noise = Vec::new();
        let mut mems = 0;
        let mut integrators = Vec::new();
        let mut pending: Vec<(Option<usize>, Unit)> = Vec::new();
        for (ei, e) in spec.elements.iter().enumerate() {
            let n = &e.nodes;
            let mut push = |u: Unit, own: bool| pending.push((own.then_some(ei), u));
            match e.kind {
                ElemKind::Resistor { r } => {
                    push(Unit::R { a: n[0], b: n[1], g: 1.0 / r }, true);
                    noise.push(NoiseSrc { a: n[0], b: n[1], psd: Psd::Thermal(r) });
                }
                ElemKind::Capacitor { c } => push(Unit::C { a: n[0], b: n[1], c }, true),
                ElemKind::Inductor { l } => push(Unit::L { a: n[0], b: n[1], l, j: 0 }, true),
                ElemKind::Diode { is, n: ni, bv, cj } => {
                    push(Unit::D { a: n[0], k: n[1], is, n: ni, bv, mem: mems }, true);
                    noise.push(NoiseSrc { a: n[0], b: n[1], psd: Psd::Shot(ei, 0) });
                    mems += 1;
                    if cj > 0.0 {
                        push(Unit::C { a: n[0], b: n[1], c: cj }, false);
                    }
                }
                ElemKind::Bjt { pnp, is, bf, br, va, cje, cjc } => {
                    let (c, b, em) = (n[0], n[1], n[2]);
                    push(Unit::Q { c, b, e: em, pnp, is, bf, br, va, mem: mems }, true);
                    noise.push(NoiseSrc { a: c, b: em, psd: Psd::Shot(ei, 0) });
                    noise.push(NoiseSrc { a: b, b: em, psd: Psd::Shot(ei, 1) });
                    mems += 2;
                    if cje > 0.0 {
                        push(Unit::C { a: b, b: em, c: cje }, false);
                    }
                    if cjc > 0.0 {
                        push(Unit::C { a: b, b: c, c: cjc }, false);
                    }
                }
                ElemKind::Mosfet { p, vth, k, lambda, cgs, cgd } => {
                    let (d, g, s) = (n[0], n[1], n[2]);
                    push(Unit::M { d, g, s, p, vth, k, lambda, mem: mems }, true);
                    noise.push(NoiseSrc { a: d, b: s, psd: Psd::Channel(ei) });
                    mems += 2;
                    if cgs > 0.0 {
                        push(Unit::C { a: g, b: s, c: cgs }, false);
                    }
                    if cgd > 0.0 {
                        push(Unit::C { a: g, b: d, c: cgd }, false);
                    }
                }
                ElemKind::Opamp { gain, gbw, rout, vos, en, headroom } => {
                    let n1 = node(&mut size);
                    let n2 = node(&mut size);
                    if let (Some(node), true) = (n1, n.len() >= 5) {
                        integrators.push(Integrator { node, vcc: n[3], vee: n[4], headroom });
                    }
                    push(
                        Unit::Gm { p: None, n: n1, cp: n[0], cn: n[1], gm: 1.0, offset: vos },
                        true,
                    );
                    push(Unit::R { a: n1, b: None, g: 1.0 / gain }, false);
                    push(Unit::C { a: n1, b: None, c: 1.0 / (std::f64::consts::TAU * gbw) }, false);
                    let rails = n.len() >= 5;
                    let (vcc, vee) = if rails { (n[3], n[4]) } else { (None, None) };
                    push(
                        Unit::Buffer { out: n2, input: n1, vcc, vee, rails, headroom, j: 0 },
                        false,
                    );
                    push(Unit::R { a: n2, b: n[2], g: 1.0 / rout.max(1e-6) }, false);
                    if en > 0.0 {
                        noise.push(NoiseSrc { a: None, b: n1, psd: Psd::Fixed(en * en) });
                    }
                }
                ElemKind::Ldo { vout, dropout, rout, iq } => {
                    let n2 = node(&mut size);
                    push(Unit::Ldo { vin: n[0], g: n[2], out: n2, vout, dropout, iq, j: 0 }, true);
                    push(Unit::R { a: n2, b: n[1], g: 1.0 / rout.max(1e-6) }, false);
                }
                ElemKind::Vcvs { gain } => {
                    push(Unit::Vcvs { p: n[0], n: n[1], cp: n[2], cn: n[3], gain, j: 0 }, true)
                }
                ElemKind::Vccs { gm } => {
                    push(Unit::Gm { p: n[0], n: n[1], cp: n[2], cn: n[3], gm, offset: 0.0 }, true)
                }
            }
        }
        for (di, d) in spec.drives.iter().enumerate() {
            let p = match (d.r > 0.0, d.current) {
                (false, _) => d.p,
                (true, true) => {
                    pending.push((None, Unit::R { a: d.p, b: d.n, g: 1.0 / d.r }));
                    d.p
                }
                (true, false) => {
                    let s = node(&mut size);
                    pending.push((None, Unit::R { a: s, b: d.p, g: 1.0 / d.r }));
                    s
                }
            };
            if d.r > 0.0 {
                noise.push(NoiseSrc { a: d.p, b: d.n, psd: Psd::Thermal(d.r) });
            }
            pending.push((None, Unit::Src { p, n: d.n, j: None, drive: di }));
        }
        let nodes = size;
        for (own, mut u) in pending {
            let mut take = || {
                let j = size;
                size += 1;
                j
            };
            match &mut u {
                Unit::L { j, .. }
                | Unit::Buffer { j, .. }
                | Unit::Ldo { j, .. }
                | Unit::Vcvs { j, .. } => *j = take(),
                Unit::Src { j, drive, .. } if !spec.drives[*drive].current => *j = Some(take()),
                _ => {}
            }
            units.push(u);
            owner.push(own);
        }
        let vt = BOLTZMANN * (spec.temperature + 273.15) / CHARGE;
        Net { size, nodes, units, owner, noise, mems, vt, integrators }
    }

    fn seed(&self, x: &[f64]) -> Vec<f64> {
        let mut mem = vec![0.0; self.mems];
        for u in &self.units {
            match *u {
                Unit::D { a, k, mem: m, .. } => mem[m] = v(x, a) - v(x, k),
                Unit::Q { c, b, e, pnp, mem: m, .. } => {
                    let sg = if pnp { -1.0 } else { 1.0 };
                    mem[m] = sg * (v(x, b) - v(x, e));
                    mem[m + 1] = sg * (v(x, b) - v(x, c));
                }
                Unit::M { d, g, s, p, mem: m, .. } => {
                    let sg = if p { -1.0 } else { 1.0 };
                    mem[m] = sg * (v(x, g) - v(x, s));
                    mem[m + 1] = sg * (v(x, d) - v(x, s));
                }
                _ => {}
            }
        }
        mem
    }

    fn drive_value(spec: &CircuitSpec, k: usize, mode: Mode) -> f64 {
        let d = &spec.drives[k];
        match mode {
            Mode::Dc { scale, .. } => d.dc * scale,
            Mode::Tran { t, .. } => match d.wave {
                Wave::Dc => d.dc,
                ref w => w.at(d.dc, t),
            },
        }
    }

    fn assemble(
        &self,
        spec: &CircuitSpec,
        x: &[f64],
        mode: Mode,
        st: &State,
        mem: &mut [f64],
    ) -> Sys {
        let mut s = Sys::new(self.size);
        let vt = self.vt;
        let gmin = match mode {
            Mode::Dc { gmin, .. } => gmin,
            Mode::Tran { .. } => GMIN,
        };
        for k in 0..self.nodes {
            s.add(Some(k), Some(k), gmin);
        }
        let (mut ci, mut li) = (0, 0);
        for u in &self.units {
            match *u {
                Unit::R { a, b, g } => s.cond(a, b, g),
                Unit::C { a, b, c } => {
                    if let Mode::Tran { h, trap, .. } = mode {
                        let (vp, ip) = st.cap[ci];
                        let geq = if trap { 2.0 * c / h } else { c / h };
                        let ieq = if trap { -(geq * vp + ip) } else { -geq * vp };
                        s.cond(a, b, geq);
                        s.cur(a, b, ieq);
                    }
                    ci += 1;
                }
                Unit::L { a, b, l, j } => {
                    s.branch(a, b, j);
                    if let Mode::Tran { h, trap, .. } = mode {
                        let (ip, vp) = st.ind[li];
                        let z = if trap { 2.0 * l / h } else { l / h };
                        s.add(Some(j), Some(j), -z);
                        s.rhs(Some(j), if trap { -z * ip - vp } else { -z * ip });
                    }
                    li += 1;
                }
                Unit::D { a, k, is, n, bv, mem: m } => {
                    let nvt = n * vt;
                    let raw = v(x, a) - v(x, k);
                    let vd = pnjlim(raw, mem[m], nvt, vcrit(is, nvt));
                    s.limited |= (vd - raw).abs() > 1e-9;
                    mem[m] = vd;
                    let (i, g) = diode(vd, is, n, bv, vt);
                    s.cond(a, k, g);
                    s.cur(a, k, i - g * vd);
                }
                Unit::Q { c, b, e, pnp, is, bf, br, va, mem: m } => {
                    let sg = if pnp { -1.0 } else { 1.0 };
                    let vc = vcrit(is, vt);
                    let (rbe, rbc) = (sg * (v(x, b) - v(x, e)), sg * (v(x, b) - v(x, c)));
                    let vbe = pnjlim(rbe, mem[m], vt, vc);
                    let vbc = pnjlim(rbc, mem[m + 1], vt, vc);
                    s.limited |= (vbe - rbe).abs() > 1e-9 || (vbc - rbc).abs() > 1e-9;
                    mem[m] = vbe;
                    mem[m + 1] = vbc;
                    let (fi, ri) = (is * (expo(vbe / vt) - 1.0), is * (expo(vbc / vt) - 1.0));
                    let (dfi, dri) = (is * dexpo(vbe / vt) / vt, is * dexpo(vbc / vt) / vt);
                    let (early, de) = match va {
                        Some(va) if va > 0.0 => ((1.0 + (vbe - vbc) / va).max(0.05), 1.0 / va),
                        _ => (1.0, 0.0),
                    };
                    let it = (fi - ri) * early;
                    let ic = it - ri / br;
                    let ib = fi / bf + ri / br;
                    let dic_be = dfi * early + (fi - ri) * de + GMIN;
                    let dic_bc = -dri * early - (fi - ri) * de - dri / br - GMIN;
                    let dib_be = dfi / bf + GMIN;
                    let dib_bc = dri / br + GMIN;
                    let ic0 = ic + GMIN * (vbe - vbc);
                    let ib0 = ib + GMIN * (vbe + vbc);
                    let lin = |f0: f64, d1: f64, d2: f64| f0 - d1 * vbe - d2 * vbc;
                    let cterms = [(b, e, dic_be), (b, c, dic_bc)];
                    let bterms = [(b, e, dib_be), (b, c, dib_bc)];
                    let eterms = [(b, e, -(dic_be + dib_be)), (b, c, -(dic_bc + dib_bc))];
                    s.ctrl(c, sg * lin(ic0, dic_be, dic_bc), &cterms);
                    s.ctrl(b, sg * lin(ib0, dib_be, dib_bc), &bterms);
                    s.ctrl(e, -sg * (lin(ic0, dic_be, dic_bc) + lin(ib0, dib_be, dib_bc)), &eterms);
                }
                Unit::M { d, g, s: so, p, vth, k, lambda, mem: m } => {
                    let sg = if p { -1.0 } else { 1.0 };
                    let (rgs, rds) = (sg * (v(x, g) - v(x, so)), sg * (v(x, d) - v(x, so)));
                    let vgs = mem[m] + (rgs - mem[m]).clamp(-2.0, 2.0);
                    let vds = mem[m + 1] + (rds - mem[m + 1]).clamp(-5.0, 5.0);
                    s.limited |= (vgs - rgs).abs() > 1e-9 || (vds - rds).abs() > 1e-9;
                    mem[m] = vgs;
                    mem[m + 1] = vds;
                    let (dd, ss, vgx, vdx) =
                        if vds >= 0.0 { (d, so, vgs, vds) } else { (so, d, vgs - vds, -vds) };
                    let (id, gm, gds) = mos(vgx, vdx, vth, k, lambda, vt);
                    let gds = gds + GMIN;
                    let id = id + GMIN * vdx;
                    let c0 = sg * (id - gm * vgx - gds * vdx);
                    s.ctrl(dd, c0, &[(g, ss, gm), (dd, ss, gds)]);
                    s.ctrl(ss, -c0, &[(g, ss, -gm), (dd, ss, -gds)]);
                }
                Unit::Gm { p, n, cp, cn, gm, offset } => {
                    s.ctrl(p, gm * offset, &[(cp, cn, gm)]);
                    s.ctrl(n, -gm * offset, &[(cp, cn, -gm)]);
                }
                Unit::Buffer { out, input, vcc, vee, rails, headroom, j } => {
                    s.add(out, Some(j), 1.0);
                    s.add(Some(j), out, 1.0);
                    let vi = v(x, input);
                    if !rails {
                        s.add(Some(j), input, -1.0);
                        continue;
                    }
                    let (hi, lo) = (v(x, vcc), v(x, vee));
                    let mid = (hi + lo) / 2.0;
                    let half = (hi - lo) / 2.0 - headroom;
                    let (f, d1, dc, de) = if half <= 1e-3 {
                        (mid, 0.0, 0.5, 0.5)
                    } else {
                        let u = (vi - mid) / half;
                        let t = u.tanh();
                        let dh = t - u * (1.0 - t * t);
                        (
                            mid + half * t,
                            1.0 - t * t,
                            t * t / 2.0 + dh / 2.0,
                            t * t / 2.0 - dh / 2.0,
                        )
                    };
                    s.add(Some(j), input, -d1);
                    s.add(Some(j), vcc, -dc);
                    s.add(Some(j), vee, -de);
                    s.rhs(Some(j), f - d1 * vi - dc * hi - de * lo);
                    if half > 1e-3 {
                        let top = mid + WINDUP_SPAN * half;
                        let bot = mid - WINDUP_SPAN * half;
                        let (f1, g1) = hinge(vi - top);
                        let (f2, g2) = hinge(bot - vi);
                        let k = (WINDUP_SPAN + 1.0) / 2.0;
                        let m = (WINDUP_SPAN - 1.0) / 2.0;
                        let di = WINDUP_G * (g1 + g2);
                        let dhi = WINDUP_G * (m * g2 - k * g1);
                        let dlo = WINDUP_G * (m * g1 - k * g2);
                        let i0 = WINDUP_G * (f1 - f2);
                        s.ctrl(
                            input,
                            i0 - di * vi - dhi * hi - dlo * lo,
                            &[(input, None, di), (vcc, None, dhi), (vee, None, dlo)],
                        );
                    }
                }
                Unit::Ldo { vin, g, out, vout, dropout, iq, j } => {
                    let vscale = match mode {
                        Mode::Dc { scale, .. } => scale,
                        _ => 1.0,
                    };
                    s.branch(out, g, j);
                    s.add(vin, Some(j), -1.0);
                    s.add(g, Some(j), 1.0);
                    let (vi, vg) = (v(x, vin), v(x, g));
                    let (s1, d1) = soft_min(vout * vscale, vi - vg - dropout);
                    let (s2, d2) = soft_pos(s1);
                    let k = d2 * d1;
                    s.add(Some(j), vin, -k);
                    s.add(Some(j), g, k);
                    s.rhs(Some(j), s2 - k * (vi - vg));
                    s.cur(vin, g, iq);
                }
                Unit::Vcvs { p, n, cp, cn, gain, j } => {
                    s.branch(p, n, j);
                    s.add(Some(j), cp, -gain);
                    s.add(Some(j), cn, gain);
                }
                Unit::Src { p, n, j, drive } => {
                    let val = Self::drive_value(spec, drive, mode);
                    match j {
                        Some(j) => {
                            s.branch(p, n, j);
                            s.rhs(Some(j), val);
                        }
                        None => s.cur(n, p, val),
                    }
                }
            }
        }
        s
    }

    fn solve(&self, s: &Sys) -> Option<Vec<f64>> {
        let trip: Vec<Triplet<usize, usize, f64>> =
            s.a.iter().map(|(&(r, c), &v)| Triplet::new(r, c, v)).collect();
        let a =
            SparseColMat::<usize, f64>::try_new_from_triplets(self.size, self.size, &trip).ok()?;
        let lu = a.sp_lu().ok()?;
        let x = lu.solve(faer::Mat::<f64>::from_fn(self.size, 1, |i, _| s.b[i]));
        let out: Vec<f64> = (0..self.size).map(|i| x[(i, 0)]).collect();
        out.iter().all(|v| v.is_finite()).then_some(out)
    }

    fn newton(
        &self,
        spec: &CircuitSpec,
        x0: &[f64],
        mode: Mode,
        st: &State,
        limit: usize,
    ) -> Option<Vec<f64>> {
        let mut x = x0.to_vec();
        let mut mem = self.seed(x0);
        let mem = &mut mem;
        for it in 0..limit {
            let sys = self.assemble(spec, &x, mode, st, mem);
            let mut new = self.solve(&sys)?;
            let mut worst = 0.0f64;
            for (i, (n, o)) in new.iter().zip(&x).enumerate() {
                let tol = if i < self.nodes {
                    1e-6 + 1e-4 * n.abs().max(o.abs())
                } else {
                    1e-9 + 1e-4 * n.abs().max(o.abs())
                };
                worst = worst.max((n - o).abs() / tol);
            }
            let jump = new
                .iter()
                .zip(&x)
                .take(self.nodes)
                .enumerate()
                .filter(|(i, _)| !self.integrators.iter().any(|g| g.node == *i))
                .map(|(_, (n, o))| (n - o).abs())
                .fold(0.0, f64::max);
            if jump > 10.0 {
                let k = 10.0 / jump;
                for (n, o) in new.iter_mut().zip(&x) {
                    *n = o + (*n - o) * k;
                }
            }
            for g in &self.integrators {
                new[g.node] = g.limit(new[g.node], x[g.node], &new);
            }
            x = new;
            if worst <= 1.0 && it > 0 && !sys.limited {
                return Some(x);
            }
        }
        None
    }

    fn op(&self, spec: &CircuitSpec) -> Option<Vec<f64>> {
        let zero = vec![0.0; self.size];
        let st = State::default();
        if let Some(x) = self.newton(spec, &zero, Mode::Dc { scale: 1.0, gmin: GMIN }, &st, 150) {
            return Some(x);
        }
        let mut x = zero.clone();
        let mut ok = true;
        for e in (2..=12).map(|k| 10f64.powi(-k)) {
            match self.newton(spec, &x, Mode::Dc { scale: 1.0, gmin: e }, &st, 150) {
                Some(y) => x = y,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some(x);
        }
        let mut x = zero;
        let mut scale: f64 = 0.0;
        let mut step: f64 = 0.1;
        while scale < 1.0 {
            let next = (scale + step).min(1.0);
            match self.newton(spec, &x, Mode::Dc { scale: next, gmin: GMIN }, &st, 100) {
                Some(y) => {
                    x = y;
                    scale = next;
                    step = (step * 1.5).min(0.25);
                }
                None => {
                    step /= 4.0;
                    if step < 1e-4 {
                        return None;
                    }
                }
            }
        }
        Some(x)
    }

    fn branch_values(&self, x: &[f64]) -> State {
        let mut st = State::default();
        for u in &self.units {
            match *u {
                Unit::C { a, b, .. } => st.cap.push((v(x, a) - v(x, b), 0.0)),
                Unit::L { j, .. } => st.ind.push((x[j], 0.0)),
                _ => {}
            }
        }
        st
    }

    fn advance(&self, x: &[f64], st: &State, h: f64, trap: bool) -> State {
        let mut out = State::default();
        let (mut ci, mut li) = (0, 0);
        for u in &self.units {
            match *u {
                Unit::C { a, b, c } => {
                    let (vp, ip) = st.cap[ci];
                    let vn = v(x, a) - v(x, b);
                    let i = if trap { 2.0 * c / h * (vn - vp) - ip } else { c / h * (vn - vp) };
                    out.cap.push((vn, i));
                    ci += 1;
                }
                Unit::L { a, b, j, .. } => {
                    let _ = st.ind[li];
                    out.ind.push((x[j], v(x, a) - v(x, b)));
                    li += 1;
                }
                _ => {}
            }
        }
        out
    }

    fn element_current(
        &self,
        spec: &CircuitSpec,
        e: usize,
        x: &[f64],
        st: Option<&State>,
    ) -> Option<f64> {
        let (mut ci, mut _li) = (0, 0);
        for (k, u) in self.units.iter().enumerate() {
            let mine = self.owner[k] == Some(e);
            match *u {
                Unit::R { a, b, g } if mine => return Some((v(x, a) - v(x, b)) * g),
                Unit::C { .. } => {
                    if mine {
                        return Some(st.map_or(0.0, |s| s.cap[ci].1));
                    }
                    ci += 1;
                }
                Unit::L { j, .. } => {
                    if mine {
                        return Some(x[j]);
                    }
                    _li += 1;
                }
                Unit::D { a, k, is, n, bv, .. } if mine => {
                    return Some(diode(v(x, a) - v(x, k), is, n, bv, self.vt).0);
                }
                _ => {}
            }
        }
        let _ = spec;
        None
    }

    fn bjt_currents(&self, u: &Unit, x: &[f64]) -> (f64, f64) {
        let Unit::Q { c, b, e, pnp, is, bf, br, va, .. } = *u else { return (0.0, 0.0) };
        let sg = if pnp { -1.0 } else { 1.0 };
        let vt = self.vt;
        let vbe = sg * (v(x, b) - v(x, e));
        let vbc = sg * (v(x, b) - v(x, c));
        let (fi, ri) = (is * (expo(vbe / vt) - 1.0), is * (expo(vbc / vt) - 1.0));
        let early = va.filter(|&va| va > 0.0).map_or(1.0, |va| (1.0 + (vbe - vbc) / va).max(0.05));
        ((fi - ri) * early - ri / br, fi / bf + ri / br)
    }

    fn mos_gm(&self, u: &Unit, x: &[f64]) -> f64 {
        let Unit::M { d, g, s, p, vth, k, lambda, .. } = *u else { return 0.0 };
        let sg = if p { -1.0 } else { 1.0 };
        let vgs = sg * (v(x, g) - v(x, s));
        let vds = sg * (v(x, d) - v(x, s));
        let (vgx, vdx) = if vds >= 0.0 { (vgs, vds) } else { (vgs - vds, -vds) };
        mos(vgx, vdx, vth, k, lambda, self.vt).1
    }

    fn ac_matrix(&self, spec: &CircuitSpec, op: &[f64], w: f64) -> BTreeMap<(usize, usize), c64> {
        let st = State::default();
        let mut mem = self.seed(op);
        let warm = self.assemble(spec, op, Mode::Dc { scale: 1.0, gmin: GMIN }, &st, &mut mem);
        let mut a: BTreeMap<(usize, usize), c64> =
            warm.a.iter().map(|(&k, &v)| (k, c64::new(v, 0.0))).collect();
        let mut put = |r: Ix, c: Ix, im: f64| {
            if let (Some(r), Some(c)) = (r, c) {
                a.entry((r, c)).or_insert(c64::new(0.0, 0.0)).im += im;
            }
        };
        for u in &self.units {
            match *u {
                Unit::C { a: p, b: n, c } => {
                    put(p, p, w * c);
                    put(n, n, w * c);
                    put(p, n, -w * c);
                    put(n, p, -w * c);
                }
                Unit::L { l, j, .. } => put(Some(j), Some(j), -w * l),
                _ => {}
            }
        }
        a
    }

    fn csolve(
        &self,
        a: &BTreeMap<(usize, usize), c64>,
        b: &[c64],
        transpose: bool,
    ) -> Option<Vec<c64>> {
        let trip: Vec<Triplet<usize, usize, c64>> = a
            .iter()
            .map(
                |(&(r, c), &v)| {
                    if transpose { Triplet::new(c, r, v) } else { Triplet::new(r, c, v) }
                },
            )
            .collect();
        let m =
            SparseColMat::<usize, c64>::try_new_from_triplets(self.size, self.size, &trip).ok()?;
        let lu = m.sp_lu().ok()?;
        let x = lu.solve(faer::Mat::<c64>::from_fn(self.size, 1, |i, _| b[i]));
        Some((0..self.size).map(|i| x[(i, 0)]).collect())
    }

    fn ac_rhs(&self, spec: &CircuitSpec, only: Option<usize>) -> Vec<c64> {
        let mut b = vec![c64::new(0.0, 0.0); self.size];
        for u in &self.units {
            if let Unit::Src { p, n, j, drive } = *u {
                if only.is_some_and(|o| o != drive) {
                    continue;
                }
                let amp = if only.is_some() { 1.0 } else { spec.drives[drive].ac };
                match j {
                    Some(j) => b[j].re += amp,
                    None => {
                        if let Some(p) = p {
                            b[p].re += amp;
                        }
                        if let Some(n) = n {
                            b[n].re -= amp;
                        }
                    }
                }
            }
        }
        b
    }
}

fn mag(c: c64) -> f64 {
    (c.re * c.re + c.im * c.im).sqrt()
}

fn at_node(x: &[c64], n: Node) -> c64 {
    n.map_or(c64::new(0.0, 0.0), |i| x[i])
}

fn interp_log(fs: &[f64], ys: &[f64], f: f64) -> Option<f64> {
    if fs.is_empty() || f < fs[0] * (1.0 - 1e-9) || f > fs[fs.len() - 1] * (1.0 + 1e-9) {
        return None;
    }
    let k = fs.partition_point(|&x| x < f).min(fs.len() - 1);
    if k == 0 {
        return Some(ys[0]);
    }
    let (f0, f1) = (fs[k - 1].ln(), fs[k].ln());
    let t = if f1 > f0 { (f.ln() - f0) / (f1 - f0) } else { 0.0 };
    Some(ys[k - 1] + (ys[k] - ys[k - 1]) * t)
}

fn interp_lin(ts: &[f64], ys: &[f64], t: f64) -> Option<f64> {
    if ts.is_empty() || t < ts[0] - 1e-15 || t > ts[ts.len() - 1] * (1.0 + 1e-9) + 1e-15 {
        return None;
    }
    let k = ts.partition_point(|&x| x < t).min(ts.len() - 1);
    if k == 0 {
        return Some(ys[0]);
    }
    let span = ts[k] - ts[k - 1];
    let u = if span > 0.0 { (t - ts[k - 1]) / span } else { 0.0 };
    Some(ys[k - 1] + (ys[k] - ys[k - 1]) * u)
}

pub fn run(spec: &CircuitSpec, name: &str, hash: u64) -> CircuitResult {
    let t0 = std::time::Instant::now();
    let net = Net::build(spec);
    let mut res = CircuitResult { name: name.to_string(), spec_hash: hash, ..Default::default() };
    let Some(op) = net.op(spec) else {
        res.failures.push("the operating point did not converge".into());
        res.seconds = t0.elapsed().as_secs_f64();
        return res;
    };
    let volts = |x: &[f64], n: Node| v(x, n);
    for (i, n) in spec.nets.iter().enumerate() {
        res.op.insert(n.clone(), op[i]);
    }
    for (ei, e) in spec.elements.iter().enumerate() {
        if e.nodes.len() == 2
            && let Some(i) = net.element_current(spec, ei, &op, None)
        {
            res.currents.insert(e.name.clone(), i);
        }
    }
    for u in &net.units {
        if let Unit::Src { j: Some(j), drive, .. } = *u {
            res.currents.insert(format!("drive {}", spec.drives[drive].net), -op[j]);
        }
    }

    let mut ac_db: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    let mut ac_ph: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    let inputs: Vec<usize> = (0..spec.drives.len()).filter(|&k| spec.drives[k].ac != 0.0).collect();
    let reference = if inputs.len() == 1 { spec.drives[inputs[0]].ac.abs() } else { 1.0 };
    if let Some(sw) = spec.ac {
        let freqs = sw.freqs();
        let rhs = net.ac_rhs(spec, None);
        for &f in &freqs {
            let a = net.ac_matrix(spec, &op, std::f64::consts::TAU * f);
            let Some(x) = net.csolve(&a, &rhs, false) else {
                res.failures.push(format!("the AC solve failed at {}", fmt_si(f, "Hz")));
                break;
            };
            for (k, &c) in x.iter().enumerate().take(spec.nets.len()) {
                ac_db.entry(k).or_default().push(20.0 * (mag(c) / reference).max(1e-30).log10());
                ac_ph.entry(k).or_default().push(c.im.atan2(c.re).to_degrees());
            }
        }
        res.freqs = freqs;
        for &k in &spec.record {
            if let (Some(g), Some(p)) = (ac_db.get(&k), ac_ph.get(&k)) {
                res.gain_db.push(Curve { name: spec.nets[k].clone(), values: g.clone() });
                res.phase_deg.push(Curve { name: spec.nets[k].clone(), values: p.clone() });
            }
        }
    }

    let mut noise_out: Vec<f64> = Vec::new();
    if let Some(nz) = &spec.noise {
        let freqs = nz.sweep.freqs();
        let kt4 = 4.0 * BOLTZMANN * (spec.temperature + 273.15);
        let psd = |p: &Psd| -> f64 {
            match *p {
                Psd::Thermal(r) => kt4 / r,
                Psd::Fixed(s) => s,
                Psd::Shot(e, which) => {
                    let unit = net
                        .units
                        .iter()
                        .enumerate()
                        .find(|(k, _)| net.owner[*k] == Some(e))
                        .map(|(_, u)| u);
                    let i = match unit {
                        Some(u @ Unit::Q { .. }) => {
                            let (ic, ib) = net.bjt_currents(u, &op);
                            if which == 0 { ic } else { ib }
                        }
                        _ => net.element_current(spec, e, &op, None).unwrap_or(0.0),
                    };
                    2.0 * CHARGE * i.abs()
                }
                Psd::Channel(e) => {
                    let gm = net
                        .units
                        .iter()
                        .enumerate()
                        .find(|(k, _)| net.owner[*k] == Some(e))
                        .map_or(0.0, |(_, u)| net.mos_gm(u, &op));
                    kt4 * 2.0 / 3.0 * gm.abs()
                }
            }
        };
        let s_of: Vec<f64> = net.noise.iter().map(|n| psd(&n.psd)).collect();
        let mut input_curve = Vec::new();
        for &f in &freqs {
            let a = net.ac_matrix(spec, &op, std::f64::consts::TAU * f);
            let mut e = vec![c64::new(0.0, 0.0); net.size];
            if let Some(o) = nz.output {
                e[o].re = 1.0;
            }
            let Some(y) = net.csolve(&a, &e, true) else {
                res.failures.push(format!("the noise solve failed at {}", fmt_si(f, "Hz")));
                break;
            };
            let total: f64 = net
                .noise
                .iter()
                .zip(&s_of)
                .map(|(src, s)| {
                    let h = at_node(&y, src.a) - at_node(&y, src.b);
                    (h.re * h.re + h.im * h.im) * s
                })
                .sum();
            noise_out.push(total.sqrt());
            if let Some(k) = nz.input {
                let rhs = net.ac_rhs(spec, Some(k));
                let gain = net.csolve(&a, &rhs, false).map_or(0.0, |x| mag(at_node(&x, nz.output)));
                input_curve.push(if gain > 0.0 { total.sqrt() / gain } else { f64::NAN });
            }
        }
        let rms: f64 = freqs
            .windows(2)
            .zip(noise_out.windows(2))
            .map(|(f, n)| (f[1] - f[0]) * (n[0] * n[0] + n[1] * n[1]) / 2.0)
            .sum::<f64>()
            .sqrt();
        res.noise_rms = Some(rms);
        res.noise
            .push(Curve { name: format!("{} output", nz.output_name), values: noise_out.clone() });
        if !input_curve.is_empty() {
            res.noise.push(Curve { name: "input referred".into(), values: input_curve });
        }
        res.noise_freqs = freqs;
    }

    let mut tran_times: Vec<f64> = Vec::new();
    let mut tran_x: Vec<Vec<f64>> = Vec::new();
    let mut tran_states: Vec<State> = Vec::new();
    if let Some((stop, step)) = spec.tran {
        let mut bps: Vec<f64> =
            spec.drives.iter().flat_map(|d| d.wave.breaks(stop)).filter(|&t| t > 0.0).collect();
        bps.push(stop);
        bps.sort_by(f64::total_cmp);
        bps.dedup_by(|a, b| (*a - *b).abs() < step * 1e-6);
        let mut x =
            match spec.drives.iter().any(|d| d.wave != Wave::Dc && d.wave.at(d.dc, 0.0) != d.dc) {
                false => op.clone(),
                true => {
                    let mut tmp = spec.clone();
                    for d in &mut tmp.drives {
                        d.dc = d.wave.at(d.dc, 0.0);
                    }
                    net.op(&tmp).unwrap_or_else(|| op.clone())
                }
            };
        let mut st = net.branch_values(&x);
        tran_times.push(0.0);
        tran_x.push(x.clone());
        tran_states.push(st.clone());
        let mut t = 0.0;
        let mut h = step / 10.0;
        let mut fresh = true;
        let mut b = 0;
        let mut steps = 0usize;
        while t < stop * (1.0 - 1e-12) {
            while b < bps.len() && bps[b] <= t + step * 1e-9 {
                b += 1;
            }
            let next = bps.get(b).copied().unwrap_or(stop);
            let hh = h.min(next - t).min(step);
            let trap = !fresh;
            match net.newton(spec, &x, Mode::Tran { t: t + hh, h: hh, trap }, &st, 60) {
                Some(y) => {
                    st = net.advance(&y, &st, hh, trap);
                    x = y;
                    t += hh;
                    steps += 1;
                    tran_times.push(t);
                    tran_x.push(x.clone());
                    tran_states.push(st.clone());
                    if (next - t).abs() < step * 1e-9 {
                        fresh = true;
                        h = step / 10.0;
                    } else {
                        fresh = false;
                        h = (hh * 2.0).min(step);
                    }
                }
                None => {
                    h = hh / 4.0;
                    fresh = true;
                    if h < step * 1e-9 {
                        res.failures.push(format!(
                            "the transient stopped at {}: no step converged",
                            fmt_si(t, "s")
                        ));
                        break;
                    }
                }
            }
            if steps > 2_000_000 {
                res.failures.push("the transient took more than 2 million steps".into());
                break;
            }
        }
        res.times = tran_times.clone();
        for &k in &spec.record {
            res.waves.push(Curve {
                name: spec.nets[k].clone(),
                values: tran_x.iter().map(|x| x[k]).collect(),
            });
        }
    }

    for c in &spec.checks {
        let value = match (&c.measure, c.at) {
            (Measure::Volts(n), At::Op) => Some(volts(&op, *n)),
            (Measure::Amps(e), At::Op) => net.element_current(spec, *e, &op, None),
            (Measure::Volts(n), At::Time(tt)) => {
                let ys: Vec<f64> = tran_x.iter().map(|x| v(x, *n)).collect();
                interp_lin(&tran_times, &ys, tt)
            }
            (Measure::Amps(e), At::Time(tt)) => {
                let ys: Vec<f64> = tran_x
                    .iter()
                    .zip(&tran_states)
                    .map(|(x, s)| net.element_current(spec, *e, x, Some(s)).unwrap_or(0.0))
                    .collect();
                interp_lin(&tran_times, &ys, tt)
            }
            (Measure::Volts(n), At::Freq(f)) => match n {
                Some(k) => ac_db.get(k).and_then(|g| interp_log(&res.freqs, g, f)),
                None => Some(-300.0),
            },
            (Measure::Phase(n), At::Freq(f)) => match n {
                Some(k) => ac_ph.get(k).and_then(|p| interp_log(&res.freqs, p, f)),
                None => Some(0.0),
            },
            (Measure::Noise(n), At::Freq(f)) => {
                if spec.noise.as_ref().is_some_and(|nz| nz.output == *n) {
                    interp_log(&res.noise_freqs, &noise_out, f)
                } else {
                    res.failures
                        .push(format!("{}: noise is computed at the [noise] output only", c.label));
                    continue;
                }
            }
            _ => None,
        };
        let unit = match (&c.measure, c.at) {
            (Measure::Volts(_), At::Freq(_)) => "dB",
            (Measure::Volts(_), _) => "V",
            (Measure::Amps(_), _) => "A",
            (Measure::Phase(_), _) => "deg",
            (Measure::Noise(_), _) => "V/rtHz",
        };
        let Some(value) = value else {
            res.failures.push(format!("{}: outside the analysed range", c.label));
            continue;
        };
        let passed = c.min.is_none_or(|m| value >= m) && c.max.is_none_or(|m| value <= m);
        if !passed {
            let range = match (c.min, c.max) {
                (Some(a), Some(b)) => format!("{a} to {b} {unit}"),
                (Some(a), None) => format!("at least {a} {unit}"),
                (None, Some(b)) => format!("at most {b} {unit}"),
                (None, None) => String::new(),
            };
            res.failures.push(format!("{} is {} {unit}, wanted {range}", c.label, short(value)));
        }
        res.checks.push(Reading {
            label: c.label.clone(),
            value,
            unit: unit.into(),
            passed,
            min: c.min,
            max: c.max,
        });
    }
    res.passed = res.failures.is_empty();
    res.seconds = t0.elapsed().as_secs_f64();
    res
}

fn short(v: f64) -> String {
    if v != 0.0 && (v.abs() < 1e-3 || v.abs() >= 1e4) {
        format!("{v:.4e}")
    } else {
        format!("{v:.4}")
    }
}

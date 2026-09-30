use crate::board::Board;
use crate::diag::Diags;
use crate::geom::{self, P};
use crate::layout::{LayoutNet, Pair, Placed, Track, Via, ZoneFill, glob};
use crate::units::{Length, Ohms, Percent, Picos};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub differential: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impedance: Option<Ohms>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impedance_tolerance: Option<Percent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_vias: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stub: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_unreferenced: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_via: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bus_skew: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_window: Option<[String; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measure: Vec<MeasureFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureFile {
    pub sim: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<[String; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<[String; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_loss: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_return_loss: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mode_conversion: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_eye_height: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_eye_width: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Limit {
    Mm(f64),
    Ps(f64),
}

impl Limit {
    pub fn parse(s: &str) -> Result<Limit, String> {
        if let Ok(t) = Picos::parse(s)
            && !s.trim().chars().last().is_some_and(|c| c.is_ascii_digit())
        {
            return Ok(Limit::Ps(t.0));
        }
        Length::parse(s)
            .map(|l| Limit::Mm(l.to_mm()))
            .map_err(|_| format!("`{s}` is neither a length (mm, mil) nor a time (ps, ns)"))
    }

    fn show(self) -> String {
        match self {
            Limit::Mm(v) => format!("{v:.3} mm"),
            Limit::Ps(v) => format!("{v:.2} ps"),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Spec {
    pub differential: bool,
    pub impedance: Option<(f64, f64)>,
    pub max_skew: Option<Limit>,
    pub max_length: Option<f64>,
    pub max_vias: Option<u32>,
    pub max_stub: Option<f64>,
    pub max_unreferenced: Option<f64>,
    pub reference: Vec<String>,
    pub return_via: Option<f64>,
}

const MIL: f64 = 0.0254;

pub fn preset(name: &str) -> Option<Spec> {
    let gnd = vec!["GND".to_string()];
    Some(match name {
        "usb3-gen1" | "usb3-gen2" => Spec {
            differential: true,
            impedance: Some((90.0, 7.0)),
            max_skew: Some(Limit::Mm(5.0 * MIL)),
            max_length: Some(if name == "usb3-gen2" { 3000.0 } else { 3500.0 } * MIL),
            max_vias: Some(2),
            max_stub: Some(15.0 * MIL),
            max_unreferenced: Some(0.5),
            reference: gnd,
            return_via: Some(200.0 * MIL),
        },
        "usb2-hs" => Spec {
            differential: true,
            impedance: Some((90.0, 10.0)),
            max_skew: Some(Limit::Mm(50.0 * MIL)),
            max_length: Some(12000.0 * MIL),
            max_vias: Some(4),
            max_stub: None,
            max_unreferenced: Some(1.0),
            reference: gnd,
            return_via: None,
        },
        "lvds" => Spec {
            differential: true,
            impedance: Some((100.0, 10.0)),
            max_unreferenced: Some(1.0),
            reference: gnd,
            ..Default::default()
        },
        "rf-50" => Spec {
            impedance: Some((50.0, 10.0)),
            max_vias: Some(0),
            max_unreferenced: Some(0.5),
            reference: gnd,
            ..Default::default()
        },
        "cmos" => Spec { max_unreferenced: Some(2.0), reference: gnd, ..Default::default() },
        _ => return None,
    })
}

pub const PRESETS: [&str; 6] = ["usb3-gen1", "usb3-gen2", "usb2-hs", "lvds", "rf-50", "cmos"];

impl InterfaceFile {
    pub fn spec(&self, d: &mut Diags, at: &str) -> Spec {
        let mut s = match &self.preset {
            Some(p) => preset(p).unwrap_or_else(|| {
                d.error(at, format!("unknown preset `{p}` ({})", PRESETS.join(", ")));
                Spec::default()
            }),
            None => Spec::default(),
        };
        if let Some(v) = self.differential {
            s.differential = v;
        }
        if let Some(z) = self.impedance {
            s.impedance = Some((z.0, s.impedance.map(|x| x.1).unwrap_or(10.0)));
        }
        if let (Some(t), Some(z)) = (self.impedance_tolerance, s.impedance.as_mut()) {
            z.1 = t.0;
        }
        if let Some(v) = &self.max_skew {
            match Limit::parse(v) {
                Ok(l) => s.max_skew = Some(l),
                Err(e) => d.error(at, e),
            }
        }
        if let Some(v) = self.max_length {
            s.max_length = Some(v.to_mm());
        }
        if let Some(v) = self.max_vias {
            s.max_vias = Some(v);
        }
        if let Some(v) = self.max_stub {
            s.max_stub = Some(v.to_mm());
        }
        if let Some(v) = self.max_unreferenced {
            s.max_unreferenced = Some(v.to_mm());
        }
        if !self.reference.is_empty() {
            s.reference = self.reference.clone();
        }
        if let Some(v) = self.return_via {
            s.return_via = Some(v.to_mm());
        }
        s
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Lane {
    pub nets: Vec<String>,
    pub length_mm: f64,
    pub delay_ps: f64,
    pub vias: usize,
    pub stub_mm: f64,
    pub unreferenced_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Interface {
    pub name: String,
    pub preset: Option<String>,
    pub spec: Spec,
    pub lanes: Vec<Lane>,
    pub pairs: Vec<(usize, usize, f64, f64)>,
    pub timing: Vec<Timing>,
    pub max_bus_skew_ps: Option<f64>,
    pub clock_window_ps: Option<[f64; 2]>,
    pub measure: Vec<MeasureFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Timing {
    pub signal: String,
    pub lanes: Vec<usize>,
    pub clock: bool,
    pub delay_ps: f64,
    pub to_clock_ps: Option<f64>,
}

pub struct Ctx<'a> {
    pub board: &'a Board,
    pub copper: &'a [String],
    pub nets: &'a [LayoutNet],
    pub tracks: &'a [Track],
    pub vias: &'a [Via],
    pub parts: &'a [Placed],
    pub zones: &'a [ZoneFill],
    pub pairs: &'a [Pair],
}

pub fn check(files: &[InterfaceFile], cx: &Ctx, d: &mut Diags) -> Vec<Interface> {
    let mut out = Vec::new();
    for f in files {
        let at = format!("interface {}", f.name);
        let spec = f.spec(d, &at);
        let members: Vec<usize> = (0..cx.nets.len())
            .filter(|&n| f.nets.iter().any(|g| glob(g, &cx.nets[n].name)))
            .collect();
        if members.is_empty() {
            d.error(&at, format!("no net matches {}", f.nets.join(", ")));
            continue;
        }
        let mut sides: Vec<Vec<usize>> = Vec::new();
        let mut pairs = Vec::new();
        if spec.differential {
            let mut seen: Vec<Vec<(usize, usize)>> = Vec::new();
            for p in cx.pairs {
                if !members.contains(&p.p) || seen.contains(&p.chain) {
                    continue;
                }
                seen.push(p.chain.clone());
                let (pn, nn): (Vec<usize>, Vec<usize>) = p.chain.iter().copied().unzip();
                pairs.push((sides.len(), sides.len() + 1, 0.0, 0.0));
                sides.push(pn);
                sides.push(nn);
            }
            for &n in &members {
                if !sides.iter().flatten().any(|&m| m == n) {
                    d.error(&at, format!("{} has no pair partner", cx.nets[n].name));
                }
            }
        } else {
            sides = members.iter().map(|&n| vec![n]).collect();
        }
        for n in sides.iter().flatten() {
            impedance(&spec, &cx.nets[*n], cx.board, &at, d);
        }
        let lanes: Vec<Lane> = sides.iter().map(|s| lane(s, &spec, cx, &at, d)).collect();
        for p in &mut pairs {
            let (a, b) = (&lanes[p.0], &lanes[p.1]);
            p.2 = a.length_mm - b.length_mm;
            p.3 = a.delay_ps - b.delay_ps;
            let over = match spec.max_skew {
                Some(Limit::Mm(l)) => p.2.abs() > l + 1e-9,
                Some(Limit::Ps(l)) => p.3.abs() > l + 1e-9,
                None => false,
            };
            if over {
                d.error(
                    &at,
                    format!(
                        "{} / {}: skew {:.3} mm ({:.2} ps), the interface allows {}",
                        a.nets.join("+"),
                        b.nets.join("+"),
                        p.2,
                        p.3,
                        spec.max_skew.map(Limit::show).unwrap_or_default()
                    ),
                );
            }
        }
        let (timing, max_bus_skew_ps, clock_window_ps) = timing(f, &lanes, &pairs, &at, d);
        out.push(Interface {
            name: f.name.clone(),
            preset: f.preset.clone(),
            spec,
            lanes,
            pairs,
            timing,
            max_bus_skew_ps,
            clock_window_ps,
            measure: f.measure.clone(),
        });
    }
    out
}

fn timing(
    f: &InterfaceFile,
    lanes: &[Lane],
    pairs: &[(usize, usize, f64, f64)],
    at: &str,
    d: &mut Diags,
) -> (Vec<Timing>, Option<f64>, Option<[f64; 2]>) {
    let mut signals: Vec<(String, f64, Vec<String>)> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut paired = vec![false; lanes.len()];
    for p in pairs {
        paired[p.0] = true;
        paired[p.1] = true;
        let (a, b) = (&lanes[p.0], &lanes[p.1]);
        let nets: Vec<String> = a.nets.iter().chain(&b.nets).cloned().collect();
        signals.push((a.nets.join("+"), (a.delay_ps + b.delay_ps) / 2.0, nets));
        members.push(vec![p.0, p.1]);
    }
    for (i, l) in lanes.iter().enumerate() {
        if !paired[i] {
            signals.push((l.nets.join("+"), l.delay_ps, l.nets.clone()));
            members.push(vec![i]);
        }
    }
    let mut bus_limit = None;
    let mut window = None;
    let clock = f.clock.as_ref().map(|c| {
        let found = signals.iter().position(|s| s.2.iter().any(|n| n == c));
        if found.is_none() {
            d.error(at, format!("clock `{c}` is not one of the interface's nets"));
        }
        found
    });
    let clock = clock.flatten();
    let clock_delay = clock.map(|k| signals[k].1);
    let data: Vec<usize> = (0..signals.len()).filter(|k| Some(*k) != clock).collect();
    if let Some(v) = &f.max_bus_skew {
        match Limit::parse(v) {
            Ok(Limit::Ps(limit)) => {
                bus_limit = Some(limit);
                let lo = data.iter().map(|&k| signals[k].1).fold(f64::MAX, f64::min);
                let hi = data.iter().map(|&k| signals[k].1).fold(f64::MIN, f64::max);
                if data.len() > 1 && hi - lo > limit + 1e-9 {
                    let early = data
                        .iter()
                        .min_by(|a, b| signals[**a].1.total_cmp(&signals[**b].1))
                        .unwrap();
                    let late = data
                        .iter()
                        .max_by(|a, b| signals[**a].1.total_cmp(&signals[**b].1))
                        .unwrap();
                    d.error(
                        at,
                        format!(
                            "the bus spreads {:.1} ps, {} at {:.1} ps to {} at {:.1} ps, the interface allows {limit} ps",
                            hi - lo,
                            signals[*early].0,
                            signals[*early].1,
                            signals[*late].0,
                            signals[*late].1
                        ),
                    );
                }
            }
            Ok(Limit::Mm(_)) => d.error(at, "max_bus_skew is a time, like 20ps"),
            Err(e) => d.error(at, e),
        }
    }
    if let Some([lo, hi]) = &f.clock_window {
        match (Picos::parse(lo), Picos::parse(hi), clock_delay) {
            (Ok(lo), Ok(hi), Some(c)) => {
                window = Some([lo.0, hi.0]);
                for &k in &data {
                    let rel = signals[k].1 - c;
                    if rel < lo.0 - 1e-9 || rel > hi.0 + 1e-9 {
                        d.error(
                            at,
                            format!(
                                "{} arrives {rel:.1} ps after the clock, the window is {} to {} ps",
                                signals[k].0, lo.0, hi.0
                            ),
                        );
                    }
                }
            }
            (_, _, None) => d.error(at, "clock_window needs a `clock`"),
            _ => d.error(at, "clock_window is two times, like [\"-50ps\", \"50ps\"]"),
        }
    }
    let timing = signals
        .iter()
        .zip(members)
        .enumerate()
        .map(|(k, (s, lanes))| Timing {
            signal: s.0.clone(),
            lanes,
            clock: Some(k) == clock,
            delay_ps: s.1,
            to_clock_ps: clock_delay.map(|c| s.1 - c),
        })
        .collect();
    (timing, bus_limit, window)
}

pub struct Measured<'a> {
    pub name: &'a str,
    pub stale: bool,
    pub untracked: bool,
    pub fdtd: Option<&'a crate::sim::SimResult>,
    pub channel: Option<&'a crate::sim::ChannelResult>,
}

pub fn measure(iface: &Interface, sims: &[Measured], d: &mut Diags) {
    let at = format!("interface {}", iface.name);
    for m in &iface.measure {
        let Some(sim) = sims.iter().find(|s| s.name == m.sim) else {
            d.error(&at, format!("no sim named `{}`", m.sim));
            continue;
        };
        if sim.fdtd.is_none() && sim.channel.is_none() {
            d.error(&at, format!("`{}` has not run, `agentee sim {}` runs it", m.sim, m.sim));
            continue;
        }
        if sim.stale {
            d.error(&at, format!("the result of `{}` is stale, run it again", m.sim));
            continue;
        }
        if sim.untracked {
            d.error(
                &at,
                format!(
                    "`{}` was run before results recorded the copper they saw, run it again",
                    m.sim
                ),
            );
            continue;
        }
        if let Some(r) = sim.fdtd {
            s_params(m, r, &at, d);
        }
        if let Some(c) = sim.channel {
            eye(m, c, &at, d);
        }
    }
}

fn s_params(m: &MeasureFile, r: &crate::sim::SimResult, at: &str, d: &mut Diags) {
    use crate::rf::Cx;
    let port = |n: &str| r.ports.iter().position(|p| p == n);
    let up_to = match m.up_to.as_deref().map(crate::sim::freq) {
        Some(Some(f)) => f,
        Some(None) => {
            d.error(
                at,
                format!(
                    "cannot read up_to `{}` as a frequency",
                    m.up_to.clone().unwrap_or_default()
                ),
            );
            return;
        }
        None => r.freqs.last().copied().unwrap_or(0.0),
    };
    let at_k = |k: usize| -> crate::rf::Matrix {
        (0..r.ports.len())
            .map(|a| {
                (0..r.ports.len()).map(|b| Cx::new(r.s[a][b][k][0], r.s[a][b][k][1])).collect()
            })
            .collect()
    };
    let band: Vec<usize> =
        (0..r.freqs.len()).filter(|&k| r.freqs[k] <= up_to * (1.0 + 1e-9)).collect();
    let (thru, refl, conv): (Vec<Cx>, Vec<Cx>, Vec<Cx>) = if let Some(p) = &m.pair {
        let ids: Vec<usize> = match p.iter().map(|n| port(n)).collect::<Option<Vec<_>>>() {
            Some(v) => v,
            None => {
                d.error(
                    at,
                    format!(
                        "`{}` has no ports {}; it has {}",
                        m.sim,
                        p.join(", "),
                        r.ports.join(", ")
                    ),
                );
                return;
            }
        };
        if !r.excited.iter().all(|e| *e) {
            d.error(at, format!("`{}` must drive every port for mixed-mode readings", m.sim));
            return;
        }
        let mm: Vec<[[Cx; 4]; 4]> = band
            .iter()
            .map(|&k| crate::sparam::mixed_mode(&at_k(k), [ids[0], ids[2]], [ids[1], ids[3]]))
            .collect();
        (
            mm.iter().map(|x| x[1][0]).collect(),
            mm.iter().map(|x| x[0][0]).collect(),
            mm.iter().map(|x| x[3][0]).collect(),
        )
    } else if let Some([a, b]) = &m.through {
        let (Some(a), Some(b)) = (port(a), port(b)) else {
            d.error(at, format!("`{}` has no ports {a}, {b}", m.sim));
            return;
        };
        let get = |i: usize, j: usize| {
            band.iter().map(|&k| Cx::new(r.s[i][j][k][0], r.s[i][j][k][1])).collect()
        };
        (get(b, a), get(a, a), Vec::new())
    } else {
        d.error(at, format!("a measure on `{}` needs `pair` or `through`", m.sim));
        return;
    };
    let db = |c: &Cx| 20.0 * (c.re * c.re + c.im * c.im).sqrt().max(1e-15).log10();
    let worst = |v: &[Cx], hi: bool| -> Option<(f64, f64)> {
        v.iter()
            .zip(&band)
            .map(|(c, &k)| (db(c), r.freqs[k]))
            .reduce(|a, b| if (b.0 > a.0) == hi { b } else { a })
    };
    let ghz = |f: f64| f / 1e9;
    if let (Some(limit), Some((g, f))) = (m.max_loss, worst(&thru, false))
        && -g > limit + 1e-9
    {
        d.error(at, format!("{}: {:.2} dB of loss at {:.2} GHz, the interface allows {limit} dB up to {:.2} GHz", m.sim, -g, ghz(f), ghz(up_to)));
    }
    if let (Some(limit), Some((g, f))) = (m.min_return_loss, worst(&refl, true))
        && -g < limit - 1e-9
    {
        d.error(at, format!("{}: return loss {:.1} dB at {:.2} GHz, the interface wants {limit} dB up to {:.2} GHz", m.sim, -g, ghz(f), ghz(up_to)));
    }
    if let (Some(limit), Some((g, f))) = (m.max_mode_conversion, worst(&conv, true))
        && g > limit + 1e-9
    {
        d.error(at, format!("{}: differential to common mode {:.1} dB at {:.2} GHz, the interface allows {limit} dB", m.sim, g, ghz(f)));
    }
}

fn eye(m: &MeasureFile, c: &crate::sim::ChannelResult, at: &str, d: &mut Diags) {
    let reading = |label: &str| c.readings.iter().find(|r| r.label == label).map(|r| r.value);
    if let Some(v) = &m.min_eye_height {
        let want = crate::sim::parse_volts(v).map(|x| x * 1000.0);
        match (want, reading("eye height")) {
            (Some(w), Some(h)) if h + 1e-9 < w => d.error(
                at,
                format!("{}: eye height {h:.1} mV, the interface wants {w:.1} mV", m.sim),
            ),
            (None, _) => d.error(at, format!("cannot read min_eye_height `{v}` as a voltage")),
            _ => {}
        }
    }
    if let Some(v) = &m.min_eye_width {
        let want = match v.trim().strip_suffix("UI").or_else(|| v.trim().strip_suffix("ui")) {
            Some(u) => u.trim().parse::<f64>().ok().map(|u| u * c.ui_ps),
            None => Picos::parse(v).ok().map(|p| p.0),
        };
        match (want, reading("eye width")) {
            (Some(w), Some(e)) if e + 1e-9 < w => d.error(
                at,
                format!(
                    "{}: eye width {e:.1} ps ({:.2} UI), the interface wants {w:.1} ps",
                    m.sim,
                    e / c.ui_ps
                ),
            ),
            (None, _) => d.error(at, format!("cannot read min_eye_width `{v}`, use ps or UI")),
            _ => {}
        }
    }
}

fn impedance(spec: &Spec, net: &LayoutNet, board: &Board, at: &str, d: &mut Diags) {
    let Some((z, tol)) = spec.impedance else { return };
    let Some(class) = board.netclasses.iter().find(|c| c.name == net.class) else { return };
    let (lo, hi) = (z * (1.0 - tol / 100.0), z * (1.0 + tol / 100.0));
    match class.impedance {
        None => d.error(
            at,
            format!(
                "{} is in class {}, which has no impedance target; the interface wants {z} ohm",
                net.name, class.name
            ),
        ),
        Some(t) if t.0 < lo - 1e-9 || t.0 > hi + 1e-9 => d.error(
            at,
            format!(
                "{} is in class {} at {} ohm, outside the interface's {lo:.1} to {hi:.1} ohm",
                net.name, class.name, t.0
            ),
        ),
        _ => {}
    }
    if spec.differential && class.diff_gap.is_none() {
        d.error(
            at,
            format!(
                "{} is in class {}, which is not a differential pair class",
                net.name, class.name
            ),
        );
    }
}

fn lane(side: &[usize], spec: &Spec, cx: &Ctx, at: &str, d: &mut Diags) -> Lane {
    let names: Vec<String> = side.iter().map(|&n| cx.nets[n].name.clone()).collect();
    let label = names.join("+");
    let length_mm: f64 = side.iter().map(|&n| cx.nets[n].length_mm).sum();
    let mut delay_ps: f64 = side.iter().map(|&n| cx.nets[n].delay_ps).sum();
    let mut vias = 0;
    let mut stub_mm: f64 = 0.0;
    let mut worst_stub: Option<P> = None;
    let mut missing_return: Option<P> = None;
    let layer_of = |l: &str| cx.copper.iter().position(|c| c == l);
    let z = |i: usize| cx.board.stackup.copper_z(&cx.copper[i]).unwrap_or(0.0);
    for v in cx.vias.iter().filter(|v| side.contains(&v.net)) {
        vias += 1;
        let r = v.diameter / 2.0 + 1e-3;
        let mut used: Vec<usize> = cx
            .tracks
            .iter()
            .filter(|t| t.net == v.net)
            .filter(|t| {
                t.points.windows(2).any(|w| geom::point_segment_distance(v.at, w[0], w[1]) <= r)
            })
            .filter_map(|t| layer_of(&t.layer))
            .collect();
        for p in cx.parts {
            for q in p.pads.iter().filter(|q| q.net == Some(v.net)) {
                if q.outlines.iter().any(|o| {
                    geom::point_in_polygon(v.at, o)
                        || o.iter()
                            .zip(o.iter().cycle().skip(1))
                            .any(|(a, b)| geom::point_segment_distance(v.at, *a, *b) <= r)
                }) {
                    used.extend(q.copper.iter().filter_map(|l| layer_of(l)));
                }
            }
        }
        let span: Vec<usize> = v.layers.iter().filter_map(|l| layer_of(l)).collect();
        let (Some(&a), Some(&b)) = (span.iter().min(), span.iter().max()) else { continue };
        if let (Some(&lo), Some(&hi)) = (used.iter().min(), used.iter().max()) {
            let stub = (z(lo) - z(a)).abs().max(0.0) + (z(b) - z(hi)).abs().max(0.0);
            if stub > stub_mm {
                stub_mm = stub;
                worst_stub = Some(v.at);
            }
            let er = cx.board.stackup.er_between(&cx.copper[lo], &cx.copper[hi]);
            delay_ps += (z(hi) - z(lo)).abs() * er.sqrt() / 0.299_792_458;
        }
        if let Some(limit) = spec.return_via {
            let found = cx.vias.iter().any(|g| {
                spec.reference.iter().any(|r| r == &cx.nets[g.net].name)
                    && geom::dist(g.at, v.at) <= limit
            });
            if !found && missing_return.is_none() {
                missing_return = Some(v.at);
            }
        }
    }
    if let Some(m) = spec.max_vias
        && vias > m as usize
    {
        d.error(at, format!("{label} has {vias} vias, the interface allows {m}"));
    }
    if let (Some(m), Some(p)) = (spec.max_stub, worst_stub)
        && stub_mm > m + 1e-6
    {
        d.error(
            at,
            format!("{label}: the via at [{:.3}, {:.3}] leaves a {stub_mm:.3} mm stub, the interface allows {m:.3} mm", p[0], p[1]),
        );
    }
    if let (Some(limit), Some(p)) = (spec.return_via, missing_return) {
        d.error(
            at,
            format!(
                "{label}: no {} via within {limit:.2} mm of the via at [{:.3}, {:.3}] to carry the return current",
                spec.reference.join("/"),
                p[0],
                p[1]
            ),
        );
    }
    if let Some(m) = spec.max_length
        && length_mm > m + 1e-6
    {
        d.error(at, format!("{label} is {length_mm:.2} mm, the interface allows {m:.2} mm"));
    }
    let (unreferenced_mm, first) = unreferenced(side, spec, cx);
    if let (Some(m), Some(p)) = (spec.max_unreferenced, first)
        && unreferenced_mm > m + 1e-6
    {
        d.error(
            at,
            format!(
                "{label} runs {unreferenced_mm:.2} mm with no {} plane under it (first at [{:.3}, {:.3}] on {}), the interface allows {m:.2} mm",
                spec.reference.join("/"),
                p.0[0],
                p.0[1],
                p.1
            ),
        );
    }
    Lane { nets: names, length_mm, delay_ps, vias, stub_mm, unreferenced_mm }
}

fn unreferenced(side: &[usize], spec: &Spec, cx: &Ctx) -> (f64, Option<(P, String)>) {
    if spec.reference.is_empty() {
        return (0.0, None);
    }
    let by_layer: Vec<Vec<&ZoneFill>> =
        cx.copper.iter().map(|l| cx.zones.iter().filter(|z| &z.layer == l).collect()).collect();
    let is_reference = |z: &ZoneFill| spec.reference.iter().any(|r| r == &cx.nets[z.net].name);
    let referenced = |li: usize, pts: &[P; 3]| {
        let walk = |mut i: isize, dir: isize| -> bool {
            while i >= 0 && (i as usize) < cx.copper.len() {
                let here = &by_layer[i as usize];
                if here.iter().any(|z| is_reference(z) && pts.iter().all(|p| z.filled(*p))) {
                    return true;
                }
                if here.iter().any(|z| pts.iter().any(|p| z.filled(*p))) {
                    return false;
                }
                i += dir;
            }
            false
        };
        walk(li as isize - 1, -1) || walk(li as isize + 1, 1)
    };
    let step = 0.05;
    let mut total = 0.0;
    let mut first = None;
    for t in cx.tracks.iter().filter(|t| side.contains(&t.net)) {
        let Some(li) = cx.copper.iter().position(|c| c == &t.layer) else { continue };
        let own: Vec<(P, f64)> = cx
            .vias
            .iter()
            .filter(|v| v.net == t.net)
            .map(|v| (v.at, v.diameter / 2.0 + cx.nets[t.net].clearance.max(0.2) + t.width / 2.0))
            .collect();
        for w in t.points.windows(2) {
            let l = geom::dist(w[0], w[1]);
            if l < 1e-9 {
                continue;
            }
            let u = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
            let n = [-u[1] * t.width / 2.0, u[0] * t.width / 2.0];
            let k = (l / step).ceil() as usize;
            for s in 0..k {
                let f = (s as f64 + 0.5) / k as f64;
                let c = [w[0][0] + (w[1][0] - w[0][0]) * f, w[0][1] + (w[1][1] - w[0][1]) * f];
                let pts = [c, [c[0] + n[0], c[1] + n[1]], [c[0] - n[0], c[1] - n[1]]];
                if own.iter().any(|(v, r)| geom::dist(*v, c) < *r) {
                    continue;
                }
                if !referenced(li, &pts) {
                    total += l / k as f64;
                    if first.is_none() {
                        first = Some((c, t.layer.clone()));
                    }
                }
            }
        }
    }
    (total, first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_read_as_length_or_time() {
        assert_eq!(Limit::parse("5ps").unwrap(), Limit::Ps(5.0));
        assert_eq!(Limit::parse("0.1ns").unwrap(), Limit::Ps(100.0));
        assert!(matches!(Limit::parse("5mil").unwrap(), Limit::Mm(v) if (v - 0.127).abs() < 1e-9));
        assert!(matches!(Limit::parse("0.2mm").unwrap(), Limit::Mm(v) if (v - 0.2).abs() < 1e-9));
        assert!(Limit::parse("3 parsecs").is_err());
    }

    #[test]
    fn presets_resolve_and_overrides_win() {
        for p in PRESETS {
            assert!(preset(p).is_some(), "{p}");
        }
        let f = InterfaceFile {
            name: "x".into(),
            preset: Some("usb3-gen2".into()),
            nets: vec!["A*".into()],
            max_vias: Some(4),
            max_skew: Some("1ps".into()),
            ..Default::default()
        };
        let mut d = Diags::new("t");
        let s = f.spec(&mut d, "x");
        assert_eq!(s.max_vias, Some(4));
        assert_eq!(s.max_skew, Some(Limit::Ps(1.0)));
        assert_eq!(s.impedance, Some((90.0, 7.0)));
        assert!(d.list.is_empty());
    }
}

use crate::board::Board;
use crate::circuit::is_ground;
use crate::layout::Layout;
use crate::logic::{Check, Level};
use crate::project::Project;
use crate::logic::netlist_hash;
use crate::schematic::{PinRef, Schematic};
use crate::sim::SimKind;
use crate::symbol::PinType;
use crate::testpoint::{is_rail, rail_volts};
use crate::units::Percent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const EXT: &str = ".testplan.toml";
pub const DEFAULT_RESISTOR_TOLERANCE: f64 = 0.01;
pub const ASSUMED_RAIL_TOLERANCE: f64 = 0.05;
pub const ASSUMED_GROUND_BAND: f64 = 0.01;
pub const ASSUMED_CLOCK_PPM: f64 = 50.0;
pub const ASSUMED_SHORT_OHMS: f64 = 10.0;
pub const ASSUMED_CONTINUITY_OHMS: f64 = 1.0;
pub const ASSUMED_DIODE_DROP: [f64; 2] = [0.1, 0.8];
pub const FIRMWARE: &str = "depends on firmware";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub design: String,
    pub hash: String,
    pub summary: String,
    #[serde(default, rename = "point")]
    pub rows: Vec<Row>,
    #[serde(default, rename = "unpowered", skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Unpowered>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unpowered {
    pub kind: String,
    pub a: String,
    pub b: String,
    pub a_net: String,
    pub b_net: String,
    pub measure: String,
    pub unit: String,
    pub min: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    pub basis: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<Source>,
}

impl Unpowered {
    pub fn is_assumed(&self) -> bool {
        self.source.iter().any(|s| s.kind == "assumed" || s.confidence == "assumed")
    }

    pub fn line(&self) -> String {
        let range = match self.max {
            Some(m) => format!("{} to {} {}", self.min, m, self.unit),
            None => format!("{} {} or more", self.min, self.unit),
        };
        format!("{} {} to {}: {}", self.kind, self.a, self.b, range)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Models {
    pub logic: Vec<LogicModel>,
    pub rf: Vec<RfModel>,
}

#[derive(Clone, Debug)]
pub struct LogicModel {
    pub sim: String,
    pub schematic: String,
    pub verified: bool,
    pub expects: Vec<crate::logic::Expect>,
}

#[derive(Clone, Debug)]
pub struct RfModel {
    pub sim: String,
    pub verified: bool,
    pub links: Vec<RfLink>,
}

#[derive(Clone, Debug)]
pub struct RfLink {
    pub from_port: String,
    pub to_port: String,
    pub from_net: String,
    pub to_net: String,
    pub db_min: f64,
    pub db_max: f64,
    pub f_min: f64,
    pub f_max: f64,
}

impl Models {
    pub fn from_project(p: &Project, sch: &Schematic) -> Models {
        let mut m = Models::default();
        for e in &p.sims {
            let sim = &e.item;
            match sim.kind {
                SimKind::Logic => {
                    let Some(spec) = sim.logic.as_ref().filter(|l| l.schematic == sch.name) else {
                        continue;
                    };
                    let verified = !sim.stale
                        && sim.logic_result.as_ref().is_some_and(|r| r.failures.is_empty());
                    m.logic.push(LogicModel {
                        sim: e.name.clone(),
                        schematic: spec.schematic.clone(),
                        verified,
                        expects: spec.expects.clone(),
                    });
                }
                SimKind::Fdtd => {
                    let (Some(res), Some(layout)) = (
                        sim.result.as_ref(),
                        p.layouts.iter().find(|l| l.name == sim.layout),
                    ) else {
                        continue;
                    };
                    if layout.item.schematic != sch.name && !layout.item.schematic.is_empty() {
                        continue;
                    }
                    let net_of_port = |i: usize| -> Option<String> {
                        let port = sim.ports.get(i)?;
                        let pad = layout.item.parts.get(port.part)?.pads.get(port.pad)?;
                        Some(layout.item.nets.get(pad.net?)?.name.clone())
                    };
                    let mut links = Vec::new();
                    for from in 0..sim.ports.len() {
                        if !res.excited.get(from).copied().unwrap_or(false) {
                            continue;
                        }
                        for to in 0..sim.ports.len() {
                            let (Some(fnet), Some(tnet)) = (net_of_port(from), net_of_port(to))
                            else {
                                continue;
                            };
                            if from == to || fnet == tnet {
                                continue;
                            }
                            let db = res.db(to, from);
                            if db.is_empty() {
                                continue;
                            }
                            links.push(RfLink {
                                from_port: sim.ports[from].name.clone(),
                                to_port: sim.ports[to].name.clone(),
                                from_net: fnet,
                                to_net: tnet,
                                db_min: db.iter().copied().fold(f64::INFINITY, f64::min),
                                db_max: db.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                                f_min: res.freqs.first().copied().unwrap_or(0.0),
                                f_max: res.freqs.last().copied().unwrap_or(0.0),
                            });
                        }
                    }
                    m.rf.push(RfModel { sim: e.name.clone(), verified: !sim.stale, links });
                }
                _ => {}
            }
        }
        m
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    #[serde(rename = "ref")]
    pub reference: String,
    pub net: String,
    pub role: String,
    pub order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest: Option<String>,
    pub measure: String,
    pub expect: Expect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpowered: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expect {
    Unknown(String),
    Reading(Reading),
    Logic(Logic),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Logic {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<LogicState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_volts: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vih: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vil: Option<f64>,
    pub basis: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogicState {
    pub when: String,
    pub level: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal: Option<f64>,
    pub min: f64,
    pub max: f64,
    pub basis: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub x: f64,
    pub y: f64,
    pub side: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    pub confidence: String,
    pub detail: String,
}

impl Row {
    pub fn is_unknown(&self) -> bool {
        matches!(&self.expect, Expect::Unknown(s) if s != FIRMWARE)
    }

    pub fn is_firmware(&self) -> bool {
        matches!(&self.expect, Expect::Unknown(s) if s == FIRMWARE)
    }

    pub fn is_assumed(&self) -> bool {
        self.source.iter().any(|s| s.kind == "assumed" || s.confidence == "assumed")
    }
}

impl Plan {
    pub fn unknown(&self) -> usize {
        self.rows.iter().filter(|r| r.is_unknown()).count()
    }

    pub fn assumed(&self) -> usize {
        self.rows.iter().filter(|r| r.is_assumed()).count()
    }

    pub fn firmware(&self) -> usize {
        self.rows.iter().filter(|r| r.is_firmware()).count()
    }

    pub fn unpowered_assumed(&self) -> usize {
        self.checks.iter().filter(|c| c.is_assumed()).count()
    }

    pub fn is_stale(&self, sch: &Schematic) -> bool {
        self.hash != hash_of(sch)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn parse(src: &str) -> Result<Plan, String> {
        toml::from_str(src).map_err(|e| e.to_string())
    }
}

pub fn path_for(schematic: &std::path::Path, name: &str) -> std::path::PathBuf {
    schematic.with_file_name(format!("{name}{EXT}"))
}

pub fn count(sch: &Schematic) -> usize {
    (0..sch.parts.len()).filter(|i| is_test_point(sch, *i)).count()
}

pub fn hash_of(sch: &Schematic) -> String {
    format!("{:016x}", netlist_hash(sch))
}

#[derive(Clone, Copy, Debug)]
struct Volts {
    nominal: f64,
    min: f64,
    max: f64,
}

struct Known {
    volts: Volts,
    basis: String,
    sources: Vec<Source>,
    conditions: Vec<String>,
    order: u32,
}

const MAX_STATES: usize = 16;

fn level_text(l: Level) -> String {
    l.char().to_string()
}

pub fn parse_hz(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    let t = t.strip_suffix("hz").unwrap_or(&t);
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let n: f64 = t[..end].parse().ok()?;
    let k = match &t[end..] {
        "" => 1.0,
        "k" => 1e3,
        "m" => 1e6,
        "g" => 1e9,
        _ => return None,
    };
    (n > 0.0).then(|| round(n * k))
}

fn parse_ppm(s: &str) -> Option<f64> {
    let t = s.trim().trim_start_matches(['+', '-', '±']).trim_start_matches('/').trim();
    let t = t.strip_suffix("ppm").unwrap_or(t).trim();
    t.parse::<f64>().ok().filter(|v| *v >= 0.0)
}

pub fn fmt_hz(hz: f64) -> String {
    let (div, unit) = if hz >= 1e9 {
        (1e9, "GHz")
    } else if hz >= 1e6 {
        (1e6, "MHz")
    } else if hz >= 1e3 {
        (1e3, "kHz")
    } else {
        (1.0, "Hz")
    };
    format!("{} {unit}", round(hz / div))
}

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

pub fn is_test_point(sch: &Schematic, part: usize) -> bool {
    let p = &sch.parts[part];
    let tp = p
        .reference
        .strip_prefix("TP")
        .is_some_and(|r| r.is_empty() || r.starts_with(|c: char| c.is_ascii_digit()));
    tp || p.footprint.as_deref().is_some_and(|f| f.starts_with("TestPoint"))
}

fn tolerance_of(sch: &Schematic, part: usize) -> (f64, bool) {
    sch.parts[part]
        .fields
        .get("tolerance")
        .and_then(|t| Percent::parse(t).ok())
        .map(|t| (t.0 / 100.0, true))
        .unwrap_or((DEFAULT_RESISTOR_TOLERANCE, false))
}

struct Resistor {
    part: usize,
    ohms: f64,
    other: usize,
}

fn resistors_on(sch: &Schematic, net: usize) -> Vec<Resistor> {
    let mut out = Vec::new();
    for r in &sch.nets[net].pins {
        let p = &sch.parts[r.part];
        if p.dnp || p.symbol.reference != "R" || p.symbol.pins.len() != 2 {
            continue;
        }
        let Some(ohms) = crate::sim::parse_value(&p.value).filter(|v| *v > 0.0) else {
            continue;
        };
        let far = 1 - r.pin;
        let Some(other) = sch.net_of(PinRef { part: r.part, pin: far }) else {
            continue;
        };
        out.push(Resistor { part: r.part, ohms, other });
    }
    out
}

fn pin_net(sch: &Schematic, part: usize, name: &str) -> Option<usize> {
    let pin = sch.parts[part].symbol.pins.iter().position(|p| p.name == name)?;
    sch.net_of(PinRef { part, pin })
}

fn corners(values: &[[f64; 2]], f: impl Fn(&[f64]) -> f64) -> [f64; 2] {
    let n = values.len();
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut pick = vec![0.0; n];
    for mask in 0..(1usize << n) {
        for (i, v) in values.iter().enumerate() {
            pick[i] = v[(mask >> i) & 1];
        }
        let x = f(&pick);
        lo = lo.min(x);
        hi = hi.max(x);
    }
    [lo, hi]
}

fn mpn(sch: &Schematic, part: usize) -> String {
    let p = &sch.parts[part];
    p.fields.get("mpn").cloned().unwrap_or_else(|| p.symbol_name.clone())
}

struct Builder<'a> {
    sch: &'a Schematic,
    board: Option<&'a Board>,
    models: &'a Models,
    known: HashMap<usize, Known>,
    reasons: HashMap<usize, String>,
}

impl<'a> Builder<'a> {
    fn ground(&self, net: usize) -> bool {
        is_ground(&self.sch.nets[net].name)
    }

    fn regulators(&mut self) {
        for part in 0..self.sch.parts.len() {
            let p = &self.sch.parts[part];
            let Some(reg) = p.symbol.regulator.clone() else { continue };
            if p.dnp {
                continue;
            }
            let Some(out) = pin_net(self.sch, part, &reg.output) else { continue };
            let label = format!("{} ({})", p.reference, mpn(self.sch, part));
            let datasheet = if reg.datasheet.is_empty() {
                p.symbol.datasheet.clone()
            } else {
                reg.datasheet.clone()
            };
            let mut sources = vec![Source {
                kind: "datasheet".into(),
                confidence: "high".into(),
                detail: format!(
                    "{label} output accuracy {:.2}%{}",
                    reg.accuracy * 100.0,
                    if datasheet.is_empty() { String::new() } else { format!(", {datasheet}") }
                ),
            }];
            let mut basis = format!("{label} output accuracy {:.2}%", reg.accuracy * 100.0);
            let (nominal, range) = if let Some(v) = reg.vout {
                (v, [v * (1.0 - reg.accuracy), v * (1.0 + reg.accuracy)])
            } else {
                let (Some(vref), Some(fb)) = (reg.vref, reg.feedback.as_deref()) else { continue };
                let Some(fbnet) = pin_net(self.sch, part, fb) else {
                    self.reasons.insert(out, format!("{label} feedback pin {fb} is not connected"));
                    continue;
                };
                let rs = resistors_on(self.sch, fbnet);
                let top = rs.iter().find(|r| r.other == out);
                let bottom = rs.iter().find(|r| self.ground(r.other));
                let (Some(top), Some(bottom)) = (top, bottom) else {
                    self.reasons.insert(
                        out,
                        format!(
                            "{label} feedback net {} has no divider to the output and ground",
                            self.sch.nets[fbnet].name
                        ),
                    );
                    continue;
                };
                let (tt, tt_set) = tolerance_of(self.sch, top.part);
                let (bt, bt_set) = tolerance_of(self.sch, bottom.part);
                let vals = [
                    [vref * (1.0 - reg.accuracy), vref * (1.0 + reg.accuracy)],
                    [top.ohms * (1.0 - tt), top.ohms * (1.0 + tt)],
                    [bottom.ohms * (1.0 - bt), bottom.ohms * (1.0 + bt)],
                ];
                let f = |v: &[f64]| v[0] * (1.0 + v[1] / v[2]);
                let range = corners(&vals, f);
                basis.push_str(&format!(
                    ", divider {} {} over {} {}",
                    self.sch.parts[top.part].reference,
                    self.sch.parts[top.part].value,
                    self.sch.parts[bottom.part].reference,
                    self.sch.parts[bottom.part].value,
                ));
                sources.push(Source {
                    kind: "arithmetic".into(),
                    confidence: "high".into(),
                    detail: format!(
                        "Vout = Vref (1 + Rtop / Rbottom), Vref {vref} V, worst corners of Vref and both resistors"
                    ),
                });
                if !(tt_set && bt_set) {
                    sources.push(Source {
                        kind: "assumed".into(),
                        confidence: "assumed".into(),
                        detail: format!(
                            "resistor tolerance {}% for a divider resistor with no `tolerance` field; set it on the part and re-run",
                            DEFAULT_RESISTOR_TOLERANCE * 100.0
                        ),
                    });
                }
                (f(&[vref, top.ohms, bottom.ohms]), range)
            };
            let mut conditions = Vec::new();
            let mut order = 1;
            if let Some(input) = reg.input.as_deref().and_then(|i| pin_net(self.sch, part, i)) {
                conditions.push(format!(
                    "input {} present on {}",
                    reg.input.as_deref().unwrap_or("VIN"),
                    self.sch.nets[input].name
                ));
                order = order.max(self.input_order(input, 0) + 1);
            }
            if let Some(en) = reg.enable.as_deref().and_then(|e| pin_net(self.sch, part, e)) {
                conditions.push(format!(
                    "{} asserted on {}",
                    reg.enable.as_deref().unwrap_or("EN"),
                    self.sch.nets[en].name
                ));
            }
            self.known.insert(
                out,
                Known {
                    volts: Volts { nominal, min: range[0], max: range[1] },
                    basis,
                    sources,
                    conditions,
                    order,
                },
            );
        }
    }

    fn input_order(&self, net: usize, depth: usize) -> u32 {
        if depth > 8 {
            return 1;
        }
        for part in 0..self.sch.parts.len() {
            let p = &self.sch.parts[part];
            let Some(reg) = &p.symbol.regulator else { continue };
            if pin_net(self.sch, part, &reg.output) == Some(net) {
                let input = reg.input.as_deref().and_then(|i| pin_net(self.sch, part, i));
                return 1 + input.map_or(0, |i| self.input_order(i, depth + 1));
            }
        }
        0
    }

    fn declared(&mut self) {
        for (name, range) in &self.sch.rails {
            let Some(net) = self.sch.nets.iter().position(|n| &n.name == name) else { continue };
            if self.known.contains_key(&net) || self.ground(net) {
                continue;
            }
            let order = 1 + self.input_order(net, 0);
            let nominal = (range[0] + range[1]) / 2.0;
            let (volts, basis, source) = if range[1] > range[0] {
                (
                    Volts { nominal, min: range[0], max: range[1] },
                    format!("rail range declared in the schematic `rails` table for {name}"),
                    Source {
                        kind: "schematic".into(),
                        confidence: "declared".into(),
                        detail: format!("rails.{name} = [{}, {}]", range[0], range[1]),
                    },
                )
            } else {
                let band = nominal.abs() * ASSUMED_RAIL_TOLERANCE;
                (
                    Volts { nominal, min: nominal - band, max: nominal + band },
                    format!(
                        "assumed {}%: no regulator tolerance known, measure a known good board",
                        ASSUMED_RAIL_TOLERANCE * 100.0
                    ),
                    Source {
                        kind: "assumed".into(),
                        confidence: "assumed".into(),
                        detail: format!(
                            "{name} nominal {nominal} V from the net name and netclass, +-{}% default; measure a known good board",
                            ASSUMED_RAIL_TOLERANCE * 100.0
                        ),
                    },
                )
            };
            self.known.insert(
                net,
                Known {
                    volts,
                    basis,
                    sources: vec![source],
                    conditions: vec!["power applied".into()],
                    order,
                },
            );
        }
    }

    fn divider(&self, net: usize) -> Option<Known> {
        let mut others = Vec::new();
        for r in &self.sch.nets[net].pins {
            let p = &self.sch.parts[r.part];
            if is_test_point(self.sch, r.part) || p.symbol.reference == "C" {
                continue;
            }
            if p.symbol.reference != "R" || p.symbol.pins.len() != 2 {
                return None;
            }
        }
        let rs = resistors_on(self.sch, net);
        for r in &rs {
            if r.other != net && !others.contains(&r.other) {
                others.push(r.other);
            }
        }
        if rs.len() < 2 || others.len() != 2 {
            return None;
        }
        let side = |n: usize| -> Option<Volts> {
            if self.ground(n) {
                Some(Volts { nominal: 0.0, min: 0.0, max: 0.0 })
            } else {
                self.known.get(&n).map(|k| k.volts)
            }
        };
        let (va, vb) = (side(others[0])?, side(others[1])?);
        let ra: Vec<&Resistor> = rs.iter().filter(|r| r.other == others[0]).collect();
        let rb: Vec<&Resistor> = rs.iter().filter(|r| r.other == others[1]).collect();
        if ra.len() != 1 || rb.len() != 1 {
            return None;
        }
        let (a, b) = (ra[0], rb[0]);
        let (ta, a_set) = tolerance_of(self.sch, a.part);
        let (tb, b_set) = tolerance_of(self.sch, b.part);
        let vals = [
            [va.min, va.max],
            [vb.min, vb.max],
            [a.ohms * (1.0 - ta), a.ohms * (1.0 + ta)],
            [b.ohms * (1.0 - tb), b.ohms * (1.0 + tb)],
        ];
        let f = |v: &[f64]| (v[0] * v[3] + v[1] * v[2]) / (v[2] + v[3]);
        let range = corners(&vals, f);
        let nominal = f(&[va.nominal, vb.nominal, a.ohms, b.ohms]);
        let mut sources = vec![Source {
            kind: "arithmetic".into(),
            confidence: "high".into(),
            detail: format!(
                "divider {} {} to {}, {} {} to {}, worst corners of both rails and both resistors",
                self.sch.parts[a.part].reference,
                self.sch.parts[a.part].value,
                self.sch.nets[others[0]].name,
                self.sch.parts[b.part].reference,
                self.sch.parts[b.part].value,
                self.sch.nets[others[1]].name
            ),
        }];
        if !(a_set && b_set) {
            sources.push(Source {
                kind: "assumed".into(),
                confidence: "assumed".into(),
                detail: format!(
                    "resistor tolerance {}% for a resistor with no `tolerance` field",
                    DEFAULT_RESISTOR_TOLERANCE * 100.0
                ),
            });
        }
        let order = [others[0], others[1]]
            .iter()
            .filter_map(|n| self.known.get(n).map(|k| k.order))
            .max()
            .unwrap_or(0)
            + 1;
        Some(Known {
            volts: Volts { nominal, min: range[0], max: range[1] },
            basis: "resistor divider with part tolerances".into(),
            sources,
            conditions: vec![format!(
                "{} and {} at their expected voltage",
                self.sch.nets[others[0]].name, self.sch.nets[others[1]].name
            )],
            order,
        })
    }

    fn sense(&self, net: usize) -> Option<(Reading, Vec<Source>, String, u32)> {
        let class_current = self
            .board
            .and_then(|b| b.netclasses.iter().find(|c| c.name == self.sch.nets[net].class))
            .and_then(|c| c.current.as_ref())
            .and_then(|c| crate::circuit::si(&c.to_string(), &["a"]))?;
        let r = resistors_on(self.sch, net)
            .into_iter()
            .filter(|r| r.ohms <= 1.0 && r.other != net)
            .min_by(|a, b| a.ohms.total_cmp(&b.ohms))?;
        let p = &self.sch.parts[r.part];
        Some((
            Reading {
                unit: "A".into(),
                nominal: None,
                min: 0.0,
                max: round(class_current),
                basis: format!(
                    "I = V / R across {} {} ({}); {} V per A; upper bound is the net class current rating",
                    p.reference,
                    p.value,
                    self.sch.nets[r.other].name,
                    round(r.ohms)
                ),
            },
            vec![Source {
                kind: "arithmetic".into(),
                confidence: "medium".into(),
                detail: format!(
                    "current = V({}, {}) / {} ohm; limit {class_current} A from netclass {}",
                    self.sch.nets[net].name,
                    self.sch.nets[r.other].name,
                    round(r.ohms),
                    self.sch.nets[net].class
                ),
            }],
            format!("{} V per A across {} {}", round(r.ohms), p.reference, p.value),
            2,
        ))
    }

    fn row(&self, part: usize, layout: Option<&Layout>) -> Row {
        let p = &self.sch.parts[part];
        let net = p
            .symbol
            .pins
            .iter()
            .enumerate()
            .find_map(|(i, _)| self.sch.net_of(PinRef { part, pin: i }));
        let location = layout.and_then(|l| {
            l.parts.iter().find(|q| q.reference == p.reference).map(|q| Location {
                x: round(q.at.to_mm()[0]),
                y: round(q.at.to_mm()[1]),
                side: if q.bottom { "bottom" } else { "top" }.into(),
            })
        });
        let nearest = layout.and_then(|l| nearest_reference(l, &p.reference));
        let mut row = Row {
            reference: p.reference.clone(),
            net: String::new(),
            role: "unconnected".into(),
            order: 100,
            location,
            nearest,
            measure: String::new(),
            expect: Expect::Unknown(String::new()),
            reason: None,
            conditions: Vec::new(),
            source: Vec::new(),
            unpowered: Vec::new(),
        };
        if p.dnp {
            row.conditions.push("pad is not fitted (dnp)".into());
        }
        let Some(net) = net else {
            row.measure = "nothing".into();
            row.expect = Expect::Unknown("unknown".into());
            row.reason = Some("the test point is not on any net".into());
            return row;
        };
        row.net = self.sch.nets[net].name.clone();
        row.role = role_of(
            &self.sch.nets[net].name,
            self.known.contains_key(&net),
            self.board.is_some_and(|b| {
                b.netclasses
                    .iter()
                    .any(|c| c.name == self.sch.nets[net].class && c.impedance.is_some())
            }),
        );
        if self.ground(net) {
            row.order = 0;
            row.measure = "DC voltage against the board ground reference".into();
            row.expect = Expect::Reading(Reading {
                unit: "V".into(),
                nominal: Some(0.0),
                min: -ASSUMED_GROUND_BAND,
                max: ASSUMED_GROUND_BAND,
                basis: "0 V by definition; band is an assumed meter and contact allowance".into(),
            });
            row.source.push(Source {
                kind: "assumed".into(),
                confidence: "assumed".into(),
                detail: format!(
                    "+-{} V band; no dc drop simulation is used, measure a known-good board",
                    ASSUMED_GROUND_BAND
                ),
            });
            return row;
        }
        if let Some(k) = self.known.get(&net) {
            self.fill_volts(&mut row, k);
            return row;
        }
        if let Some(reason) = self.reasons.get(&net) {
            row.measure = "DC voltage against ground".into();
            row.expect = Expect::Unknown("unknown".into());
            row.reason = Some(reason.clone());
            row.order = 10;
            return row;
        }
        if let Some(k) = self.divider(net) {
            row.role = "bias".into();
            self.fill_volts(&mut row, &k);
            return row;
        }
        if let Some((reading, sources, measure, order)) = self.sense(net) {
            row.role = "current sense".into();
            row.measure = format!("DC voltage across the shunt, {measure}; read current as V / R");
            row.expect = Expect::Reading(reading);
            row.source = sources;
            row.conditions = vec!["load running at its expected current".into()];
            row.order = 50 + order;
            return row;
        }
        let name = &self.sch.nets[net].name;
        if (is_rail(name) || rail_volts(name).is_some())
            && let Some(v) = rail_volts(name)
        {
            let k = Known {
                volts: Volts {
                    nominal: v,
                    min: v * (1.0 - ASSUMED_RAIL_TOLERANCE),
                    max: v * (1.0 + ASSUMED_RAIL_TOLERANCE),
                },
                basis: format!(
                    "voltage taken from the net name {name} with an assumed +-{}%",
                    ASSUMED_RAIL_TOLERANCE * 100.0
                ),
                sources: vec![Source {
                    kind: "assumed".into(),
                    confidence: "assumed".into(),
                    detail: format!(
                        "no regulator model drives {name}; nominal read from its name, +-{}% default; measure a known-good board",
                        ASSUMED_RAIL_TOLERANCE * 100.0
                    ),
                }],
                conditions: vec!["power applied".into()],
                order: 1,
            };
            self.fill_volts(&mut row, &k);
            return row;
        }
        if self.clock(net, &mut row) {
            return row;
        }
        if self.rf(net, &mut row) {
            return row;
        }
        if matches!(row.role.as_str(), "reset" | "digital bus" | "signal" | "clock")
            && self.digital(net, &mut row)
        {
            return row;
        }
        row.order = 100;
        row.measure = measure_for(&row.role);
        row.reason = Some(match row.role.as_str() {
            "power rail" => {
                format!("no regulator model drives {name} and its name carries no voltage")
            }
            "RF" => "RF net: no RF simulation result reaches it, and a high impedance probe would load it; use a directional pad or skip".into(),
            _ => "no model for this net: no regulator, divider, shunt or logic expectation".into(),
        });
        row.expect = Expect::Unknown("unknown".into());
        row
    }

    fn net_matches(&self, net: usize, name: &str) -> bool {
        let mine = &self.sch.nets[net].name;
        mine == name || mine.rsplit('/').next() == Some(name.rsplit('/').next().unwrap_or(name))
    }

    fn net_volts(&self, net: usize) -> Option<Volts> {
        if self.ground(net) {
            return Some(Volts { nominal: 0.0, min: 0.0, max: 0.0 });
        }
        if let Some(k) = self.known.get(&net) {
            return Some(k.volts);
        }
        let name = &self.sch.nets[net].name;
        let v = self.sch.rails.get(name).map(|r| Volts {
            nominal: (r[0] + r[1]) / 2.0,
            min: r[0],
            max: r[1],
        });
        v.or_else(|| {
            rail_volts(name).filter(|_| is_rail(name)).map(|v| Volts {
                nominal: v,
                min: v * (1.0 - ASSUMED_RAIL_TOLERANCE),
                max: v * (1.0 + ASSUMED_RAIL_TOLERANCE),
            })
        })
    }

    fn timing_part(&self, part: usize) -> Option<(bool, Option<f64>)> {
        let p = &self.sch.parts[part];
        if p.dnp {
            return None;
        }
        let text = format!("{} {} {}", p.symbol_name, p.value, p.symbol.description).to_lowercase();
        let osc = ["oscillator", "tcxo", "vcxo", "ocxo", "xo "].iter().any(|k| text.contains(k));
        let xtal = ["crystal", "xtal", "resonator"].iter().any(|k| text.contains(k));
        let declared = p.fields.get("frequency").or_else(|| p.fields.get("freq"));
        let lead = p.reference.starts_with(['X', 'Y']);
        if declared.is_none() && !((osc || xtal) && lead) {
            return None;
        }
        let hz = declared.and_then(|f| parse_hz(f)).or_else(|| parse_hz(&p.value));
        Some((osc && !xtal, hz))
    }

    fn clock(&self, net: usize, row: &mut Row) -> bool {
        for r in &self.sch.nets[net].pins {
            let pin = &self.sch.parts[r.part].symbol.pins[r.pin];
            let upper = pin.name.to_ascii_uppercase();
            let power_pin = matches!(pin.kind, PinType::PowerIn | PinType::PowerOut)
                || ["GND", "VCC", "VDD", "VSS", "VEE", "EN", "OE", "NC", "VC", "STBY"]
                    .iter()
                    .any(|k| upper == *k || upper.starts_with(&format!("{k}_")));
            if power_pin {
                continue;
            }
            let Some((oscillator, hz)) = self.timing_part(r.part) else { continue };
            let p = &self.sch.parts[r.part];
            let what = if oscillator { "oscillator" } else { "crystal" };
            row.role = "clock".into();
            row.order = 80;
            row.measure = "frequency".into();
            let Some(hz) = hz else {
                row.expect = Expect::Unknown("unknown".into());
                row.reason = Some(format!(
                    "{} ({what}) has no frequency: set the `frequency` field on the part, or give a frequency as its value",
                    p.reference
                ));
                return true;
            };
            let declared = p
                .fields
                .get("ppm")
                .or_else(|| p.fields.get("stability"))
                .or_else(|| p.fields.get("frequency_tolerance"))
                .and_then(|t| parse_ppm(t));
            let ppm = declared.unwrap_or(ASSUMED_CLOCK_PPM);
            let label = format!("{} ({})", p.reference, mpn(self.sch, r.part));
            row.expect = Expect::Reading(Reading {
                unit: "Hz".into(),
                nominal: Some(hz),
                min: round(hz * (1.0 - ppm / 1e6)),
                max: round(hz * (1.0 + ppm / 1e6)),
                basis: format!(
                    "{label} {what} {} +-{ppm} ppm; excludes the counter timebase error",
                    fmt_hz(hz)
                ),
            });
            row.source.push(Source {
                kind: "part field".into(),
                confidence: "declared".into(),
                detail: format!("{label} frequency {}", fmt_hz(hz)),
            });
            if declared.is_none() {
                row.source.push(Source {
                    kind: "assumed".into(),
                    confidence: "assumed".into(),
                    detail: format!(
                        "+-{ASSUMED_CLOCK_PPM} ppm: no `ppm` field on {}; take the datasheet figure and set it",
                        p.reference
                    ),
                });
            }
            if oscillator {
                row.conditions.push("oscillator supplied and enabled".into());
            } else {
                row.conditions.push("oscillator circuit running".into());
                row.conditions.push(
                    "a probe adds capacitance and pulls a crystal off frequency: use a FET probe under 1 pF, or read the buffered clock output instead"
                        .into(),
                );
            }
            return true;
        }
        false
    }

    fn rf(&self, net: usize, row: &mut Row) -> bool {
        let mut found = false;
        for m in &self.models.rf {
            for l in m.links.iter().filter(|l| self.net_matches(net, &l.to_net)) {
                row.role = "RF".into();
                row.order = 70;
                row.measure = format!(
                    "RF level through a 50 ohm directional pad or analyser port, as the transfer from {} in dB",
                    l.from_port
                );
                if !found {
                    row.expect = Expect::Reading(Reading {
                        unit: "dB".into(),
                        nominal: None,
                        min: round(l.db_min),
                        max: round(l.db_max),
                        basis: format!(
                            "|S| from {} to {} over {} to {} in {}",
                            l.from_port,
                            l.to_port,
                            fmt_hz(l.f_min),
                            fmt_hz(l.f_max),
                            m.sim
                        ),
                    });
                    row.conditions.push(format!(
                        "RF source applied at {} on {}",
                        l.from_port, l.from_net
                    ));
                    row.conditions.push(
                        "a high impedance probe loads this net and corrupts the level: use a directional pad or skip"
                            .into(),
                    );
                    row.reason = Some(
                        "relative level from the simulation, not an absolute power".into(),
                    );
                }
                row.source.push(Source {
                    kind: "simulation".into(),
                    confidence: if m.verified { "high" } else { "stale" }.into(),
                    detail: format!(
                        "{} port {} to {}: {} to {} dB{}",
                        m.sim,
                        l.from_port,
                        l.to_port,
                        round(l.db_min),
                        round(l.db_max),
                        if m.verified { "" } else { "; layout changed since the run, re-run it" }
                    ),
                });
                found = true;
            }
        }
        found
    }

    fn pull(&self, net: usize) -> Option<(bool, Volts, String)> {
        let mut up: Vec<(Resistor, Volts)> = Vec::new();
        let mut down: Vec<Resistor> = Vec::new();
        for r in resistors_on(self.sch, net) {
            if r.other == net {
                continue;
            }
            if self.ground(r.other) {
                down.push(r);
            } else if let Some(v) = self.net_volts(r.other).filter(|v| v.nominal > 0.0) {
                up.push((r, v));
            }
        }
        match (up.first(), down.first(), up.len() + down.len()) {
            (Some((r, v)), None, 1) => Some((
                true,
                *v,
                format!(
                    "{} {} pull up to {}",
                    self.sch.parts[r.part].reference,
                    self.sch.parts[r.part].value,
                    self.sch.nets[r.other].name
                ),
            )),
            (None, Some(r), 1) => Some((
                false,
                Volts { nominal: 0.0, min: 0.0, max: 0.0 },
                format!(
                    "{} {} pull down",
                    self.sch.parts[r.part].reference, self.sch.parts[r.part].value
                ),
            )),
            _ => None,
        }
    }

    fn supply_volts(&self, part: usize, supply: &Option<String>) -> Option<f64> {
        let Some(name) = supply else { return Some(0.0) };
        let p = &self.sch.parts[part];
        let pin = p.symbol.pins.iter().position(|q| &q.number == name || &q.name == name)?;
        let net = self.sch.net_of(PinRef { part, pin })?;
        self.net_volts(net).map(|v| v.nominal)
    }

    fn thresholds(&self, net: usize) -> (Option<f64>, Option<f64>) {
        let mut vih: Option<f64> = None;
        let mut vil: Option<f64> = None;
        for r in &self.sch.nets[net].pins {
            let pin = &self.sch.parts[r.part].symbol.pins[r.pin];
            let Some(levels) = &pin.levels else { continue };
            let Some(supply) = self.supply_volts(r.part, &levels.supply) else { continue };
            if let Some(t) = levels.vih {
                vih = Some(vih.map_or(t.at(supply), |v| v.max(t.at(supply))));
            }
            if let Some(t) = levels.vil {
                vil = Some(vil.map_or(t.at(supply), |v| v.min(t.at(supply))));
            }
        }
        (vih.map(round), vil.map(round))
    }

    fn firmware_pins(&self, net: usize) -> Vec<String> {
        self.sch.nets[net]
            .pins
            .iter()
            .filter(|r| {
                let p = &self.sch.parts[r.part];
                let pin = &p.symbol.pins[r.pin];
                !p.dnp
                    && !is_test_point(self.sch, r.part)
                    && !p.reference.starts_with(['R', 'C', 'L', 'D', 'F', 'Q', 'T'])
                    && (matches!(
                        pin.kind,
                        PinType::Input
                            | PinType::Output
                            | PinType::Bidirectional
                            | PinType::TriState
                            | PinType::OpenCollector
                            | PinType::OpenEmitter
                    ) || (matches!(pin.kind, PinType::Unspecified | PinType::Passive)
                        && p.reference.starts_with(['U', 'J', 'P'])))
            })
            .map(|r| {
                let p = &self.sch.parts[r.part];
                let pin = &p.symbol.pins[r.pin];
                if pin.name.is_empty() {
                    format!("{}.{}", p.reference, pin.number)
                } else {
                    format!("{}.{} {}", p.reference, pin.number, pin.name)
                }
            })
            .collect()
    }

    fn sim_states(&self, net: usize) -> (Vec<LogicState>, Vec<Source>) {
        let mut states = Vec::new();
        let mut sources = Vec::new();
        for m in &self.models.logic {
            let before = states.len();
            for e in &m.expects {
                let Some(bit) = e.names.iter().position(|n| self.net_matches(net, n)) else {
                    continue;
                };
                match &e.check {
                    Check::At { time, value } => {
                        if let Some(Some(l)) = value.get(bit) {
                            states.push(LogicState {
                                when: format!("{} at {}", m.sim, crate::logic::fmt_time(*time)),
                                level: level_text(*l),
                            });
                        }
                    }
                    Check::Sequence { clock_name, edge, values, .. } => {
                        for (i, v) in values.iter().enumerate() {
                            if let Some(Some(l)) = v.get(bit) {
                                states.push(LogicState {
                                    when: format!(
                                        "{} before {} edge {} of {}",
                                        m.sim,
                                        format!("{edge:?}").to_lowercase(),
                                        i + 1,
                                        clock_name
                                    ),
                                    level: level_text(*l),
                                });
                            }
                        }
                    }
                }
            }
            if states.len() > before {
                sources.push(Source {
                    kind: "simulation".into(),
                    confidence: if m.verified { "high" } else { "stale" }.into(),
                    detail: format!(
                        "{} [[expect]] levels{}",
                        m.sim,
                        if m.verified {
                            ""
                        } else {
                            "; the run is stale, failing or missing, run `agentee sim`"
                        }
                    ),
                });
            }
        }
        states.truncate(MAX_STATES);
        (states, sources)
    }

    fn digital(&self, net: usize, row: &mut Row) -> bool {
        let (states, mut sources) = self.sim_states(net);
        let pull = self.pull(net);
        let pins = self.firmware_pins(net);
        if states.is_empty() && pull.is_none() {
            if pins.is_empty() {
                return false;
            }
            let shown: Vec<&str> = pins.iter().take(6).map(String::as_str).collect();
            let more = pins.len().saturating_sub(shown.len());
            row.order = 90;
            row.measure = "logic level".into();
            row.expect = Expect::Unknown(FIRMWARE.into());
            row.reason = Some(format!(
                "level is set by firmware or the connected device; pins: {}{}",
                shown.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            ));
            row.conditions.push("firmware state decides the level".into());
            return true;
        }
        let (vih, vil) = self.thresholds(net);
        let mut basis = Vec::new();
        let mut idle = None;
        let mut idle_volts = None;
        if let Some((high, v, text)) = &pull {
            idle = Some(if *high { "1" } else { "0" }.to_string());
            idle_volts = Some([round(v.min), round(v.max)]);
            basis.push(format!("idle {} from the {text}, nothing driving", if *high { "high" } else { "low" }));
            sources.push(Source {
                kind: "arithmetic".into(),
                confidence: "medium".into(),
                detail: format!("{text}; input leakage ignored"),
            });
            row.conditions.push("bus idle, no device driving the net".into());
        }
        if !states.is_empty() {
            basis.push(format!("{} simulated level(s)", states.len()));
            row.conditions.push("stimulus as in the logic simulation".into());
        }
        if vih.is_some() || vil.is_some() {
            basis.push("thresholds from the receiving pins' levels".into());
            sources.push(Source {
                kind: "datasheet".into(),
                confidence: "high".into(),
                detail: format!(
                    "receiver V_IH {} V, V_IL {} V from the pin levels in the symbol",
                    vih.map_or("-".into(), |v| v.to_string()),
                    vil.map_or("-".into(), |v| v.to_string())
                ),
            });
        }
        row.order = 90;
        row.measure = "logic level".into();
        row.expect = Expect::Logic(Logic {
            states,
            idle,
            idle_volts,
            vih,
            vil,
            basis: basis.join("; "),
        });
        row.source = sources;
        true
    }

    fn fill_volts(&self, row: &mut Row, k: &Known) {
        row.measure = "DC voltage against ground".into();
        row.order = k.order + 1;
        row.expect = Expect::Reading(Reading {
            unit: "V".into(),
            nominal: Some(round(k.volts.nominal)),
            min: round(k.volts.min),
            max: round(k.volts.max),
            basis: k.basis.clone(),
        });
        row.conditions.extend(k.conditions.iter().cloned());
        row.source = k.sources.clone();
    }
}

fn role_of(name: &str, modelled_rail: bool, rf: bool) -> String {
    if is_ground(name) {
        return "ground".into();
    }
    if rf {
        return "RF".into();
    }
    let up = name.to_ascii_uppercase();
    let leaf = up.rsplit('/').next().unwrap_or(&up);
    let has = |keys: &[&str]| leaf.split(['_', '-']).any(|t| keys.iter().any(|k| t.contains(k)));
    if modelled_rail || is_rail(name) {
        "power rail".into()
    } else if has(&["RST", "RESET", "NRST"]) {
        "reset".into()
    } else if has(&["CLK", "XTAL", "OSC", "REFCLK"]) {
        "clock".into()
    } else if has(&["RF", "LNA", "ANT"]) {
        "RF".into()
    } else if has(&["ADC", "DAC", "SENSE", "SNS", "VREF", "BIAS"])
        || leaf.contains("BB.")
        || leaf.contains("BB_") {
        "analog".into()
    } else if has(&[
        "SCL", "SDA", "SDI", "SDO", "SCK", "MOSI", "MISO", "CS", "TX", "RX", "SWD", "TCK", "TMS",
        "TDI", "TDO",
    ]) {
        "digital bus".into()
    } else {
        "signal".into()
    }
}

fn measure_for(role: &str) -> String {
    match role {
        "clock" => "frequency and waveform".into(),
        "reset" | "digital bus" | "signal" => "logic level".into(),
        "RF" => "none: do not probe with a high impedance probe".into(),
        _ => "DC voltage against ground".into(),
    }
}

fn nearest_reference(l: &Layout, reference: &str) -> Option<String> {
    let me = l.parts.iter().find(|q| q.reference == reference)?;
    let at = me.at.to_mm();
    l.parts
        .iter()
        .filter(|q| {
            let r = q.reference.as_str();
            let big = q.pads.len() >= 8 && r.starts_with('U');
            big || r.starts_with('J')
                || r.starts_with('P') && q.pads.len() >= 2 && !r.starts_with("TP")
        })
        .map(|q| {
            let c = q.at.to_mm();
            (((c[0] - at[0]).powi(2) + (c[1] - at[1]).powi(2)).sqrt(), q)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(d, q)| format!("{} ({:.1} mm)", q.reference, d))
}

pub fn build(sch: &Schematic, layout: Option<&Layout>, board: Option<&Board>) -> Plan {
    build_with(sch, layout, board, &Models::default())
}

fn tp_of_net(sch: &Schematic, net: usize) -> Vec<String> {
    let mut out: Vec<String> = sch.nets[net]
        .pins
        .iter()
        .filter(|r| is_test_point(sch, r.part) && !sch.parts[r.part].dnp)
        .map(|r| sch.parts[r.part].reference.clone())
        .collect();
    out.sort_by(|a, b| crate::footprint::natural_cmp(a, b));
    out.dedup();
    out
}

fn assumed(detail: String) -> Source {
    Source { kind: "assumed".into(), confidence: "assumed".into(), detail }
}

fn unpowered(b: &Builder) -> Vec<Unpowered> {
    let sch = b.sch;
    let mut out = Vec::new();
    let ground: Option<(usize, String)> = (0..sch.nets.len())
        .filter(|n| b.ground(*n))
        .find_map(|n| tp_of_net(sch, n).into_iter().next().map(|t| (n, t)));
    let Some((gnet, gtp)) = ground else {
        return out;
    };
    for net in 0..sch.nets.len() {
        if b.ground(net) {
            continue;
        }
        let tps = tp_of_net(sch, net);
        let Some(first) = tps.first() else { continue };
        let name = &sch.nets[net].name;
        let rail = b.known.contains_key(&net) || is_rail(name);
        if rail {
            out.push(Unpowered {
                kind: "resistance to ground".into(),
                a: first.clone(),
                b: gtp.clone(),
                a_net: name.clone(),
                b_net: sch.nets[gnet].name.clone(),
                measure: "resistance with power off and the supply disconnected".into(),
                unit: "ohm".into(),
                min: ASSUMED_SHORT_OHMS,
                max: None,
                basis: format!(
                    "{name} must not read near 0 ohm (a short to ground); decoupling capacitors make the reading climb while the meter charges them, so read after it settles"
                ),
                source: vec![assumed(format!(
                    "{ASSUMED_SHORT_OHMS} ohm short threshold; measure a known-good board for the expected value"
                ))],
            });
        }
        for other in &tps[1..] {
            out.push(Unpowered {
                kind: "continuity".into(),
                a: first.clone(),
                b: other.clone(),
                a_net: name.clone(),
                b_net: name.clone(),
                measure: "resistance with power off".into(),
                unit: "ohm".into(),
                min: 0.0,
                max: Some(ASSUMED_CONTINUITY_OHMS),
                basis: format!("{first} and {other} are the same net {name} on the schematic"),
                source: vec![
                    Source {
                        kind: "schematic".into(),
                        confidence: "high".into(),
                        detail: format!("{first} and {other} share net {name}"),
                    },
                    assumed(format!(
                        "{ASSUMED_CONTINUITY_OHMS} ohm limit for meter leads and contact resistance"
                    )),
                ],
            });
        }
    }
    for part in 0..sch.parts.len() {
        let p = &sch.parts[part];
        if p.dnp || p.symbol.reference != "D" || p.symbol.pins.len() != 2 {
            continue;
        }
        let find = |names: &[&str]| {
            p.symbol.pins.iter().position(|q| {
                let n = q.name.to_ascii_uppercase();
                names.iter().any(|k| n == *k)
            })
        };
        let (Some(a), Some(k)) =
            (find(&["A", "A1", "ANODE", "+"]), find(&["K", "C", "K1", "CATHODE", "-"]))
        else {
            continue;
        };
        let (Some(an), Some(kn)) = (
            sch.net_of(PinRef { part, pin: a }),
            sch.net_of(PinRef { part, pin: k }),
        ) else {
            continue;
        };
        let signal = if b.ground(kn) {
            an
        } else if b.ground(an) {
            kn
        } else {
            continue;
        };
        let Some(tp) = tp_of_net(sch, signal).into_iter().next() else { continue };
        let (red, black, red_net, black_net) = if signal == an {
            (tp, gtp.clone(), sch.nets[an].name.clone(), sch.nets[kn].name.clone())
        } else {
            (gtp.clone(), tp, sch.nets[an].name.clone(), sch.nets[kn].name.clone())
        };
        let declared = p
            .fields
            .get("vf")
            .or_else(|| p.fields.get("forward_voltage"))
            .and_then(|v| crate::circuit::si(v, &["v"]));
        let (min, max, source) = match declared {
            Some(vf) => (
                round(vf * 0.8),
                round(vf * 1.2),
                vec![Source {
                    kind: "part field".into(),
                    confidence: "declared".into(),
                    detail: format!(
                        "{} ({}) forward voltage {vf} V, +-20% for the meter's test current",
                        p.reference,
                        mpn(sch, part)
                    ),
                }],
            ),
            None => (
                ASSUMED_DIODE_DROP[0],
                ASSUMED_DIODE_DROP[1],
                vec![assumed(format!(
                    "{}-{} V covers a Schottky to a silicon diode at a meter's test current; set `vf` on {} to tighten it",
                    ASSUMED_DIODE_DROP[0], ASSUMED_DIODE_DROP[1], p.reference
                ))],
            ),
        };
        out.push(Unpowered {
            kind: "diode drop".into(),
            a: red,
            b: black,
            a_net: red_net,
            b_net: black_net,
            measure: format!(
                "meter in diode mode, red lead on the anode side, across {}",
                p.reference
            ),
            unit: "V".into(),
            min,
            max: Some(max),
            basis: format!(
                "forward drop of protection diode {} {}; the reverse reading should be open",
                p.reference, p.value
            ),
            source,
        });
    }
    out
}

pub fn build_with(
    sch: &Schematic,
    layout: Option<&Layout>,
    board: Option<&Board>,
    models: &Models,
) -> Plan {
    let mut b = Builder { sch, board, models, known: HashMap::new(), reasons: HashMap::new() };
    b.regulators();
    b.declared();
    let mut rows: Vec<Row> =
        (0..sch.parts.len()).filter(|i| is_test_point(sch, *i)).map(|i| b.row(i, layout)).collect();
    rows.sort_by(|a, c| {
        a.order
            .cmp(&c.order)
            .then_with(|| crate::footprint::natural_cmp(&a.reference, &c.reference))
    });
    let checks = unpowered(&b);
    let mut plan = Plan {
        design: sch.name.clone(),
        hash: hash_of(sch),
        summary: String::new(),
        rows,
        checks,
    };
    plan.summary = format!(
        "{} test points, {} unknown, {} assumed, {} firmware, {} unpowered checks",
        plan.rows.len(),
        plan.unknown(),
        plan.assumed() + plan.unpowered_assumed(),
        plan.firmware(),
        plan.checks.len()
    );
    plan
}

fn cell(v: &str) -> String {
    if v.contains([',', '"', '\n']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() }
}

fn expectation(r: &Row) -> String {
    match &r.expect {
        Expect::Unknown(_) => "unknown".into(),
        Expect::Reading(x) => match x.nominal {
            Some(n) => format!("{} {} ({} to {})", n, x.unit, x.min, x.max),
            None => format!("{} to {} {}", x.min, x.max, x.unit),
        },
    }
}

fn place(r: &Row) -> String {
    match &r.location {
        Some(l) => format!("{:.1}, {:.1} {}", l.x, l.y, l.side),
        None => String::new(),
    }
}

impl Plan {
    pub fn to_csv(&self) -> String {
        let mut out = String::from(
            "order,ref,net,role,x,y,side,nearest,measure,unit,nominal,min,max,expect,source,conditions,measured\n",
        );
        for r in &self.rows {
            let (unit, nominal, min, max) = match &r.expect {
                Expect::Reading(x) => (
                    x.unit.clone(),
                    x.nominal.map(|n| n.to_string()).unwrap_or_default(),
                    x.min.to_string(),
                    x.max.to_string(),
                ),
                Expect::Unknown(_) => Default::default(),
            };
            let (x, y, side) = match &r.location {
                Some(l) => (l.x.to_string(), l.y.to_string(), l.side.clone()),
                None => Default::default(),
            };
            let source =
                r.source.iter().map(|s| format!("{} ({})", s.kind, s.confidence)).collect::<Vec<_>>();
            let expect = match &r.expect {
                Expect::Unknown(_) => format!("unknown: {}", r.reason.clone().unwrap_or_default()),
                Expect::Reading(_) => expectation(r),
            };
            let cols = [
                r.order.to_string(),
                r.reference.clone(),
                r.net.clone(),
                r.role.clone(),
                x,
                y,
                side,
                r.nearest.clone().unwrap_or_default(),
                r.measure.clone(),
                unit,
                nominal,
                min,
                max,
                expect,
                source.join("; "),
                r.conditions.join("; "),
                String::new(),
            ];
            out += &cols.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",");
            out.push('\n');
        }
        out
    }

    pub fn to_markdown(&self) -> String {
        let mut out = format!("# Test plan: {}\n\n{}\n\n", self.design, self.summary);
        let off: Vec<&Row> = self.rows.iter().filter(|r| !r.unpowered.is_empty()).collect();
        if !off.is_empty() {
            out += "## Power off\n\n| Done | Pad | Net | Check | Measured |\n|---|---|---|---|---|\n";
            for r in off {
                for u in &r.unpowered {
                    out += &format!("| [ ] | {} | {} | {} | |\n", r.reference, r.net, u);
                }
            }
            out.push('\n');
        }
        out += "## Power up\n\nApply power in this order; check each pad before the rails that depend on it.\n\n";
        out += "| Done | Pad | Net | Where | Measure | Expect | Conditions | Measured |\n|---|---|---|---|---|---|---|---|\n";
        for r in &self.rows {
            let near = r.nearest.as_deref().map(|n| format!(", near {n}")).unwrap_or_default();
            let expect = match &r.expect {
                Expect::Unknown(_) => {
                    format!("unknown: {}", r.reason.as_deref().unwrap_or("no model"))
                }
                Expect::Reading(_) => expectation(r),
            };
            let flag = if r.is_assumed() { " (assumed, check a known good board)" } else { "" };
            out += &format!(
                "| [ ] | {} | {} | {}{} | {} | {}{} | {} | |\n",
                r.reference,
                r.net,
                place(r),
                near,
                r.measure,
                expect,
                flag,
                r.conditions.join("; ")
            );
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Fail,
    NoExpectation,
    NotMeasured,
}

#[derive(Clone, Debug)]
pub struct Compared {
    pub reference: String,
    pub net: String,
    pub verdict: Verdict,
    pub measured: Option<f64>,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub off_by: f64,
    pub from_nominal: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Comparison {
    pub rows: Vec<Compared>,
    pub unmatched: Vec<String>,
}

impl Comparison {
    pub fn failed(&self) -> usize {
        self.rows.iter().filter(|r| r.verdict == Verdict::Fail).count()
    }

    pub fn passed(&self) -> usize {
        self.rows.iter().filter(|r| r.verdict == Verdict::Pass).count()
    }

    pub fn ok(&self) -> bool {
        self.failed() == 0 && self.unmatched.is_empty()
    }

    pub fn summary(&self) -> String {
        let count = |v: Verdict| self.rows.iter().filter(|r| r.verdict == v).count();
        format!(
            "{} pass, {} fail, {} not measured, {} without an expectation",
            self.passed(),
            self.failed(),
            count(Verdict::NotMeasured),
            count(Verdict::NoExpectation)
        )
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for r in &self.rows {
            let line = match (&r.verdict, r.measured) {
                (Verdict::Pass, Some(m)) => {
                    format!("pass {} {}: {} {} within {} to {}", r.reference, r.net, m, r.unit, r.min, r.max)
                }
                (Verdict::Fail, Some(m)) => format!(
                    "FAIL {} {}: {} {} is {} {} {} the {} to {} range",
                    r.reference,
                    r.net,
                    m,
                    r.unit,
                    round(r.off_by.abs()),
                    r.unit,
                    if r.off_by > 0.0 { "above" } else { "below" },
                    r.min,
                    r.max
                ),
                (Verdict::NoExpectation, Some(m)) => {
                    format!("skip {} {}: {} measured, the plan has no expectation", r.reference, r.net, m)
                }
                _ => format!("skip {} {}: not measured", r.reference, r.net),
            };
            out += &line;
            out.push('\n');
        }
        for u in &self.unmatched {
            out += &format!("FAIL {u}: measured but not in the plan\n");
        }
        out += &format!("{}\n", self.summary());
        out
    }
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn parse_measured(csv: &str) -> Result<Vec<(String, Option<f64>)>, String> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let header = split_csv_line(lines.next().ok_or("the measured file is empty")?);
    let col = |name: &str| header.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let (Some(r), Some(m)) = (col("ref"), col("measured")) else {
        return Err("the measured file needs `ref` and `measured` columns in its first line".into());
    };
    let mut out = Vec::new();
    for (i, line) in lines.enumerate() {
        let cells = split_csv_line(line);
        let reference = cells.get(r).map(|c| c.trim()).unwrap_or_default();
        if reference.is_empty() {
            continue;
        }
        let raw = cells.get(m).map(|c| c.trim()).unwrap_or_default();
        let value = if raw.is_empty() {
            None
        } else {
            Some(
                raw.parse::<f64>()
                    .map_err(|_| format!("line {}: `{raw}` for {reference} is not a number", i + 2))?,
            )
        };
        out.push((reference.to_string(), value));
    }
    Ok(out)
}

impl Plan {
    pub fn compare(&self, measured: &[(String, Option<f64>)]) -> Comparison {
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let value = measured
                    .iter()
                    .rev()
                    .find(|(r, _)| r.eq_ignore_ascii_case(&row.reference))
                    .and_then(|(_, v)| *v);
                let mut c = Compared {
                    reference: row.reference.clone(),
                    net: row.net.clone(),
                    verdict: Verdict::NotMeasured,
                    measured: value,
                    unit: String::new(),
                    min: 0.0,
                    max: 0.0,
                    off_by: 0.0,
                    from_nominal: None,
                };
                match (&row.expect, value) {
                    (Expect::Unknown(_), Some(_)) => c.verdict = Verdict::NoExpectation,
                    (Expect::Unknown(_), None) | (_, None) => {
                        if let Expect::Reading(x) = &row.expect {
                            c.unit = x.unit.clone();
                            c.min = x.min;
                            c.max = x.max;
                        }
                    }
                    (Expect::Reading(x), Some(m)) => {
                        c.unit = x.unit.clone();
                        c.min = x.min;
                        c.max = x.max;
                        c.from_nominal = x.nominal.map(|n| round(m - n));
                        c.off_by = if m < x.min {
                            round(m - x.min)
                        } else if m > x.max {
                            round(m - x.max)
                        } else {
                            0.0
                        };
                        c.verdict = if c.off_by == 0.0 { Verdict::Pass } else { Verdict::Fail };
                    }
                }
                c
            })
            .collect();
        let unmatched = measured
            .iter()
            .filter(|(r, v)| v.is_some() && !self.rows.iter().any(|x| x.reference.eq_ignore_ascii_case(r)))
            .map(|(r, _)| r.clone())
            .collect();
        Comparison { rows, unmatched }
    }
}

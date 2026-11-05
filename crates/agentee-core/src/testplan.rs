use crate::board::Board;
use crate::circuit::is_ground;
use crate::layout::Layout;
use crate::logic::netlist_hash;
use crate::schematic::{PinRef, Schematic};
use crate::testpoint::{is_rail, rail_volts};
use crate::units::Percent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const EXT: &str = ".testplan.toml";
pub const DEFAULT_RESISTOR_TOLERANCE: f64 = 0.01;
pub const ASSUMED_RAIL_TOLERANCE: f64 = 0.05;
pub const ASSUMED_GROUND_BAND: f64 = 0.01;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub design: String,
    pub hash: String,
    pub summary: String,
    #[serde(default, rename = "point")]
    pub rows: Vec<Row>,
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
        matches!(self.expect, Expect::Unknown(_))
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

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn is_test_point(sch: &Schematic, part: usize) -> bool {
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
            self.known.insert(
                net,
                Known {
                    volts: Volts {
                        nominal: (range[0] + range[1]) / 2.0,
                        min: range[0],
                        max: range[1],
                    },
                    basis: format!("rail range declared in the schematic `rails` table for {name}"),
                    sources: vec![Source {
                        kind: "schematic".into(),
                        confidence: "declared".into(),
                        detail: format!("rails.{name} = [{}, {}]", range[0], range[1]),
                    }],
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
        row.order = 100;
        row.measure = measure_for(&row.role);
        row.reason = Some(match row.role.as_str() {
            "power rail" => {
                format!("no regulator model drives {name} and its name carries no voltage")
            }
            "RF" => "RF net: a probe loads it, no level is modelled".into(),
            _ => "no model for this net: no regulator, divider, shunt or logic expectation".into(),
        });
        row.expect = Expect::Unknown("unknown".into());
        row
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
    } else if has(&["ADC", "SENSE", "SNS", "VREF", "BIAS"]) {
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
    let mut b = Builder { sch, board, known: HashMap::new(), reasons: HashMap::new() };
    b.regulators();
    b.declared();
    let mut rows: Vec<Row> =
        (0..sch.parts.len()).filter(|i| is_test_point(sch, *i)).map(|i| b.row(i, layout)).collect();
    rows.sort_by(|a, c| {
        a.order
            .cmp(&c.order)
            .then_with(|| crate::footprint::natural_cmp(&a.reference, &c.reference))
    });
    let mut plan =
        Plan { design: sch.name.clone(), hash: hash_of(sch), summary: String::new(), rows };
    plan.summary = format!(
        "{} test points, {} unknown, {} assumed",
        plan.rows.len(),
        plan.unknown(),
        plan.assumed()
    );
    plan
}

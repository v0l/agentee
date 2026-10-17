use crate::diag::Diags;
use crate::schematic::{PinRef, Schematic};
use crate::sim::SimFile;
use serde::{Deserialize, Serialize};

pub use sims_circuit::{
    At, Check, CircuitResult, CircuitSpec, Curve, Drive, Elem, ElemKind, Measure, Node, Noise,
    Reading, Surge, Sweep, Wave, fmt_si,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Quantity {
    Number(f64),
    Text(String),
}

impl Quantity {
    pub fn value(&self, units: &[&str]) -> Option<f64> {
        match self {
            Quantity::Number(v) => Some(*v),
            Quantity::Text(s) => si(s, units),
        }
    }
}

pub fn si(s: &str, units: &[&str]) -> Option<f64> {
    let s = s.trim().replace('µ', "u").replace('Ω', "ohm");
    let lower = s.to_ascii_lowercase();
    let mut body = s.as_str();
    for u in units {
        if lower.ends_with(&u.to_ascii_lowercase()) && lower.len() > u.len() {
            body = &s[..s.len() - u.len()];
            break;
        }
    }
    let body = body.trim();
    if let Ok(v) = body.parse::<f64>() {
        return Some(v);
    }
    if let Some(n) = body.strip_suffix("meg").or_else(|| body.strip_suffix("Meg")) {
        return n.trim().parse::<f64>().ok().map(|v| v * 1e6);
    }
    let last = body.chars().last()?;
    let mult = match last {
        'f' => 1e-15,
        'p' => 1e-12,
        'n' => 1e-9,
        'u' => 1e-6,
        'm' => 1e-3,
        'k' | 'K' => 1e3,
        'M' => 1e6,
        'G' => 1e9,
        _ => return crate::sim::parse_value(body),
    };
    body[..body.len() - last.len_utf8()].trim().parse::<f64>().ok().map(|v| v * mult)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PulseFile {
    pub low: Quantity,
    pub high: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rise: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fall: Option<Quantity>,
    pub width: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Quantity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SineFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Quantity>,
    pub amplitude: Quantity,
    pub frequency: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Quantity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurgeFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Quantity>,
    pub peak: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurstFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Quantity>,
    pub peak: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Quantity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveFile {
    pub net: String,
    #[serde(default, rename = "return", skip_serializing_if = "Option::is_none")]
    pub ret: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dc: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ac: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistance: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<PulseFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sine: Option<SineFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surge: Option<SurgeFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burst: Option<BurstFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<(Quantity, Quantity)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepFile {
    pub from: Quantity,
    pub to: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranFile {
    pub stop: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Quantity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseFile {
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    pub from: Quantity,
    pub to: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockFile {
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistance: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacitance: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inductance: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gbw: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rout: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vos: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub en: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headroom: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headroom_high: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headroom_low: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slew: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ib: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psrr: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fc: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vout: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropout: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bv: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cj: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bf: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub br: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub va: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cje: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cjc: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vth: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lambda: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgs: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgd: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v1ma: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<Quantity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trough: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rms: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harmonics: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mains: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Quantity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Quantity>,
}

pub fn circuit_hash(src: u64, netlist: u64) -> u64 {
    crate::sim::cascade_hash(src, netlist ^ 0x6369_7263, &[])
}

struct Uf(Vec<usize>);

impl Uf {
    fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = i;
        while self.0[c] != r {
            let n = self.0[c];
            self.0[c] = r;
            c = n;
        }
        r
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}

pub fn is_ground(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    !u.contains("PWR_FLAG")
        && (u.contains("GND") || u == "0V" || u == "GROUND" || u.starts_with("VSS"))
}

struct Pin {
    number: String,
    name: String,
    unit: u32,
    output: bool,
    net: Option<usize>,
}

struct Ref {
    reference: String,
    value: String,
    symbol: String,
    about: String,
    prefix: String,
    power: bool,
    pins: Vec<Pin>,
}

impl Ref {
    fn by(&self, key: &str) -> Option<&Pin> {
        self.pins
            .iter()
            .find(|p| p.number == key)
            .or_else(|| self.pins.iter().find(|p| p.name.eq_ignore_ascii_case(key)))
    }

    fn named(&self, names: &[&str]) -> Option<&Pin> {
        names.iter().find_map(|n| self.pins.iter().find(|p| clean(&p.name) == clean(n)))
    }

    fn label(&self) -> String {
        if self.value.is_empty() || self.value == self.reference {
            self.reference.clone()
        } else {
            format!("{} ({})", self.reference, self.value)
        }
    }
}

fn clean(s: &str) -> String {
    s.trim_start_matches("~{").trim_end_matches('}').to_ascii_uppercase().replace(['_', ' '], "")
}

fn prefix(r: &str) -> String {
    r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase()
}

const IGNORED: &[&str] =
    &["J", "P", "TP", "H", "MH", "FID", "Y", "X", "SW", "BT", "LS", "ANT", "AE", "T"];

fn zener_volts(value: &str) -> Option<f64> {
    let u = value.to_ascii_uppercase();
    let b = u.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'V' && i > 0 && b[i - 1].is_ascii_digit() {
            let start = (0..i).rev().take_while(|&k| b[k].is_ascii_digit()).last()?;
            let whole: f64 = u[start..i].parse().ok()?;
            let frac: String = u[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            let f = if frac.is_empty() { 0.0 } else { format!("0.{frac}").parse().ok()? };
            return Some(whole + f);
        }
    }
    None
}

struct Ctx<'a> {
    d: &'a mut Diags,
    node: Vec<Node>,
}

impl Ctx<'_> {
    fn q(
        &mut self,
        v: &Option<Quantity>,
        units: &[&str],
        at: &str,
        field: &str,
        default: f64,
    ) -> f64 {
        match v {
            None => default,
            Some(q) => match q.value(units) {
                Some(x) => x,
                None => {
                    self.d.error(at, format!("cannot read `{field}` {q:?} as a number"));
                    default
                }
            },
        }
    }

    fn opt(&mut self, v: &Option<Quantity>, units: &[&str], at: &str, field: &str) -> Option<f64> {
        v.as_ref().map(|_| self.q(v, units, at, field, 0.0))
    }

    fn pin_node(&mut self, r: &Ref, p: Option<&Pin>, role: &str, at: &str) -> Option<Node> {
        match p {
            Some(p) => Some(p.net.and_then(|n| self.node[n])),
            None => {
                self.d.error(
                    at,
                    format!("{} has no pin for {role}; name it in the block's `pins`", r.label()),
                );
                None
            }
        }
    }
}

fn block_pin<'a>(r: &'a Ref, b: Option<&BlockFile>, role: &str, names: &[&str]) -> Option<&'a Pin> {
    match b.and_then(|b| b.pins.get(role)) {
        Some(k) => r.by(k),
        None => r.named(names),
    }
}

pub fn resolve(file: &SimFile, sch: &Schematic, d: &mut Diags) -> CircuitSpec {
    let mut pin_net: HashMap<PinRef, usize> = HashMap::new();
    for (i, n) in sch.nets.iter().enumerate() {
        for p in &n.pins {
            pin_net.insert(*p, i);
        }
    }
    let mut refs: Vec<Ref> = Vec::new();
    for (pi, part) in sch.parts.iter().enumerate().filter(|(_, p)| !p.dnp) {
        let k = match refs.iter().position(|r| r.reference == part.reference) {
            Some(k) => k,
            None => {
                refs.push(Ref {
                    reference: part.reference.clone(),
                    value: part.value.clone(),
                    symbol: part.symbol_name.clone(),
                    about: format!("{} {}", part.symbol.description, part.symbol.keywords.join(" ")),
                    prefix: part.symbol.reference.to_ascii_uppercase(),
                    power: part.symbol.power,
                    pins: Vec::new(),
                });
                refs.len() - 1
            }
        };
        for (ni, pin) in part.pins() {
            let net = pin_net.get(&PinRef { part: pi, pin: ni }).copied();
            match refs[k].pins.iter_mut().find(|q| q.number == pin.number) {
                Some(q) => q.net = q.net.or(net),
                None => refs[k].pins.push(Pin {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    unit: part.unit,
                    output: pin.kind == crate::symbol::PinType::Output,
                    net,
                }),
            }
        }
    }
    for r in &mut refs {
        if r.prefix.is_empty() {
            r.prefix = prefix(&r.reference);
        }
    }
    let blocks: Vec<(usize, &BlockFile)> = file.blocks.iter().enumerate().collect();
    let block_of = |r: &Ref| -> Option<(usize, &BlockFile)> {
        blocks
            .iter()
            .find(|(_, b)| b.reference.as_deref() == Some(r.reference.as_str()))
            .or_else(|| {
                blocks.iter().find(|(_, b)| {
                    b.reference.is_none() && b.value.as_deref() == Some(r.value.as_str())
                })
            })
            .copied()
    };
    for (i, b) in &blocks {
        let at = format!("blocks[{i}]");
        match (&b.reference, &b.value) {
            (None, None) => d.error(&at, "a block needs `ref` or `value`"),
            (Some(r), _) if !refs.iter().any(|x| &x.reference == r) => {
                d.error(&at, format!("no part `{r}` in the schematic"))
            }
            (None, Some(v)) if !refs.iter().any(|x| &x.value == v) => {
                d.error(&at, format!("no part has value `{v}`"))
            }
            (Some(r), _) if blocks[..*i].iter().any(|(_, o)| o.reference.as_ref() == Some(r)) => d
                .error(
                    &at,
                    format!("`{r}` already has a block; one block covers every unit of a part"),
                ),
            _ => {}
        }
    }
    let ignored = |r: &Ref| file.ignore.iter().any(|g| crate::layout::glob(g, &r.reference));

    let n = sch.nets.len();
    let mut uf = Uf((0..n).collect());
    let shorted = |r: &Ref| -> bool {
        let p = r.prefix.as_str();
        let zero = p == "R" && crate::sim::parse_value(&r.value).is_some_and(|v| v == 0.0);
        zero || p == "JP" || p == "SJ" || p == "NT"
    };
    for r in refs.iter().filter(|r| shorted(r) && block_of(r).is_none() && !ignored(r)) {
        let nets: Vec<usize> = r.pins.iter().filter_map(|p| p.net).collect();
        for w in nets.windows(2) {
            uf.union(w[0], w[1]);
        }
    }
    let driven: Vec<String> = file.drives.iter().map(|x| x.net.clone()).collect();
    for g in &file.ground {
        if !sch.nets.iter().any(|x| &x.name == g) {
            d.error("ground", format!("net `{g}` is not in the schematic"));
        } else if driven.contains(g) {
            d.error("ground", format!("`{g}` is the ground, so a drive cannot set it"));
        }
    }
    let mut ground = vec![false; n];
    for (i, net) in sch.nets.iter().enumerate() {
        let named = file.ground.contains(&net.name);
        if (named || is_ground(&net.name)) && !driven.contains(&net.name) {
            ground[uf.find(i)] = true;
        }
    }
    let mut names: Vec<String> = Vec::new();
    let mut root_node: HashMap<usize, usize> = HashMap::new();
    let mut node: Vec<Node> = vec![None; n];
    for (i, slot) in node.iter_mut().enumerate() {
        let r = uf.find(i);
        if ground[r] {
            continue;
        }
        let k = *root_node.entry(r).or_insert_with(|| {
            names.push(sch.nets[r].name.clone());
            names.len() - 1
        });
        *slot = Some(k);
    }
    if !ground.iter().any(|g| *g) {
        d.error(
            "schematic",
            "no ground net (a net named GND, AGND, 0V or VSS, or `ground = \"NET\"`) to reference the circuit to",
        );
    }
    let net_named = |name: &str| sch.nets.iter().position(|x| x.name == name);
    let mut cx = Ctx { d, node };
    let mut elements: Vec<Elem> = Vec::new();

    for r in &refs {
        if r.power || shorted(r) && block_of(r).is_none() {
            continue;
        }
        let block = block_of(r);
        if ignored(r) && block.is_none() {
            continue;
        }
        let at = match block {
            Some((i, _)) => format!("blocks[{i}]"),
            None => format!("part {}", r.reference),
        };
        let b = block.map(|x| x.1);
        let kind = b.map(|b| b.kind.to_ascii_lowercase());
        let two = |cx: &mut Ctx| -> Option<Vec<Node>> {
            let a = block_pin(r, b, "1", &["1"]).or_else(|| r.pins.first());
            let c = block_pin(r, b, "2", &["2"]).or_else(|| r.pins.get(1));
            Some(vec![cx.pin_node(r, a, "pin 1", &at)?, cx.pin_node(r, c, "pin 2", &at)?])
        };
        let value = |units: &[&str]| -> Option<f64> {
            si(&r.value, units).or_else(|| crate::sim::parse_value(&r.value))
        };
        let push = |elements: &mut Vec<Elem>, name: String, kind: ElemKind, nodes: Vec<Node>| {
            elements.push(Elem { name, kind, nodes });
        };
        let p = r.prefix.as_str();
        let chosen = kind.as_deref().unwrap_or(match p {
            "R" | "RV" => "resistor",
            "C" => "capacitor",
            "L" => "inductor",
            "FB" => "bead",
            "D" | "LED" | "DZ" | "ZD" => "diode",
            "Q" | "M" | "T" if r.named(&["G"]).is_some() => "mosfet",
            "Q" | "T" if r.named(&["B"]).is_some() => "bjt",
            _ => "",
        });
        match chosen {
            "resistor" | "bead" => {
                let Some(nodes) = two(&mut cx) else { continue };
                let rv = match b.and_then(|b| b.resistance.clone()) {
                    Some(q) => cx.q(&Some(q), &["ohm", "r"], &at, "resistance", 0.0),
                    None if chosen == "bead" => 0.1,
                    None => match value(&["ohm", "r"]) {
                        Some(v) => v,
                        None => {
                            cx.d.error(&at, format!("cannot read {} as a resistance", r.label()));
                            continue;
                        }
                    },
                };
                push(&mut elements, r.reference.clone(), ElemKind::Resistor { r: rv.max(1e-6) }, nodes);
            }
            "varistor" | "mov" => {
                let Some(nodes) = two(&mut cx) else { continue };
                let v1ma = match b.and_then(|b| b.v1ma.clone()) {
                    Some(q) => cx.q(&Some(q), &["v"], &at, "v1ma", 0.0),
                    None => {
                        cx.d.error(&at, format!("a varistor block for {} needs `v1ma`", r.label()));
                        continue;
                    }
                };
                let kind = ElemKind::Varistor {
                    v1ma,
                    alpha: cx.q(&b.and_then(|b| b.alpha.clone()), &[], &at, "alpha", 30.0),
                    c: cx.q(&b.and_then(|b| b.capacitance.clone()), &["f"], &at, "capacitance", 0.0),
                };
                push(&mut elements, r.reference.clone(), kind, nodes);
            }
            "potentiometer" | "pot" => {
                let ends = [("1", "pin 1"), ("2", "the wiper (pin 2)"), ("3", "pin 3")];
                let mut nodes = Vec::new();
                for (role, what) in ends {
                    let pin = b.and_then(|b| b.pins.get(role)).and_then(|k| r.by(k)).or_else(|| r.by(role));
                    match cx.pin_node(r, pin, what, &at) {
                        Some(n) => nodes.push(n),
                        None => break,
                    }
                }
                if nodes.len() < 3 {
                    continue;
                }
                let total = match b.and_then(|b| b.resistance.clone()) {
                    Some(q) => cx.q(&Some(q), &["ohm", "r"], &at, "resistance", 0.0),
                    None => match value(&["ohm", "r"]) {
                        Some(v) => v,
                        None => {
                            cx.d.error(&at, format!("cannot read {} as a resistance", r.label()));
                            continue;
                        }
                    },
                };
                let pos = cx.q(&b.and_then(|b| b.position.clone()), &[], &at, "position", 0.5);
                if !(0.0..=1.0).contains(&pos) {
                    cx.d.error(&at, "`position` runs from 0 (wiper at pin 1) to 1 (wiper at pin 3)");
                    continue;
                }
                let floor = (total * 1e-6).max(1e-3);
                for (half, share, pair) in
                    [("a", pos, [nodes[0], nodes[1]]), ("b", 1.0 - pos, [nodes[1], nodes[2]])]
                {
                    push(
                        &mut elements,
                        format!("{}.{half}", r.reference),
                        ElemKind::Resistor { r: (total * share).max(floor) },
                        pair.to_vec(),
                    );
                }
            }
            "capacitor" => {
                let Some(nodes) = two(&mut cx) else { continue };
                let c = match b.and_then(|b| b.capacitance.clone()) {
                    Some(q) => cx.q(&Some(q), &["f"], &at, "capacitance", 0.0),
                    None => match value(&["f"]) {
                        Some(v) => v,
                        None => {
                            cx.d.error(&at, format!("cannot read {} as a capacitance", r.label()));
                            continue;
                        }
                    },
                };
                push(&mut elements, r.reference.clone(), ElemKind::Capacitor { c }, nodes);
            }
            "inductor" => {
                let Some(nodes) = two(&mut cx) else { continue };
                let l = match b.and_then(|b| b.inductance.clone()) {
                    Some(q) => cx.q(&Some(q), &["h"], &at, "inductance", 0.0),
                    None => match value(&["h"]) {
                        Some(v) => v,
                        None => {
                            cx.d.error(&at, format!("cannot read {} as an inductance", r.label()));
                            continue;
                        }
                    },
                };
                push(&mut elements, r.reference.clone(), ElemKind::Inductor { l: l.max(1e-15) }, nodes);
            }
            "diode" | "led" | "schottky" | "zener" => {
                let a = block_pin(r, b, "a", &["A", "ANODE", "+"]);
                let k = block_pin(r, b, "k", &["K", "CATHODE", "-"]);
                let (Some(na), Some(nk)) = (cx.pin_node(r, a, "the anode (A)", &at), cx.pin_node(r, k, "the cathode (K)", &at)) else {
                    continue;
                };
                let sym = format!("{} {}", r.symbol, r.value).to_ascii_uppercase();
                let led = chosen == "led" || p == "LED" || sym.contains("LED");
                let schottky = chosen == "schottky" || sym.contains("SCHOTTKY") || sym.contains("BAT") || sym.contains("SS1");
                let zener = chosen == "zener" || sym.contains("ZENER") || p == "DZ" || p == "ZD";
                let (is0, n0) = if led {
                    (1e-18, 2.0)
                } else if schottky {
                    (1e-8, 1.05)
                } else {
                    (1e-14, 1.0)
                };
                let bv = match b.and_then(|b| b.bv.clone()) {
                    Some(q) => Some(cx.q(&Some(q), &["v"], &at, "bv", 0.0)),
                    None if zener => {
                        let v = zener_volts(&r.value);
                        if v.is_none() {
                            cx.d.error(&at, format!("{} is a zener with no voltage in its value; give the block a `bv`", r.label()));
                        }
                        v
                    }
                    None => None,
                };
                let kind = ElemKind::Diode {
                    is: cx.q(&b.and_then(|b| b.is.clone()), &["a"], &at, "is", is0),
                    n: cx.q(&b.and_then(|b| b.n.clone()), &[], &at, "n", n0),
                    bv,
                    cj: cx.q(&b.and_then(|b| b.cj.clone()), &["f"], &at, "cj", 0.0),
                };
                push(&mut elements, r.reference.clone(), kind, vec![na, nk]);
            }
            "bjt" | "npn" | "pnp" => {
                let c = block_pin(r, b, "c", &["C", "COLLECTOR"]);
                let bb = block_pin(r, b, "b", &["B", "BASE"]);
                let e = block_pin(r, b, "e", &["E", "EMITTER"]);
                let (Some(nc), Some(nb), Some(ne)) = (
                    cx.pin_node(r, c, "the collector (C)", &at),
                    cx.pin_node(r, bb, "the base (B)", &at),
                    cx.pin_node(r, e, "the emitter (E)", &at),
                ) else {
                    continue;
                };
                let sym = format!("{} {} {}", r.symbol, r.value, r.about).to_ascii_uppercase();
                let pnp = chosen == "pnp" || (chosen == "bjt" && sym.contains("PNP"));
                let kind = ElemKind::Bjt {
                    pnp,
                    is: cx.q(&b.and_then(|b| b.is.clone()), &["a"], &at, "is", 1e-14),
                    bf: cx.q(&b.and_then(|b| b.bf.clone()), &[], &at, "bf", 100.0),
                    br: cx.q(&b.and_then(|b| b.br.clone()), &[], &at, "br", 1.0),
                    va: cx.opt(&b.and_then(|b| b.va.clone()), &["v"], &at, "va"),
                    cje: cx.q(&b.and_then(|b| b.cje.clone()), &["f"], &at, "cje", 0.0),
                    cjc: cx.q(&b.and_then(|b| b.cjc.clone()), &["f"], &at, "cjc", 0.0),
                };
                push(&mut elements, r.reference.clone(), kind, vec![nc, nb, ne]);
            }
            "mosfet" | "nmos" | "pmos" => {
                let dr = block_pin(r, b, "d", &["D", "DRAIN"]);
                let g = block_pin(r, b, "g", &["G", "GATE"]);
                let s = block_pin(r, b, "s", &["S", "SOURCE"]);
                let (Some(nd), Some(ng), Some(ns)) = (
                    cx.pin_node(r, dr, "the drain (D)", &at),
                    cx.pin_node(r, g, "the gate (G)", &at),
                    cx.pin_node(r, s, "the source (S)", &at),
                ) else {
                    continue;
                };
                let sym = format!("{} {} {}", r.symbol, r.value, r.about).to_ascii_uppercase();
                let pch = chosen == "pmos"
                    || (chosen == "mosfet"
                        && (sym.contains("PMOS") || sym.contains("P-CH")));
                let kind = ElemKind::Mosfet {
                    p: pch,
                    vth: cx.q(&b.and_then(|b| b.vth.clone()), &["v"], &at, "vth", 1.5).abs(),
                    k: cx.q(&b.and_then(|b| b.k.clone()), &[], &at, "k", 0.5),
                    lambda: cx.q(&b.and_then(|b| b.lambda.clone()), &[], &at, "lambda", 0.01),
                    cgs: cx.q(&b.and_then(|b| b.cgs.clone()), &["f"], &at, "cgs", 0.0),
                    cgd: cx.q(&b.and_then(|b| b.cgd.clone()), &["f"], &at, "cgd", 0.0),
                };
                push(&mut elements, r.reference.clone(), kind, vec![nd, ng, ns]);
            }
            "opamp" => {
                let vcc = block_pin(r, b, "vcc", &["V+", "VCC", "VDD", "VS+", "+VS", "V+IN"]);
                let vee = block_pin(r, b, "vee", &["V-", "VEE", "VSS", "VS-", "-VS", "GND"]);
                let vcc = vcc.map(|p| p.net.and_then(|n| cx.node[n]));
                let vee = vee.map(|p| p.net.and_then(|n| cx.node[n]));
                let mut units: Vec<u32> = r.pins.iter().map(|p| p.unit).collect();
                units.sort_unstable();
                units.dedup();
                let given = b.is_some_and(|b| b.pins.contains_key("out"));
                let mut found = 0;
                for u in units {
                    let only = |p: &&Pin| p.unit == u || given;
                    let pick = |role: &str, names: &[&str]| -> Option<&Pin> {
                        match b.and_then(|b| b.pins.get(role)) {
                            Some(k) => r.by(k),
                            None => names.iter().find_map(|n| r.pins.iter().filter(only).find(|p| clean(&p.name) == clean(n))),
                        }
                    };
                    let (Some(ip), Some(im), Some(o)) = (
                        pick("inp", &["+", "IN+", "+IN", "INP", "NONINV", "VIN+"]),
                        pick("inn", &["-", "IN-", "-IN", "INN", "INV", "VIN-"]),
                        pick("out", &["~", "OUT", "VOUT", "O", "OUTPUT"]).or_else(|| {
                            r.pins.iter().filter(only).find(|p| p.output && (p.name.is_empty() || p.name == "~"))
                        }),
                    ) else {
                        continue;
                    };
                    let nodes = vec![
                        ip.net.and_then(|n| cx.node[n]),
                        im.net.and_then(|n| cx.node[n]),
                        o.net.and_then(|n| cx.node[n]),
                        vcc.flatten(),
                        vee.flatten(),
                    ];
                    let rails = (vcc.is_some() && vee.is_some()) as usize;
                    let mut nodes = nodes;
                    nodes.truncate(3 + 2 * rails);
                    let headroom = cx.q(&b.and_then(|b| b.headroom.clone()), &["v"], &at, "headroom", 0.1);
                    let psrr_db = cx.opt(&b.and_then(|b| b.psrr.clone()), &["db"], &at, "psrr");
                    let kind = ElemKind::Opamp {
                        gain: cx.q(&b.and_then(|b| b.gain.clone()), &[], &at, "gain", 1e5),
                        gbw: cx.q(&b.and_then(|b| b.gbw.clone()), &["hz"], &at, "gbw", 1e6),
                        rout: cx.q(&b.and_then(|b| b.rout.clone()), &["ohm"], &at, "rout", 10.0),
                        vos: cx.q(&b.and_then(|b| b.vos.clone()), &["v"], &at, "vos", 0.0),
                        en: cx.q(&b.and_then(|b| b.en.clone()), &["v/rthz", "v/√hz", "v"], &at, "en", 0.0),
                        fc: cx.q(&b.and_then(|b| b.fc.clone()), &["hz"], &at, "fc", 0.0),
                        headroom_high: cx.q(&b.and_then(|b| b.headroom_high.clone()), &["v"], &at, "headroom_high", headroom),
                        headroom_low: cx.q(&b.and_then(|b| b.headroom_low.clone()), &["v"], &at, "headroom_low", headroom),
                        slew: cx.q(&b.and_then(|b| b.slew.clone()), &["v/us"], &at, "slew", 0.0) * 1e6,
                        ib: cx.q(&b.and_then(|b| b.ib.clone()), &["a"], &at, "ib", 0.0),
                        psrr: psrr_db.map(|db| 10f64.powf(db / 20.0)).unwrap_or(0.0),
                    };
                    let name = if found == 0 { r.reference.clone() } else { format!("{}#{}", r.reference, found + 1) };
                    push(&mut elements, name, kind, nodes);
                    found += 1;
                    if given {
                        break;
                    }
                }
                if found == 0 {
                    cx.d.error(&at, format!("{} has no +, - and output pins an op-amp needs; name them in `pins`", r.label()));
                }
            }
            "ldo" | "regulator" => {
                let i = block_pin(r, b, "vin", &["VIN", "IN", "VI", "INPUT"]);
                let o = block_pin(r, b, "vout", &["VOUT", "OUT", "VO", "OUTPUT"]);
                let g = block_pin(r, b, "gnd", &["GND", "VSS", "ADJ/GND", "GND/ADJ"]);
                let (Some(ni), Some(no), Some(ng)) = (
                    cx.pin_node(r, i, "the input (vin)", &at),
                    cx.pin_node(r, o, "the output (vout)", &at),
                    cx.pin_node(r, g, "ground (gnd)", &at),
                ) else {
                    continue;
                };
                let vout = match b.and_then(|b| b.vout.clone()) {
                    Some(q) => cx.q(&Some(q), &["v"], &at, "vout", 0.0),
                    None => {
                        cx.d.error(&at, format!("an ldo block for {} needs `vout`", r.label()));
                        continue;
                    }
                };
                let kind = ElemKind::Ldo {
                    vout,
                    dropout: cx.q(&b.and_then(|b| b.dropout.clone()), &["v"], &at, "dropout", 0.2),
                    rout: cx.q(&b.and_then(|b| b.rout.clone()), &["ohm"], &at, "rout", 0.05),
                    iq: cx.q(&b.and_then(|b| b.iq.clone()), &["a"], &at, "iq", 0.0),
                };
                push(&mut elements, r.reference.clone(), kind, vec![ni, no, ng]);
            }
            "vcvs" | "vccs" => {
                let roles = [("p", "out+"), ("n", "out-"), ("cp", "control+"), ("cn", "control-")];
                let mut nodes = Vec::new();
                for (role, what) in roles {
                    let pin = b.and_then(|b| b.pins.get(role)).and_then(|k| r.by(k));
                    match cx.pin_node(r, pin, what, &at) {
                        Some(n) => nodes.push(n),
                        None => break,
                    }
                }
                if nodes.len() < 4 {
                    continue;
                }
                let gain = cx.q(&b.and_then(|b| b.gain.clone()), &[], &at, "gain", 1.0);
                let kind = if chosen == "vcvs" { ElemKind::Vcvs { gain } } else { ElemKind::Vccs { gm: gain } };
                push(&mut elements, r.reference.clone(), kind, nodes);
            }
            "open" | "ignore" => {}
            "" if IGNORED.contains(&p) => {}
            "" => cx.d.error(
                &at,
                format!(
                    "{} has no circuit model; give it a [[blocks]] entry (opamp, ldo, vcvs, ...) or list it in `ignore`",
                    r.label()
                ),
            ),
            other => cx.d.error(&at, format!("unknown block kind `{other}`")),
        }
    }

    let node_of = |cx: &mut Ctx, name: &str, at: &str| -> Option<Node> {
        match net_named(name) {
            Some(i) => Some(cx.node[i]),
            None => {
                cx.d.error(at, format!("no net `{name}` in the schematic"));
                None
            }
        }
    };
    let mut drives: Vec<Drive> = Vec::new();
    for (i, f) in file.drives.iter().enumerate() {
        let at = format!("drives[{i}]");
        let Some(p) = node_of(&mut cx, &f.net, &at) else { continue };
        let n = match &f.ret {
            Some(r) => match node_of(&mut cx, r, &at) {
                Some(n) => n,
                None => continue,
            },
            None => None,
        };
        let units: &[&str] = if f.current { &["a"] } else { &["v"] };
        let dc = cx.q(&f.dc, units, &at, "dc", 0.0);
        let ac = cx.q(&f.ac, units, &at, "ac", 0.0);
        let r = cx.q(&f.resistance, &["ohm"], &at, "resistance", 0.0);
        let shapes = f.pulse.is_some() as usize
            + f.sine.is_some() as usize
            + f.surge.is_some() as usize
            + f.burst.is_some() as usize
            + (!f.steps.is_empty()) as usize;
        if shapes > 1 {
            cx.d.error(&at, "give one of `pulse`, `sine`, `surge`, `burst` and `steps`");
        }
        let time =
            |cx: &mut Ctx, v: &Option<Quantity>, field: &str| cx.q(v, &["s"], &at, field, 0.0);
        let wave = if let Some(pf) = &f.pulse {
            Wave::Pulse {
                low: cx.q(&Some(pf.low.clone()), units, &at, "pulse.low", 0.0),
                high: cx.q(&Some(pf.high.clone()), units, &at, "pulse.high", 0.0),
                delay: time(&mut cx, &pf.delay, "pulse.delay"),
                rise: time(&mut cx, &pf.rise, "pulse.rise").max(1e-12),
                fall: time(&mut cx, &pf.fall, "pulse.fall").max(1e-12),
                width: cx.q(&Some(pf.width.clone()), &["s"], &at, "pulse.width", 0.0),
                period: time(&mut cx, &pf.period, "pulse.period"),
            }
        } else if let Some(sf) = &f.sine {
            Wave::Sine {
                offset: cx.q(&sf.offset, units, &at, "sine.offset", dc),
                amplitude: cx.q(&Some(sf.amplitude.clone()), units, &at, "sine.amplitude", 0.0),
                frequency: cx.q(&Some(sf.frequency.clone()), &["hz"], &at, "sine.frequency", 0.0),
                delay: time(&mut cx, &sf.delay, "sine.delay"),
            }
        } else if let Some(sf) = &f.surge {
            let shape = match sf.shape.as_deref() {
                None | Some("open") => Surge::Open,
                Some("short") => Surge::Short,
                Some(other) => {
                    cx.d.error(&at, format!("surge.shape is `open` (1.2/50 us) or `short` (8/20 us), not `{other}`"));
                    Surge::Open
                }
            };
            Wave::Surge {
                offset: cx.q(&sf.offset, units, &at, "surge.offset", dc),
                peak: cx.q(&Some(sf.peak.clone()), units, &at, "surge.peak", 0.0),
                delay: time(&mut cx, &sf.delay, "surge.delay"),
                shape,
            }
        } else if let Some(bf) = &f.burst {
            Wave::Burst {
                offset: cx.q(&bf.offset, units, &at, "burst.offset", dc),
                peak: cx.q(&Some(bf.peak.clone()), units, &at, "burst.peak", 0.0),
                delay: time(&mut cx, &bf.delay, "burst.delay"),
                rate: cx.q(&bf.rate, &["hz"], &at, "burst.rate", 5e3),
                length: cx.q(&bf.length, &["s"], &at, "burst.length", 15e-3),
                period: cx.q(&bf.period, &["s"], &at, "burst.period", 300e-3),
            }
        } else if !f.steps.is_empty() {
            let mut points = Vec::new();
            for (k, (t, v)) in f.steps.iter().enumerate() {
                let t = cx.q(&Some(t.clone()), &["s"], &at, &format!("steps[{k}]"), 0.0);
                let v = cx.q(&Some(v.clone()), units, &at, &format!("steps[{k}]"), 0.0);
                if points.last().is_some_and(|l: &(f64, f64)| t < l.0) {
                    cx.d.error(&at, "`steps` must be in time order");
                }
                points.push((t, v));
            }
            Wave::Steps { points }
        } else {
            Wave::Dc
        };
        let dc = match &wave {
            Wave::Dc => dc,
            w if f.dc.is_none() => w.at(dc, 0.0),
            _ => dc,
        };
        drives.push(Drive { net: f.net.clone(), p, n, current: f.current, dc, ac, r, wave });
    }
    let sweep = |cx: &mut Ctx,
                 from: &Quantity,
                 to: &Quantity,
                 points: Option<usize>,
                 at: &str|
     -> Option<Sweep> {
        let f0 = cx.q(&Some(from.clone()), &["hz"], at, "from", 0.0);
        let f1 = cx.q(&Some(to.clone()), &["hz"], at, "to", 0.0);
        if f0 <= 0.0 || f1 < f0 {
            cx.d.error(at, "`from` must be above 0 Hz and `to` at least `from`");
            return None;
        }
        Some(Sweep { from: f0, to: f1, per_decade: points.unwrap_or(20).max(1) })
    };
    let ac = file.ac.as_ref().and_then(|s| sweep(&mut cx, &s.from, &s.to, s.points, "ac"));
    let tran = file.tran.as_ref().and_then(|t| {
        let stop = cx.q(&Some(t.stop.clone()), &["s"], "tran", "stop", 0.0);
        if stop <= 0.0 {
            cx.d.error("tran", "`stop` must be above 0");
            return None;
        }
        let step = cx.q(&t.step, &["s"], "tran", "step", stop / 1000.0);
        Some((stop, step.clamp(stop * 1e-7, stop)))
    });
    let noise = file.noise.as_ref().and_then(|f| {
        let output = node_of(&mut cx, &f.output, "noise")?;
        let input = match &f.input {
            None => None,
            Some(name) => match drives.iter().position(|x| &x.net == name) {
                Some(k) => Some(k),
                None => {
                    cx.d.error("noise", format!("`input` {name} is not a net in `drives`"));
                    return None;
                }
            },
        };
        let sw = sweep(&mut cx, &f.from, &f.to, f.points, "noise")?;
        Some(Noise { output, output_name: f.output.clone(), input, sweep: sw })
    });

    let mut checks = Vec::new();
    for (i, c) in file.checks.iter().enumerate() {
        let at = format!("checks[{i}]");
        let given = [
            &c.v,
            &c.i,
            &c.phase,
            &c.noise,
            &c.peak,
            &c.trough,
            &c.rms,
            &c.freq,
            &c.thd,
            &c.harmonics,
            &c.emission,
        ]
        .iter()
        .filter(|x| x.is_some())
        .count();
        if given != 1 {
            cx.d.error(
                &at,
                "a check names exactly one of `v`, `i`, `phase`, `noise`, `peak`, `trough`, `rms`, `freq`, `thd`, `harmonics` and `emission`",
            );
            continue;
        }
        let windowed = [&c.peak, &c.trough, &c.rms, &c.freq, &c.thd, &c.harmonics, &c.emission]
            .iter()
            .any(|x| x.is_some());
        if windowed && c.at.is_some() {
            cx.d.error(&at, "`peak`, `trough`, `rms`, `freq`, `thd`, `harmonics` and `emission` read a span: give `from` and `to`, not `at`");
            continue;
        }
        if !windowed && (c.from.is_some() || c.to.is_some()) {
            cx.d.error(&at, "`from` and `to` go with `peak`, `trough`, `rms`, `freq`, `thd`, `harmonics` and `emission`");
            continue;
        }
        if c.harmonics.is_none() && c.mains.is_some() {
            cx.d.error(&at, "`mains` goes with `harmonics`");
            continue;
        }
        if c.harmonics.is_none() && c.emission.is_none() && c.class.is_some() {
            cx.d.error(&at, "`class` goes with `harmonics` and `emission`");
            continue;
        }
        if c.emission.is_none() && c.detector.is_some() {
            cx.d.error(&at, "`detector` goes with `emission`");
            continue;
        }
        let when = match &c.at {
            None if windowed => {
                let from = cx.opt(&c.from, &["s"], &at, "from").unwrap_or(0.0);
                let to = cx.opt(&c.to, &["s"], &at, "to").unwrap_or(f64::INFINITY);
                if to <= from {
                    cx.d.error(&at, "`to` comes before `from`");
                    continue;
                }
                At::Window(from, to)
            }
            None => At::Op,
            Some(Quantity::Text(s)) if s.eq_ignore_ascii_case("op") => At::Op,
            Some(q) => {
                let text = match q {
                    Quantity::Text(s) => s.to_ascii_lowercase(),
                    Quantity::Number(_) => String::new(),
                };
                if text.ends_with("hz") {
                    At::Freq(cx.q(&c.at, &["hz"], &at, "at", 0.0))
                } else if text.ends_with('s') {
                    At::Time(cx.q(&c.at, &["s"], &at, "at", 0.0))
                } else {
                    cx.d.error(
                        &at,
                        "`at` is \"op\", a time like \"1ms\" or a frequency like \"1kHz\"",
                    );
                    continue;
                }
            }
        };
        let (measure, label, units): (Measure, String, &[&str]) = if let Some(v) = &c.v {
            let Some(nd) = node_of(&mut cx, v, &at) else { continue };
            let units: &[&str] = if matches!(when, At::Freq(_)) { &["db"] } else { &["v"] };
            (Measure::Volts(nd), format!("v({v})"), units)
        } else if let Some(p) = &c.phase {
            let Some(nd) = node_of(&mut cx, p, &at) else { continue };
            (Measure::Phase(nd), format!("phase({p})"), &["deg", "°"])
        } else if let Some(nn) = &c.noise {
            let Some(nd) = node_of(&mut cx, nn, &at) else { continue };
            (Measure::Noise(nd), format!("noise({nn})"), &["v/rthz", "v/√hz", "v"])
        } else if let Some(w) = &c.peak {
            let Some(nd) = node_of(&mut cx, w, &at) else { continue };
            (Measure::Peak(nd), format!("peak({w})"), &["v"])
        } else if let Some(w) = &c.trough {
            let Some(nd) = node_of(&mut cx, w, &at) else { continue };
            (Measure::Trough(nd), format!("trough({w})"), &["v"])
        } else if let Some(w) = &c.rms {
            let Some(nd) = node_of(&mut cx, w, &at) else { continue };
            (Measure::Rms(nd), format!("rms({w})"), &["v"])
        } else if let Some(w) = &c.freq {
            let Some(nd) = node_of(&mut cx, w, &at) else { continue };
            (Measure::Freq(nd), format!("freq({w})"), &["hz"])
        } else if let Some(w) = &c.thd {
            let Some(nd) = node_of(&mut cx, w, &at) else { continue };
            (Measure::Thd(nd), format!("thd({w})"), &["%"])
        } else if let Some(name) = &c.harmonics {
            let Some(element) = elements.iter().position(|e| &e.name == name && e.nodes.len() == 2)
            else {
                cx.d.error(&at, format!("`harmonics` needs a two-pin part in the circuit, `{name}` is not one"));
                continue;
            };
            let class_b = match c.class.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None | Some("a") => false,
                Some("b") => true,
                Some(other) => {
                    cx.d.error(&at, format!("`class` is \"a\" or \"b\", not `{other}`"));
                    continue;
                }
            };
            let mains = cx.q(&c.mains, &["hz"], &at, "mains", 50.0);
            (Measure::Harmonics { element, mains, class_b }, format!("harmonics({name})"), &["%"])
        } else if let Some(w) = &c.emission {
            let Some(node) = node_of(&mut cx, w, &at) else { continue };
            let class_a = match c.class.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None | Some("b") => false,
                Some("a") => true,
                Some(other) => {
                    cx.d.error(&at, format!("`class` is \"a\" or \"b\", not `{other}`"));
                    continue;
                }
            };
            let average = match c.detector.as_deref() {
                None | Some("quasi-peak") | Some("qp") => false,
                Some("average") | Some("av") => true,
                Some(other) => {
                    cx.d.error(&at, format!("`detector` is \"quasi-peak\" or \"average\", not `{other}`"));
                    continue;
                }
            };
            (Measure::Emission { node, class_a, average }, format!("emission({w})"), &["db"])
        } else {
            let name = c.i.as_deref().unwrap_or_default();
            match elements.iter().position(|e| e.name == name && e.nodes.len() == 2) {
                Some(k) => (Measure::Amps(k), format!("i({name})"), &["a"]),
                None => {
                    cx.d.error(
                        &at,
                        format!("`i` needs a two-pin part in the circuit, `{name}` is not one"),
                    );
                    continue;
                }
            }
        };
        match (&measure, when) {
            (Measure::Phase(_), At::Freq(_)) | (Measure::Noise(_), At::Freq(_)) => {}
            (Measure::Phase(_), _) | (Measure::Noise(_), _) => {
                cx.d.error(&at, "`phase` and `noise` are read at a frequency, like at = \"1kHz\"");
                continue;
            }
            (Measure::Amps(_), At::Freq(_)) => {
                cx.d.error(&at, "`i` is read at the operating point or a time");
                continue;
            }
            _ => {}
        }
        match when {
            At::Freq(_) if matches!(measure, Measure::Noise(_)) && noise.is_none() => {
                cx.d.error(&at, "a noise check needs a [noise] analysis")
            }
            At::Freq(_) if !matches!(measure, Measure::Noise(_)) && ac.is_none() => {
                cx.d.error(&at, "a check at a frequency needs an [ac] analysis")
            }
            At::Time(_) if tran.is_none() => {
                cx.d.error(&at, "a check at a time needs a [tran] analysis")
            }
            At::Window(..) if tran.is_none() => {
                cx.d.error(&at, "`peak`, `trough`, `rms`, `freq` and `thd` need a [tran] analysis")
            }
            _ => {}
        }
        let min = cx.opt(&c.min, units, &at, "min");
        let max = cx.opt(&c.max, units, &at, "max");
        if min.is_none() && max.is_none() {
            cx.d.error(&at, "give the check a `min`, a `max` or both");
        }
        let label = match when {
            At::Op => label,
            At::Time(t) => format!("{label} at {}", fmt_si(t, "s")),
            At::Freq(f) => format!("{label} at {}", fmt_si(f, "Hz")),
            At::Window(a, b) if b.is_finite() => {
                format!("{label} from {} to {}", fmt_si(a, "s"), fmt_si(b, "s"))
            }
            At::Window(a, _) if a > 0.0 => format!("{label} from {}", fmt_si(a, "s")),
            At::Window(..) => label,
        };
        checks.push(Check { label, measure, at: when, min, max });
    }

    let mut record: Vec<usize> = Vec::new();
    for rf in &file.record {
        let names: Vec<String> = match rf {
            crate::logic::RecordFile::Net(n) => vec![n.clone()],
            crate::logic::RecordFile::Bus { nets, .. } => nets.clone(),
        };
        for name in names {
            match net_named(&name).map(|i| cx.node[i]) {
                Some(Some(k)) if !record.contains(&k) => record.push(k),
                Some(_) => {}
                None => cx.d.error("record", format!("no net `{name}` in the schematic")),
            }
        }
    }
    if file.record.is_empty() {
        record = (0..names.len()).collect();
    }
    if elements.is_empty() && drives.is_empty() {
        cx.d.error("schematic", "nothing to simulate: no part has a circuit model");
    }
    CircuitSpec {
        schematic: sch.name.clone(),
        nets: names,
        elements,
        drives,
        ac,
        tran,
        noise,
        checks,
        record,
        temperature: file.temperature.unwrap_or(27.0),
        netlist_hash: crate::logic::netlist_hash(sch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_reads_prefixes_and_units() {
        assert_eq!(si("5V", &["v"]), Some(5.0));
        assert_eq!(si("3.3", &["v"]), Some(3.3));
        assert!((si("1MHz", &["hz"]).unwrap() - 1e6).abs() < 1e-6);
        assert!((si("1mA", &["a"]).unwrap() - 1e-3).abs() < 1e-12);
        assert!((si("100nF", &["f"]).unwrap() - 1e-7).abs() < 1e-15);
        assert!((si("4k7", &["ohm"]).unwrap() - 4700.0).abs() < 1e-9);
        assert!((si("10meg", &["ohm"]).unwrap() - 1e7).abs() < 1e-3);
        assert!((si("2.5us", &["s"]).unwrap() - 2.5e-6).abs() < 1e-15);
    }

    #[test]
    fn zener_voltage_from_value() {
        assert_eq!(zener_volts("BZX84C3V3"), Some(3.3));
        assert_eq!(zener_volts("MMSZ5V1"), Some(5.1));
        assert_eq!(zener_volts("12V"), Some(12.0));
    }

    #[test]
    fn pulse_shape() {
        let w = Wave::Pulse {
            low: 0.0,
            high: 1.0,
            delay: 1.0,
            rise: 1.0,
            fall: 1.0,
            width: 2.0,
            period: 0.0,
        };
        assert_eq!(w.at(0.0, 0.5), 0.0);
        assert!((w.at(0.0, 1.5) - 0.5).abs() < 1e-12);
        assert_eq!(w.at(0.0, 3.0), 1.0);
        assert!((w.at(0.0, 4.5) - 0.5).abs() < 1e-12);
        assert_eq!(w.at(0.0, 6.0), 0.0);
    }
}

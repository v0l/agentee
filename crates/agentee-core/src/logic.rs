use crate::diag::Diags;
use crate::schematic::{NetStyle, PinRef, Schematic, UnionFind};
use crate::sim::{Reading, SimFile, parse_time, parse_value};
use crate::symbol::{PinShape, PinType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Level {
    L,
    H,
    X,
    Z,
}

impl Level {
    pub fn of(b: bool) -> Level {
        if b { Level::H } else { Level::L }
    }

    pub fn bit(self) -> Option<bool> {
        match self {
            Level::L => Some(false),
            Level::H => Some(true),
            _ => None,
        }
    }

    pub fn flip(self) -> Level {
        match self {
            Level::L => Level::H,
            Level::H => Level::L,
            _ => Level::X,
        }
    }

    pub fn char(self) -> char {
        match self {
            Level::L => '0',
            Level::H => '1',
            Level::X => 'x',
            Level::Z => 'z',
        }
    }

    pub fn from_char(c: char) -> Option<Level> {
        match c {
            '0' => Some(Level::L),
            '1' => Some(Level::H),
            'x' | 'X' => Some(Level::X),
            'z' | 'Z' => Some(Level::Z),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LevelFile {
    Number(u64),
    Text(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockFile {
    pub period: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StimulusFile {
    pub net: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<(String, LevelFile)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constant: Option<LevelFile>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    #[default]
    Rising,
    Falling,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<LevelFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<Edge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence: Vec<LevelFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartModelFile {
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truth: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub delays: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removal: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecordFile {
    Net(String),
    Bus { name: String, nets: Vec<String> },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnViolation {
    #[default]
    X,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum GateOp {
    And,
    Or,
    Xor,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Prim {
    Const(Level),
    Gate { op: GateOp, invert: bool },
    Tri,
    Dff,
    Jk,
    Sr,
    Dlatch,
    Mux2,
    Dec138,
    Counter { sync_reset: bool },
    Shift164,
    Shift595,
    Xcvr,
    Truth(Vec<(Vec<Option<bool>>, Vec<Level>)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clocked {
    pub clock: usize,
    pub falling: bool,
    pub data: &'static [usize],
    pub resets: &'static [usize],
    pub lead: Option<usize>,
    pub state: std::ops::Range<usize>,
}

impl Prim {
    pub fn clocked(&self) -> Vec<Clocked> {
        let on = |clock, data, resets, state| Clocked {
            clock,
            falling: false,
            data,
            resets,
            lead: None,
            state,
        };
        match self {
            Prim::Dff => vec![on(1, &[0], &[2, 3], 0..2)],
            Prim::Jk => vec![on(2, &[0, 1], &[3, 4], 0..2)],
            Prim::Dlatch => vec![Clocked { falling: true, ..on(1, &[0], &[2], 0..2) }],
            Prim::Counter { sync_reset: false } => vec![on(1, &[2, 3, 4, 5, 6, 7, 8], &[0], 0..4)],
            Prim::Counter { sync_reset: true } => vec![on(1, &[0, 2, 3, 4, 5, 6, 7, 8], &[], 0..4)],
            Prim::Shift164 => vec![on(2, &[0, 1], &[3], 0..8)],
            Prim::Shift595 => {
                vec![on(1, &[0], &[3], 0..8), Clocked { lead: Some(1), ..on(2, &[], &[], 8..16) }]
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Input {
    Net { net: usize, invert: bool },
    Fixed(Level),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Output {
    pub net: Option<usize>,
    pub invert: bool,
    pub delay: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Cell {
    pub part: String,
    pub prim: Prim,
    pub input_names: Vec<String>,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub weak: bool,
    pub open_drain: bool,
    pub setup: u64,
    pub hold: u64,
    pub recovery: u64,
    pub removal: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Circuit {
    pub nets: Vec<String>,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Wave {
    Clock { period: u64, high: u64, phase: u64 },
    Steps(Vec<(u64, Level)>),
}

#[derive(Clone, Debug, Serialize)]
pub struct Stimulus {
    pub net: usize,
    pub name: String,
    pub wave: Wave,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Check {
    At {
        time: u64,
        value: Vec<Option<Level>>,
    },
    Sequence {
        clock: usize,
        clock_name: String,
        edge: Edge,
        from: u64,
        values: Vec<Vec<Option<Level>>>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Expect {
    pub label: String,
    pub nets: Vec<usize>,
    pub names: Vec<String>,
    pub check: Check,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bus {
    pub name: String,
    pub nets: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Assertion,
    Timing,
    Contention,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub time: u64,
    pub kind: MarkKind,
    pub nets: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LogicSpec {
    pub schematic: String,
    pub duration: u64,
    pub record: Vec<(String, usize)>,
    pub buses: Vec<Bus>,
    pub on_violation: OnViolation,
    pub stimuli: Vec<Stimulus>,
    pub expects: Vec<Expect>,
    pub circuit: Circuit,
    pub netlist_hash: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trace {
    pub name: String,
    pub times: Vec<u64>,
    pub values: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusGroup {
    pub name: String,
    pub stem: String,
    pub msb: u32,
    pub lsb: u32,
    pub bits: Vec<usize>,
}

pub fn bus_bit(name: &str) -> Option<(String, u32, String)> {
    if let Some(stem) = name.strip_suffix(']')
        && let Some((prefix, idx)) = stem.rsplit_once('[')
        && !prefix.is_empty()
        && let Ok(i) = idx.parse()
    {
        return Some((prefix.to_string(), i, String::new()));
    }
    let end = name.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = name[..end].rfind(|c: char| !c.is_ascii_digit())? + 1;
    let suffix = &name[end..];
    if !["", "_N", "_n", "#"].contains(&suffix) {
        return None;
    }
    let i = name[start..end].parse().ok()?;
    Some((name[..start].to_string(), i, suffix.to_string()))
}

type AutoBus = ((String, String), Vec<(u32, usize)>);

pub fn bus_groups(traces: &[Trace], buses: &[Bus]) -> Vec<BusGroup> {
    let find = |n: &str| traces.iter().position(|t| t.name == n);
    let mut groups: Vec<BusGroup> = Vec::new();
    let mut used = vec![false; traces.len()];
    for b in buses {
        let bits: Option<Vec<usize>> = b.nets.iter().map(|n| find(n)).collect();
        if let Some(bits) = bits
            && !bits.is_empty()
        {
            bits.iter().for_each(|k| used[*k] = true);
            groups.push(BusGroup {
                name: b.name.clone(),
                stem: b.name.clone(),
                msb: bits.len() as u32 - 1,
                lsb: 0,
                bits,
            });
        }
    }
    let mut auto: Vec<AutoBus> = Vec::new();
    for (k, t) in traces.iter().enumerate() {
        if used[k] {
            continue;
        }
        let Some((prefix, i, suffix)) = bus_bit(&t.name) else { continue };
        let key = (prefix, suffix);
        match auto.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push((i, k)),
            None => auto.push((key, vec![(i, k)])),
        }
    }
    for ((prefix, suffix), mut bits) in auto {
        bits.sort_by_key(|b| std::cmp::Reverse(b.0));
        let distinct = bits.windows(2).all(|w| w[0].0 != w[1].0);
        if bits.len() < 2 || !distinct {
            continue;
        }
        let (hi, lo) = (bits[0].0, bits[bits.len() - 1].0);
        groups.push(BusGroup {
            name: format!("{prefix}[{hi}:{lo}]{suffix}"),
            stem: format!("{prefix}{suffix}"),
            msb: hi,
            lsb: lo,
            bits: bits.into_iter().map(|b| b.1).collect(),
        });
    }
    groups
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicResult {
    pub name: String,
    pub kind: String,
    pub spec_hash: u64,
    pub duration_ps: u64,
    pub end_ps: u64,
    pub traces: Vec<Trace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<Bus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    pub passed: usize,
    pub failures: Vec<String>,
    pub readings: Vec<Reading>,
    pub events: u64,
    pub seconds: f64,
    pub vcd: String,
}

pub fn logic_hash(src: u64, netlist: u64) -> u64 {
    crate::sim::cascade_hash(src, netlist, &[])
}

pub fn netlist_hash(sch: &Schematic) -> u64 {
    let mut s = String::new();
    for p in &sch.parts {
        s.push_str(&format!("{}|{}|{}|{}|{};", p.reference, p.value, p.symbol_name, p.unit, p.dnp));
        for (_, pin) in p.pins() {
            s.push_str(&format!("{}:{}:{:?},", pin.number, pin.name, pin.kind));
        }
    }
    for n in &sch.nets {
        s.push_str(&format!("#{}:{:?}", n.name, n.style));
        for r in &n.pins {
            s.push_str(&sch.pin_label(*r));
            s.push(',');
        }
    }
    crate::sim::hash(&s)
}

pub fn ps(s: &str) -> Option<u64> {
    let t = parse_time(s)?;
    (t >= 0.0 && t.is_finite()).then(|| (t * 1e12).round() as u64)
}

pub fn fmt_time(t: u64) -> String {
    let (div, unit) = match t {
        1_000_000_000.. => (1e9, "ms"),
        1_000_000.. => (1e6, "us"),
        1_000.. => (1e3, "ns"),
        0 => (1.0, "ns"),
        _ => (1.0, "ps"),
    };
    let s = format!("{:.3}", t as f64 / div);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}{unit}")
}

pub fn pattern(v: &LevelFile, width: usize) -> Result<Vec<Option<Level>>, String> {
    match v {
        LevelFile::Number(n) => {
            if width < 64 && n >> width != 0 {
                return Err(format!("{n} does not fit in {width} bit(s)"));
            }
            Ok((0..width).rev().map(|i| Some(Level::of(n >> i & 1 == 1))).collect())
        }
        LevelFile::Text(s) => {
            let chars: Vec<char> = s.chars().filter(|c| *c != '_').collect();
            if chars.len() != width {
                return Err(format!("`{s}` has {} character(s), expected {width}", chars.len()));
            }
            chars
                .iter()
                .map(|c| match c {
                    '-' => Ok(None),
                    c => Level::from_char(*c)
                        .map(Some)
                        .ok_or(format!("`{s}` should use 0, 1, x, z or -")),
                })
                .collect()
        }
    }
}

fn level(v: &LevelFile) -> Result<Level, String> {
    match pattern(v, 1)?[0] {
        Some(l) => Ok(l),
        None => Err("a driven level is 0, 1, x or z".into()),
    }
}

pub fn rail_level(name: &str) -> Option<Level> {
    let u = name.to_ascii_uppercase();
    if u.contains("PWR_FLAG") {
        return None;
    }
    let low = u.contains("GND")
        || u.starts_with("VSS")
        || u.starts_with("VEE")
        || u.starts_with("0V")
        || u == "GROUND";
    Some(Level::of(!low))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Timing {
    pub delay: u64,
    pub setup: u64,
    pub hold: u64,
    pub recovery: u64,
    pub removal: u64,
}

pub const GENERIC_TIMING: Timing =
    Timing { delay: 1_000, setup: 0, hold: 0, recovery: 0, removal: 0 };

pub fn family_timing(family: &str) -> Timing {
    let t = |d: u64, s: u64, h: u64, recovery: u64, removal: u64| Timing {
        delay: d * 1000,
        setup: s * 1000,
        hold: h * 1000,
        recovery,
        removal,
    };
    match family {
        "HC" | "HCT" => t(10, 15, 3, 8_000, 0),
        "AHC" | "AHCT" | "VHC" | "VHCT" => t(6, 5, 1, 3_500, 0),
        "AC" | "ACT" => t(6, 4, 1, 2_400, 0),
        "LVC" | "ALVC" | "LVT" | "ALVT" | "AUC" | "AVC" => t(4, 2, 1, 2_000, 0),
        "AUP" | "LV" | "LVX" => t(6, 3, 1, 3_000, 1_000),
        "LS" | "" => t(15, 20, 5, 25_000, 3_000),
        _ => t(10, 10, 2, 10_000, 2_000),
    }
}

type Unit = (&'static str, Vec<(&'static str, &'static str)>);

fn units_of(prim: &'static str, keys: &[&'static str], table: &[&[&'static str]]) -> Vec<Unit> {
    table
        .iter()
        .map(|pins| (prim, keys.iter().copied().zip(pins.iter().copied()).collect()))
        .collect()
}

const QUAD00: &[&[&str]] =
    &[&["1", "2", "3"], &["4", "5", "6"], &["9", "10", "8"], &["12", "13", "11"]];
const QUAD02: &[&[&str]] =
    &[&["2", "3", "1"], &["5", "6", "4"], &["8", "9", "10"], &["11", "12", "13"]];
const AB: &[&str] = &["A", "B", "Y"];

const OCT_IN: [&str; 8] = ["2", "3", "4", "5", "6", "7", "8", "9"];
const OCT_OUT: [&str; 8] = ["19", "18", "17", "16", "15", "14", "13", "12"];
const OCT_B: [&str; 8] = ["18", "17", "16", "15", "14", "13", "12", "11"];

fn octal(keys: &[&'static str], shared: &[&'static str], high: [&'static str; 8]) -> Vec<Unit> {
    (0..8)
        .map(|i| {
            let pins = shared.iter().copied().chain([OCT_IN[i], high[i]]);
            (keys[0], keys[1..].iter().copied().zip(pins).collect())
        })
        .collect()
}

pub fn library(code: &str) -> Option<Vec<Unit>> {
    const HEX04: &[&[&str]] =
        &[&["1", "2"], &["3", "4"], &["5", "6"], &["9", "8"], &["11", "10"], &["13", "12"]];
    const TRIPLE3: &[&[&str]] =
        &[&["1", "2", "13", "12"], &["3", "4", "5", "6"], &["9", "10", "11", "8"]];
    const DUAL4: &[&[&str]] = &[&["1", "2", "4", "5", "6"], &["9", "10", "12", "13", "8"]];
    const QUAD125: &[&[&str]] =
        &[&["1", "2", "3"], &["4", "5", "6"], &["10", "9", "8"], &["13", "12", "11"]];
    const OCT244: &[&[&str]] = &[
        &["1", "2", "18"],
        &["1", "4", "16"],
        &["1", "6", "14"],
        &["1", "8", "12"],
        &["19", "11", "9"],
        &["19", "13", "7"],
        &["19", "15", "5"],
        &["19", "17", "3"],
    ];
    const QUAD157: &[&[&str]] = &[
        &["1", "15", "2", "3", "4"],
        &["1", "15", "5", "6", "7"],
        &["1", "15", "11", "10", "9"],
        &["1", "15", "14", "13", "12"],
    ];
    const SINGLE2: &[&[&str]] = &[&["1", "2", "4"]];
    const SINGLE1: &[&[&str]] = &[&["2", "4"]];
    const SINGLE125: &[&[&str]] = &[&["1", "2", "4"]];
    const DUAL2: &[&[&str]] = &[&["1", "2", "7"], &["5", "6", "3"]];
    const DUAL1: &[&[&str]] = &[&["1", "6"], &["3", "4"]];
    const DUAL125: &[&[&str]] = &[&["1", "2", "6"], &["7", "5", "3"]];
    const ABC: &[&str] = &["A", "B", "C", "Y"];
    const ABCD: &[&str] = &["A", "B", "C", "D", "Y"];
    const AY: &[&str] = &["A", "Y"];
    const TRI_N: &[&str] = &["OE_N", "A", "Y"];
    const TRI: &[&str] = &["OE", "A", "Y"];
    let dff74 = vec![("CLK", "1"), ("D", "2"), ("QN", "3"), ("Q", "5"), ("R_N", "6"), ("S_N", "7")];
    let counter = |prim| {
        vec![(
            prim,
            vec![
                ("R_N", "1"),
                ("CLK", "2"),
                ("D0", "3"),
                ("D1", "4"),
                ("D2", "5"),
                ("D3", "6"),
                ("CEP", "7"),
                ("LOAD_N", "9"),
                ("CET", "10"),
                ("Q3", "11"),
                ("Q2", "12"),
                ("Q1", "13"),
                ("Q0", "14"),
                ("TC", "15"),
            ],
        )]
    };
    Some(match code {
        "00" | "132" => units_of("nand", AB, QUAD00),
        "01" => units_of("nand_od", AB, QUAD02),
        "03" => units_of("nand_od", AB, QUAD00),
        "05" | "06" => units_of("not_od", AY, HEX04),
        "07" => units_of("buf_od", AY, HEX04),
        "573" => octal(&["dlatch", "OE_N", "EN", "D", "Q"], &["1", "11"], OCT_OUT),
        "574" => octal(&["dff", "OE_N", "CLK", "D", "Q"], &["1", "11"], OCT_OUT),
        "245" => octal(&["xcvr", "DIR", "OE_N", "A", "B"], &["1", "19"], OCT_B),
        "08" => units_of("and", AB, QUAD00),
        "32" => units_of("or", AB, QUAD00),
        "86" => units_of("xor", AB, QUAD00),
        "02" => units_of("nor", AB, QUAD02),
        "04" | "14" => units_of("not", AY, HEX04),
        "10" => units_of("nand", ABC, TRIPLE3),
        "11" => units_of("and", ABC, TRIPLE3),
        "27" => units_of("nor", ABC, TRIPLE3),
        "20" => units_of("nand", ABCD, DUAL4),
        "21" => units_of("and", ABCD, DUAL4),
        "74" => vec![
            (
                "dff",
                vec![("R_N", "1"), ("D", "2"), ("CLK", "3"), ("S_N", "4"), ("Q", "5"), ("QN", "6")],
            ),
            (
                "dff",
                vec![
                    ("R_N", "13"),
                    ("D", "12"),
                    ("CLK", "11"),
                    ("S_N", "10"),
                    ("Q", "9"),
                    ("QN", "8"),
                ],
            ),
        ],
        "125" => units_of("tri", TRI_N, QUAD125),
        "126" => units_of("tri", TRI, QUAD125),
        "244" => units_of("tri", TRI_N, OCT244),
        "157" => units_of("mux2", &["S", "EN_N", "I0", "I1", "Y"], QUAD157),
        "138" => vec![(
            "dec138",
            vec![
                ("A0", "1"),
                ("A1", "2"),
                ("A2", "3"),
                ("E1_N", "4"),
                ("E2_N", "5"),
                ("E3", "6"),
                ("Y0_N", "15"),
                ("Y1_N", "14"),
                ("Y2_N", "13"),
                ("Y3_N", "12"),
                ("Y4_N", "11"),
                ("Y5_N", "10"),
                ("Y6_N", "9"),
                ("Y7_N", "7"),
            ],
        )],
        "161" => counter("counter161"),
        "163" => counter("counter163"),
        "164" => vec![(
            "shift164",
            vec![
                ("A", "1"),
                ("B", "2"),
                ("Q0", "3"),
                ("Q1", "4"),
                ("Q2", "5"),
                ("Q3", "6"),
                ("CLK", "8"),
                ("R_N", "9"),
                ("Q4", "10"),
                ("Q5", "11"),
                ("Q6", "12"),
                ("Q7", "13"),
            ],
        )],
        "595" => vec![(
            "shift595",
            vec![
                ("Q1", "1"),
                ("Q2", "2"),
                ("Q3", "3"),
                ("Q4", "4"),
                ("Q5", "5"),
                ("Q6", "6"),
                ("Q7", "7"),
                ("QS", "9"),
                ("R_N", "10"),
                ("CLK", "11"),
                ("LATCH", "12"),
                ("OE_N", "13"),
                ("D", "14"),
                ("Q0", "15"),
            ],
        )],
        "1G00" => units_of("nand", AB, SINGLE2),
        "1G08" => units_of("and", AB, SINGLE2),
        "1G32" => units_of("or", AB, SINGLE2),
        "1G86" => units_of("xor", AB, SINGLE2),
        "1G02" => units_of("nor", AB, SINGLE2),
        "1G04" | "1G14" => units_of("not", AY, SINGLE1),
        "1G34" | "1G17" => units_of("buf", AY, SINGLE1),
        "1G06" => units_of("not_od", AY, SINGLE1),
        "1G07" => units_of("buf_od", AY, SINGLE1),
        "1G125" => units_of("tri", TRI_N, SINGLE125),
        "1G126" => units_of("tri", TRI, SINGLE125),
        "1G79" => vec![("dff", vec![("D", "1"), ("CLK", "2"), ("Q", "4")])],
        "1G80" => vec![("dff", vec![("D", "1"), ("CLK", "2"), ("QN", "4")])],
        "1G74" | "2G74" => vec![("dff", dff74)],
        "1G157" => vec![("mux2", vec![("I1", "1"), ("I0", "3"), ("Y", "4"), ("S", "6")])],
        "2G00" => units_of("nand", AB, DUAL2),
        "2G08" => units_of("and", AB, DUAL2),
        "2G32" => units_of("or", AB, DUAL2),
        "2G86" => units_of("xor", AB, DUAL2),
        "2G02" => units_of("nor", AB, DUAL2),
        "2G04" | "2G14" => units_of("not", AY, DUAL1),
        "2G34" | "2G17" => units_of("buf", AY, DUAL1),
        "2G125" => units_of("tri", TRI_N, DUAL125),
        "2G126" => units_of("tri", TRI, DUAL125),
        _ => return None,
    })
}

pub fn part_code(s: &str) -> Option<(String, String)> {
    let u: String = s.to_ascii_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut from = 0;
    while let Some(k) = u[from..].find("74") {
        let at = from + k + 2;
        let fam_end = at + u[at..].bytes().take_while(u8::is_ascii_alphabetic).count();
        let rest = &u[fam_end..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let code = if digits == 1 && rest[1..].starts_with('G') {
            let more = rest[2..].bytes().take_while(u8::is_ascii_digit).count();
            (more > 0).then(|| rest[..2 + more].to_string())
        } else {
            (digits >= 2).then(|| rest[..digits].to_string())
        };
        if let Some(c) = code
            && library(&c).is_some()
        {
            return Some((u[at..fam_end].to_string(), c));
        }
        from += k + 1;
    }
    None
}

pub fn generic(s: &str) -> Option<&'static str> {
    let u: String = s.to_ascii_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let base = u.trim_end_matches(|c: char| c.is_ascii_digit());
    Some(match base {
        "AND" => "and",
        "OR" => "or",
        "XOR" => "xor",
        "NAND" => "nand",
        "NOR" => "nor",
        "XNOR" => "xnor",
        _ => match u.as_str() {
            "NOT" | "INV" | "INVERTER" => "not",
            "BUF" | "BUFFER" => "buf",
            "TRIBUF" | "TRISTATE" | "BUFT" | "TBUF" => "tri",
            "DFF" | "DFLIPFLOP" | "DFFSR" => "dff",
            "JK" | "JKFF" | "JKFLIPFLOP" => "jk",
            "SR" | "SRLATCH" | "RSLATCH" => "sr",
            "DLATCH" => "dlatch",
            "MUX" | "MUX2" | "MUX21" => "mux2",
            _ => return None,
        },
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub key: String,
    pub pin: Option<String>,
    pub invert: bool,
    pub default: Option<Level>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub prim: Prim,
    pub inputs: Vec<Slot>,
    pub outputs: Vec<Slot>,
    pub open_drain: bool,
}

fn split_key(k: &str) -> (String, bool) {
    let u = k.trim().to_ascii_uppercase();
    if u == "QN" || u == "Q_N" {
        return ("QN".into(), false);
    }
    match u.strip_suffix("_N") {
        Some(b) if !b.is_empty() => (b.to_string(), true),
        _ => (u, false),
    }
}

type Sig = (&'static [(&'static str, Option<Level>)], &'static [&'static str]);

fn signature(prim: &str) -> Option<(Prim, Sig)> {
    use Level::{H, L};
    Some(match prim {
        "tri" => (Prim::Tri, (&[("A", None), ("OE", None)], &["Y"])),
        "dff" => (
            Prim::Dff,
            (
                &[("D", None), ("CLK", None), ("S", Some(L)), ("R", Some(L)), ("OE", Some(H))],
                &["Q", "QN"],
            ),
        ),
        "jk" => (
            Prim::Jk,
            (
                &[("J", None), ("K", None), ("CLK", None), ("S", Some(L)), ("R", Some(L))],
                &["Q", "QN"],
            ),
        ),
        "sr" => (Prim::Sr, (&[("S", None), ("R", None)], &["Q", "QN"])),
        "dlatch" => (
            Prim::Dlatch,
            (&[("D", None), ("EN", None), ("R", Some(L)), ("OE", Some(H))], &["Q", "QN"]),
        ),
        "xcvr" => {
            (Prim::Xcvr, (&[("A", None), ("B", None), ("DIR", None), ("OE", Some(H))], &["A", "B"]))
        }
        "mux2" => {
            (Prim::Mux2, (&[("I0", None), ("I1", None), ("S", None), ("EN", Some(H))], &["Y"]))
        }
        "dec138" => (
            Prim::Dec138,
            (
                &[
                    ("A0", None),
                    ("A1", None),
                    ("A2", None),
                    ("E1", None),
                    ("E2", None),
                    ("E3", None),
                ],
                &["Y0", "Y1", "Y2", "Y3", "Y4", "Y5", "Y6", "Y7"],
            ),
        ),
        "counter161" | "counter163" => (
            Prim::Counter { sync_reset: prim == "counter163" },
            (
                &[
                    ("R", Some(L)),
                    ("CLK", None),
                    ("D0", Some(L)),
                    ("D1", Some(L)),
                    ("D2", Some(L)),
                    ("D3", Some(L)),
                    ("CEP", Some(H)),
                    ("LOAD", Some(L)),
                    ("CET", Some(H)),
                ],
                &["Q0", "Q1", "Q2", "Q3", "TC"],
            ),
        ),
        "shift164" => (
            Prim::Shift164,
            (
                &[("A", None), ("B", Some(H)), ("CLK", None), ("R", Some(L))],
                &["Q0", "Q1", "Q2", "Q3", "Q4", "Q5", "Q6", "Q7"],
            ),
        ),
        "shift595" => (
            Prim::Shift595,
            (
                &[("D", None), ("CLK", None), ("LATCH", None), ("R", Some(L)), ("OE", Some(H))],
                &["Q0", "Q1", "Q2", "Q3", "Q4", "Q5", "Q6", "Q7", "QS"],
            ),
        ),
        _ => return None,
    })
}

pub const PRIMITIVES: &[&str] = &[
    "and",
    "or",
    "xor",
    "nand",
    "nor",
    "xnor",
    "not",
    "buf",
    "tri",
    "dff",
    "jk",
    "sr",
    "dlatch",
    "mux2",
    "dec138",
    "counter161",
    "counter163",
    "shift164",
    "shift595",
    "xcvr",
];

pub fn bind(prim: &str, keys: &[(String, String)]) -> Result<Template, String> {
    let prim = prim.trim().to_ascii_lowercase();
    let (prim, open_drain) = match prim.strip_suffix("_od") {
        Some(p) => (p.to_string(), true),
        None => (prim, false),
    };
    let mut seen: Vec<String> = Vec::new();
    for (k, _) in keys {
        let base = split_key(k).0;
        if seen.contains(&base) {
            return Err(format!("pin `{k}` is given twice"));
        }
        seen.push(base);
    }
    let gate = match prim.as_str() {
        "and" => Some((GateOp::And, false)),
        "nand" => Some((GateOp::And, true)),
        "or" => Some((GateOp::Or, false)),
        "nor" => Some((GateOp::Or, true)),
        "xor" => Some((GateOp::Xor, false)),
        "xnor" => Some((GateOp::Xor, true)),
        "not" | "inv" => Some((GateOp::And, true)),
        "buf" | "buffer" => Some((GateOp::And, false)),
        _ => None,
    };
    if let Some((op, invert)) = gate {
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for (k, pin) in keys {
            let (base, inv) = split_key(k);
            let slot = Slot { key: k.clone(), pin: Some(pin.clone()), invert: inv, default: None };
            if base == "Y" { outputs.push(slot) } else { inputs.push(slot) }
        }
        let single = matches!(prim.as_str(), "not" | "inv" | "buf" | "buffer");
        if outputs.len() != 1 {
            return Err(format!("`{prim}` needs one output pin `Y`"));
        }
        if inputs.is_empty() || (single && inputs.len() != 1) {
            let want = if single { "one input" } else { "at least one input" };
            return Err(format!("`{prim}` needs {want} besides `Y`"));
        }
        return Ok(Template { prim: Prim::Gate { op, invert }, inputs, outputs, open_drain });
    }
    if open_drain {
        return Err(format!("`{prim}_od`: only the gates, not and buf have an open-drain form"));
    }
    let Some((p, (ins, outs))) = signature(&prim) else {
        return Err(format!("no primitive `{prim}`; there is {}", PRIMITIVES.join(", ")));
    };
    let mut inputs: Vec<Slot> = ins
        .iter()
        .map(|(k, d)| Slot { key: k.to_string(), pin: None, invert: false, default: *d })
        .collect();
    let mut outputs: Vec<Slot> = outs
        .iter()
        .map(|k| Slot { key: k.to_string(), pin: None, invert: false, default: None })
        .collect();
    for (k, pin) in keys {
        let (base, inv) = split_key(k);
        let mut found = false;
        for slot in inputs.iter_mut().chain(outputs.iter_mut()).filter(|s| s.key == base) {
            slot.pin = Some(pin.clone());
            slot.invert = inv;
            slot.key = k.trim().to_ascii_uppercase();
            found = true;
        }
        if !found {
            let mut all: Vec<&str> = ins.iter().map(|x| x.0).collect();
            for o in outs.iter() {
                if !all.contains(o) {
                    all.push(o);
                }
            }
            return Err(format!("`{prim}` has no pin `{k}`; it has {}", all.join(", ")));
        }
    }
    let missing: Vec<&str> = inputs
        .iter()
        .filter(|s| s.pin.is_none() && s.default.is_none())
        .map(|s| s.key.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(format!("`{prim}` needs pin(s) {}", missing.join(", ")));
    }
    Ok(Template { prim: p, inputs, outputs, open_drain: false })
}

pub fn truth_template(
    inputs: &[String],
    outputs: &[String],
    rows: &[String],
) -> Result<Template, String> {
    if inputs.is_empty() || outputs.is_empty() {
        return Err("a truth table needs `inputs` and `outputs`, lists of pins".into());
    }
    let mut table = Vec::new();
    for r in rows {
        let parts: Vec<&str> = r.split_whitespace().collect();
        let [a, b] = parts.as_slice() else {
            return Err(format!("row `{r}` should be INPUTS OUTPUTS, like \"01- 1\""));
        };
        if a.chars().count() != inputs.len() || b.chars().count() != outputs.len() {
            return Err(format!(
                "row `{r}` needs {} input and {} output character(s)",
                inputs.len(),
                outputs.len()
            ));
        }
        let pat = a
            .chars()
            .map(|c| match c {
                '0' => Ok(Some(false)),
                '1' => Ok(Some(true)),
                '-' => Ok(None),
                _ => Err(format!("row `{r}`: inputs are 0, 1 or -")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let out = b
            .chars()
            .map(|c| Level::from_char(c).ok_or(format!("row `{r}`: outputs are 0, 1, x or z")))
            .collect::<Result<Vec<_>, _>>()?;
        table.push((pat, out));
    }
    if table.is_empty() {
        return Err("the truth table has no rows".into());
    }
    let slot =
        |p: &String| Slot { key: p.clone(), pin: Some(p.clone()), invert: false, default: None };
    Ok(Template {
        prim: Prim::Truth(table),
        inputs: inputs.iter().map(slot).collect(),
        outputs: outputs.iter().map(slot).collect(),
        open_drain: false,
    })
}

fn pin_name_key(name: &str, shape: PinShape) -> (String, bool) {
    let mut neg = matches!(
        shape,
        PinShape::Inverted
            | PinShape::InvertedClock
            | PinShape::InputLow
            | PinShape::ClockLow
            | PinShape::OutputLow
    );
    let mut s = name.trim();
    if let Some(inner) = s.strip_prefix("~{").and_then(|x| x.strip_suffix('}')) {
        neg = true;
        s = inner;
    } else if let Some(x) = s.strip_prefix('~').or(s.strip_prefix('/')).or(s.strip_prefix('!')) {
        neg = true;
        s = x;
    }
    if let Some(x) = s.strip_suffix('#').or(s.strip_suffix("_N")).or(s.strip_suffix("_n")) {
        neg = true;
        s = x;
    }
    let u: String = s.to_ascii_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    match u.as_str() {
        "QN" | "NQ" | "QB" | "QBAR" => ("Q".into(), !neg),
        _ => (u, neg),
    }
}

fn alias(prim: &str, base: &str) -> String {
    let mux = prim == "mux2";
    match base {
        "CLK" | "CK" | "CP" | "C" | "CLOCK" | "T" if !mux => "CLK",
        "SET" | "PRE" | "PR" | "SD" | "S" if !mux => "S",
        "RESET" | "RST" | "CLR" | "CL" | "RD" | "R" | "MR" => "R",
        "E" | "G" | "LE" | "ENABLE" | "GATE" | "EN" if prim == "dlatch" => "EN",
        "E" | "EN" | "ENABLE" if mux => "EN",
        "OUT" | "O" | "Z" | "Y" => "Y",
        "A" | "D0" | "I0" | "IN0" if mux => "I0",
        "B" | "D1" | "I1" | "IN1" if mux => "I1",
        "SEL" | "S" | "S0" if mux => "S",
        "OE" | "EN" | "E" | "G" if prim == "tri" => "OE",
        "IN" | "I" if prim == "tri" => "A",
        other => other,
    }
    .to_string()
}

pub struct PinInfo {
    pub number: String,
    pub name: String,
    pub kind: PinType,
    pub shape: PinShape,
    pub net: Option<usize>,
}

pub fn keys_by_name(prim: &str, pins: &[PinInfo]) -> Vec<(String, String)> {
    let gate = signature(prim).is_none();
    let mut out = Vec::new();
    for p in pins {
        let power = matches!(p.kind, PinType::PowerIn | PinType::PowerOut | PinType::NoConnect);
        let upper = p.name.to_ascii_uppercase();
        if power || ["VCC", "VDD", "GND", "VSS", "VEE"].contains(&upper.as_str()) {
            continue;
        }
        let (base, neg) = pin_name_key(&p.name, p.shape);
        let is_out = matches!(
            p.kind,
            PinType::Output | PinType::TriState | PinType::OpenCollector | PinType::OpenEmitter
        );
        let key = if gate {
            if is_out || matches!(base.as_str(), "Y" | "O" | "OUT" | "Q" | "Z") {
                "Y".to_string()
            } else {
                format!("IN{}", p.number)
            }
        } else if base == "Q" && neg {
            "QN".to_string()
        } else {
            alias(prim, &base)
        };
        let key =
            if neg && !(key == "QN" || (!gate && base == "Q")) { format!("{key}_N") } else { key };
        out.push((key, p.number.clone()));
    }
    out
}

struct RefInfo {
    reference: String,
    value: String,
    symbol: String,
    power: bool,
    pins: Vec<PinInfo>,
}

impl RefInfo {
    fn pin(&self, key: &str) -> Option<&PinInfo> {
        if let Some(p) = self.pins.iter().find(|p| p.number == key) {
            return Some(p);
        }
        let by_name: Vec<&PinInfo> = self.pins.iter().filter(|p| p.name == key).collect();
        (by_name.len() == 1).then(|| by_name[0])
    }
}

fn prefix(r: &str) -> String {
    r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase()
}

const PASSIVES: &[&str] = &["C", "L", "FB", "FL", "D", "LED", "TP", "H", "MH", "FID", "Y", "X"];

struct Built {
    cells: Vec<Cell>,
}

fn instantiate(
    info: &RefInfo,
    t: &Template,
    timing: Timing,
    delays: &BTreeMap<String, u64>,
    skip_missing_unit: bool,
    out: &mut Built,
) -> Result<(), String> {
    let slots = t.inputs.iter().chain(&t.outputs).filter(|s| s.pin.is_some());
    let found =
        slots.clone().filter(|s| info.pin(s.pin.as_deref().unwrap_or("")).is_some()).count();
    if skip_missing_unit && found == 0 {
        return Ok(());
    }
    if !skip_missing_unit {
        for s in slots {
            let p = s.pin.as_deref().unwrap_or("");
            if info.pin(p).is_none() {
                return Err(format!("{} has no pin `{p}` for `{}`", info.reference, s.key));
            }
        }
    }
    let net_of = |s: &Slot| s.pin.as_deref().and_then(|p| info.pin(p)).and_then(|p| p.net);
    let inputs = t
        .inputs
        .iter()
        .map(|s| match (&s.pin, net_of(s)) {
            (Some(_), Some(net)) => Input::Net { net, invert: s.invert },
            (Some(_), None) => Input::Fixed(Level::Z),
            (None, _) => Input::Fixed(s.default.unwrap_or(Level::X)),
        })
        .collect();
    let collector = t.outputs.iter().filter_map(|s| s.pin.as_deref().and_then(|p| info.pin(p)));
    let open_collector = collector.clone().count() > 0
        && collector.clone().all(|p| p.kind == PinType::OpenCollector);
    let outputs = t
        .outputs
        .iter()
        .map(|s| Output {
            net: net_of(s),
            invert: s.invert,
            delay: delays.get(&split_key(&s.key).0).copied().unwrap_or(timing.delay),
        })
        .collect();
    out.cells.push(Cell {
        part: info.reference.clone(),
        prim: t.prim.clone(),
        input_names: t.inputs.iter().map(|s| s.key.clone()).collect(),
        inputs,
        outputs,
        weak: false,
        open_drain: t.open_drain || open_collector,
        setup: timing.setup,
        hold: timing.hold,
        recovery: timing.recovery,
        removal: timing.removal,
    });
    Ok(())
}

fn library_templates(code: &str) -> Vec<Template> {
    library(code)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(prim, keys)| {
            let keys: Vec<(String, String)> =
                keys.into_iter().map(|(k, p)| (k.to_string(), p.to_string())).collect();
            bind(prim, &keys).ok()
        })
        .collect()
}

pub fn resolve(file: &SimFile, sch: &Schematic, d: &mut Diags) -> LogicSpec {
    let mut pin_net: HashMap<PinRef, usize> = HashMap::new();
    for (i, n) in sch.nets.iter().enumerate() {
        for p in &n.pins {
            pin_net.insert(*p, i);
        }
    }
    let mut refs: Vec<RefInfo> = Vec::new();
    for (pi, part) in sch.parts.iter().enumerate() {
        if part.dnp {
            continue;
        }
        let k = match refs.iter().position(|r| r.reference == part.reference) {
            Some(k) => k,
            None => {
                refs.push(RefInfo {
                    reference: part.reference.clone(),
                    value: part.value.clone(),
                    symbol: part.symbol_name.clone(),
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
                None => refs[k].pins.push(PinInfo {
                    number: pin.number.clone(),
                    name: pin.name.clone(),
                    kind: pin.kind,
                    shape: pin.shape,
                    net,
                }),
            }
        }
    }
    let time = |s: &str, at: &str, d: &mut Diags| -> Option<u64> {
        let v = ps(s);
        if v.is_none() {
            d.error(at, format!("cannot read `{s}` as a time, like \"100ns\""));
        }
        v
    };
    let mut rails: Vec<(usize, Level, String)> = Vec::new();
    for (i, n) in sch.nets.iter().enumerate() {
        if n.style == NetStyle::Power
            && let Some(l) = rail_level(&n.name)
        {
            rails.push((i, l, n.name.clone()));
        }
    }
    let mut merges: Vec<(usize, usize)> = Vec::new();
    let mut resistors: Vec<(String, usize, usize)> = Vec::new();
    let mut built = Built { cells: Vec::new() };
    for (i, m) in file.parts.iter().enumerate() {
        let at = format!("parts[{i}]");
        match (&m.reference, &m.value) {
            (Some(r), None) if !refs.iter().any(|x| &x.reference == r) => {
                d.error(&at, format!("no part `{r}` in schematic {}", sch.name))
            }
            (Some(_), None) | (None, Some(_)) => {}
            _ => d.error(&at, "give `ref` or `value`, the part or parts this model covers"),
        }
    }
    for r in &file.ignore {
        if !refs.iter().any(|x| &x.reference == r) {
            d.warn("ignore", format!("no part `{r}` in schematic {}", sch.name));
        }
    }
    for info in &refs {
        let at = format!("part {}", info.reference);
        if file.ignore.contains(&info.reference) {
            continue;
        }
        let inline: Vec<&PartModelFile> = file
            .parts
            .iter()
            .filter(|m| match (&m.reference, &m.value) {
                (Some(r), _) => r == &info.reference,
                (None, Some(v)) => v.eq_ignore_ascii_case(&info.value),
                _ => false,
            })
            .collect();
        let family = part_code(&info.value).or_else(|| part_code(&info.symbol));
        let mut timing = match &family {
            Some((f, _)) => family_timing(f),
            None => GENERIC_TIMING,
        };
        let mut delays: BTreeMap<String, u64> = BTreeMap::new();
        for m in &inline {
            if let Some(v) = m.delay.as_deref().and_then(|s| time(s, &at, d)) {
                timing.delay = v;
            }
            if let Some(v) = m.setup.as_deref().and_then(|s| time(s, &at, d)) {
                timing.setup = v;
            }
            if let Some(v) = m.hold.as_deref().and_then(|s| time(s, &at, d)) {
                timing.hold = v;
            }
            if let Some(v) = m.recovery.as_deref().and_then(|s| time(s, &at, d)) {
                timing.recovery = v;
            }
            if let Some(v) = m.removal.as_deref().and_then(|s| time(s, &at, d)) {
                timing.removal = v;
            }
            for (k, v) in &m.delays {
                if let Some(t) = time(v, &at, d) {
                    delays.insert(split_key(k).0, t);
                }
            }
        }
        let defining: Vec<&&PartModelFile> =
            inline.iter().filter(|m| m.primitive.is_some() || !m.truth.is_empty()).collect();
        if !defining.is_empty() {
            for m in defining {
                let template = if !m.truth.is_empty() {
                    truth_template(&m.inputs, &m.outputs, &m.truth).map(|t| vec![t])
                } else {
                    let p = m.primitive.clone().unwrap_or_default();
                    match part_code(&p) {
                        Some((f, code)) if m.pins.is_empty() => {
                            let custom = [&m.delay, &m.setup, &m.hold, &m.recovery, &m.removal];
                            if custom.iter().all(|x| x.is_none()) {
                                timing = family_timing(&f);
                            }
                            Ok(library_templates(&code))
                        }
                        _ => {
                            let keys: Vec<(String, String)> =
                                m.pins.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                            bind(&p, &keys).map(|t| vec![t])
                        }
                    }
                };
                match template {
                    Ok(ts) => {
                        for t in ts {
                            if let Err(e) =
                                instantiate(info, &t, timing, &delays, false, &mut built)
                            {
                                d.error(&at, e);
                            }
                        }
                    }
                    Err(e) => d.error(&at, e),
                }
            }
            continue;
        }
        if info.power {
            if let Some(l) = rail_level(&info.value) {
                for p in &info.pins {
                    if let Some(n) = p.net {
                        rails.push((n, l, info.value.clone()));
                    }
                }
            }
            continue;
        }
        let pre = prefix(&info.reference);
        let net = |k: &str| info.pin(k).and_then(|p| p.net);
        if pre == "R" && info.pins.len() == 2 {
            if let (Some(a), Some(b)) =
                (net("1").or(info.pins[0].net), net("2").or(info.pins[1].net))
            {
                if parse_value(&info.value) == Some(0.0) {
                    merges.push((a, b));
                } else {
                    resistors.push((info.reference.clone(), a, b));
                }
            }
            continue;
        }
        if matches!(pre.as_str(), "JP" | "SJ" | "NT") {
            let nets: Vec<usize> = if pre == "NT" {
                info.pins.iter().filter_map(|p| p.net).collect()
            } else {
                [net("1"), net("2")].into_iter().flatten().collect()
            };
            for w in nets.windows(2) {
                merges.push((w[0], w[1]));
            }
            continue;
        }
        if PASSIVES.contains(&pre.as_str()) {
            continue;
        }
        if let Some((_, code)) = &family {
            for t in library_templates(code) {
                if let Err(e) = instantiate(info, &t, timing, &delays, true, &mut built) {
                    d.error(&at, e);
                }
            }
            continue;
        }
        if let Some(prim) = generic(&info.value).or_else(|| generic(&info.symbol)) {
            let keys = keys_by_name(prim, &info.pins);
            match bind(prim, &keys) {
                Ok(t) => {
                    if let Err(e) = instantiate(info, &t, timing, &delays, false, &mut built) {
                        d.error(&at, e);
                    }
                }
                Err(e) => d.error(
                    &at,
                    format!(
                        "{} reads as `{prim}` but its pin names do not fit: {e}",
                        info.reference
                    ),
                ),
            }
            continue;
        }
        d.error(
            &at,
            format!(
                "{} ({}) has no logic model; list it in `ignore` or give it a [[parts]] model",
                info.reference, info.value
            ),
        );
    }
    let n = sch.nets.len();
    let mut uf = UnionFind::new(n);
    for (a, b) in &merges {
        uf.union(*a, *b);
    }
    let rail_of = |uf: &mut UnionFind, net: usize| -> Option<Level> {
        let r = uf.find(net);
        rails.iter().find(|x| uf.find(x.0) == r).map(|x| x.1)
    };
    let mut pulls: Vec<(String, usize, Level)> = Vec::new();
    for (r, a, b) in &resistors {
        match (rail_of(&mut uf, *a), rail_of(&mut uf, *b)) {
            (Some(_), Some(_)) => {}
            (None, None) => uf.union(*a, *b),
            (Some(l), None) => pulls.push((r.clone(), *b, l)),
            (None, Some(l)) => pulls.push((r.clone(), *a, l)),
        }
    }
    let mut compact: HashMap<usize, usize> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut alias: Vec<(String, usize)> = Vec::new();
    for i in 0..n {
        let root = uf.find(i);
        let id = *compact.entry(root).or_insert_with(|| {
            names.push(sch.nets[i].name.clone());
            names.len() - 1
        });
        alias.push((sch.nets[i].name.clone(), id));
    }
    let map = |uf: &mut UnionFind, net: usize| compact[&uf.find(net)];
    let mut rail_level_of: BTreeMap<usize, (Level, String)> = BTreeMap::new();
    for (net, l, name) in &rails {
        let id = map(&mut uf, *net);
        match rail_level_of.get(&id) {
            Some((other, oname)) if other != l => d.error(
                "rails",
                format!(
                    "net {} joins rail {oname} and rail {name} through a 0 ohm link",
                    names[id]
                ),
            ),
            Some(_) => {}
            None => {
                rail_level_of.insert(id, (*l, name.clone()));
            }
        }
    }
    let find_net = |name: &str| alias.iter().find(|(n, _)| n == name).map(|x| x.1);
    let mut stimuli = Vec::new();
    for (i, s) in file.stimulus.iter().enumerate() {
        let at = format!("stimulus[{i}] {}", s.net);
        let Some(net) = find_net(&s.net) else {
            d.error(&at, format!("no net `{}` in schematic {}", s.net, sch.name));
            continue;
        };
        let given =
            s.clock.is_some() as u8 + !s.steps.is_empty() as u8 + s.constant.is_some() as u8;
        if given != 1 {
            d.error(&at, "give exactly one of `clock`, `steps` or `constant`");
            continue;
        }
        let wave = if let Some(c) = &s.clock {
            let Some(period) = time(&c.period, &at, d) else { continue };
            let phase = match &c.phase {
                Some(p) => match time(p, &at, d) {
                    Some(v) => v,
                    None => continue,
                },
                None => 0,
            };
            let duty = c.duty.unwrap_or(0.5);
            if period == 0 || !(duty > 0.0 && duty < 1.0) {
                d.error(&at, "a clock needs a period above zero and a duty between 0 and 1");
                continue;
            }
            let high = ((period as f64 * duty).round() as u64).clamp(1, period - 1);
            Wave::Clock { period, high, phase }
        } else if let Some(c) = &s.constant {
            match level(c) {
                Ok(l) => Wave::Steps(vec![(0, l)]),
                Err(e) => {
                    d.error(&at, e);
                    continue;
                }
            }
        } else {
            let mut steps = Vec::new();
            for (t, v) in &s.steps {
                match (time(t, &at, d), level(v)) {
                    (Some(t), Ok(l)) => steps.push((t, l)),
                    (_, Err(e)) => d.error(&at, e),
                    _ => {}
                }
            }
            if steps.windows(2).any(|w| w[1].0 < w[0].0) {
                d.error(&at, "steps must be in time order");
            }
            Wave::Steps(steps)
        };
        if stimuli.iter().any(|x: &Stimulus| x.net == net) {
            d.error(&at, format!("net `{}` already has a stimulus", s.net));
            continue;
        }
        stimuli.push(Stimulus { net, name: s.net.clone(), wave });
    }
    let mut cells: Vec<Cell> = Vec::new();
    for (id, (l, name)) in &rail_level_of {
        if stimuli.iter().any(|s| s.net == *id) {
            continue;
        }
        cells.push(Cell {
            part: name.clone(),
            prim: Prim::Const(*l),
            input_names: Vec::new(),
            inputs: Vec::new(),
            outputs: vec![Output { net: Some(*id), invert: false, delay: 0 }],
            weak: false,
            open_drain: false,
            setup: 0,
            hold: 0,
            recovery: 0,
            removal: 0,
        });
    }
    for (r, net, l) in &pulls {
        cells.push(Cell {
            part: r.clone(),
            prim: Prim::Const(*l),
            input_names: Vec::new(),
            inputs: Vec::new(),
            outputs: vec![Output { net: Some(map(&mut uf, *net)), invert: false, delay: 0 }],
            weak: true,
            open_drain: false,
            setup: 0,
            hold: 0,
            recovery: 0,
            removal: 0,
        });
    }
    for mut c in built.cells {
        for i in &mut c.inputs {
            if let Input::Net { net, .. } = i {
                *net = map(&mut uf, *net);
            }
        }
        for o in &mut c.outputs {
            o.net = o.net.map(|x| map(&mut uf, x));
        }
        cells.push(c);
    }
    let duration = match file.duration.as_deref() {
        Some(s) => time(s, "duration", d).unwrap_or(0),
        None => {
            d.error("duration", "give `duration`, how long to simulate, like \"2us\"");
            0
        }
    };
    let mut record = Vec::new();
    let mut buses = Vec::new();
    if file.record.is_empty() {
        for (id, name) in names.iter().enumerate() {
            if !rail_level_of.contains_key(&id) {
                record.push((name.clone(), id));
            }
        }
    } else {
        for r in &file.record {
            let (nets, bus) = match r {
                RecordFile::Net(n) => (std::slice::from_ref(n), None),
                RecordFile::Bus { name, nets } => (nets.as_slice(), Some(name)),
            };
            if let Some(name) = bus
                && nets.is_empty()
            {
                d.error("record", format!("bus `{name}` needs `nets`, MSB first"));
                continue;
            }
            let mut ok = true;
            for n in nets {
                if record.iter().any(|x: &(String, usize)| &x.0 == n) {
                    continue;
                }
                match find_net(n) {
                    Some(id) => record.push((n.clone(), id)),
                    None => {
                        ok = false;
                        d.error("record", format!("no net `{n}` in schematic {}", sch.name))
                    }
                }
            }
            if let Some(name) = bus
                && ok
            {
                buses.push(Bus { name: name.clone(), nets: nets.to_vec() });
            }
        }
    }
    let mut expects = Vec::new();
    for (i, e) in file.expect.iter().enumerate() {
        let names_given: Vec<String> = match (&e.net, e.nets.is_empty()) {
            (Some(n), true) => vec![n.clone()],
            (None, false) => e.nets.clone(),
            _ => {
                d.error(format!("expect[{i}]"), "give `net`, or `nets` for a bus (MSB first)");
                continue;
            }
        };
        let label = format!("expect[{i}] {}", names_given.join(","));
        let nets: Vec<Option<usize>> = names_given.iter().map(|n| find_net(n)).collect();
        if let Some(k) = nets.iter().position(Option::is_none) {
            d.error(&label, format!("no net `{}` in schematic {}", names_given[k], sch.name));
            continue;
        }
        let nets: Vec<usize> = nets.into_iter().flatten().collect();
        let w = nets.len();
        let check = match (&e.at, &e.value, &e.clock, e.sequence.is_empty()) {
            (Some(t), Some(v), None, true) => {
                let Some(time) = time(t, &label, d) else { continue };
                match pattern(v, w) {
                    Ok(value) => Check::At { time, value },
                    Err(err) => {
                        d.error(&label, err);
                        continue;
                    }
                }
            }
            (None, None, Some(c), false) => {
                let Some(clock) = find_net(c) else {
                    d.error(&label, format!("no clock net `{c}` in schematic {}", sch.name));
                    continue;
                };
                let from = match e.from.as_deref() {
                    Some(f) => match time(f, &label, d) {
                        Some(v) => v,
                        None => continue,
                    },
                    None => 0,
                };
                let values: Result<Vec<_>, String> =
                    e.sequence.iter().map(|v| pattern(v, w)).collect();
                match values {
                    Ok(values) => Check::Sequence {
                        clock,
                        clock_name: c.clone(),
                        edge: e.edge.unwrap_or_default(),
                        from,
                        values,
                    },
                    Err(err) => {
                        d.error(&label, err);
                        continue;
                    }
                }
            }
            _ => {
                d.error(
                    &label,
                    "give `at` and `value`, or `clock` and `sequence` (with optional `edge` and `from`)",
                );
                continue;
            }
        };
        if let Check::At { time, .. } = &check
            && *time > duration
        {
            d.error(&label, format!("{} is past the duration", fmt_time(*time)));
        }
        expects.push(Expect { label, nets, names: names_given, check });
    }
    LogicSpec {
        schematic: sch.name.clone(),
        duration,
        record,
        buses,
        on_violation: file.on_violation.unwrap_or_default(),
        stimuli,
        expects,
        circuit: Circuit { nets: names, cells },
        netlist_hash: netlist_hash(sch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_codes_find_the_function() {
        assert_eq!(part_code("74HC161"), Some(("HC".into(), "161".into())));
        assert_eq!(part_code("SN74LVC1G08DBVR"), Some(("LVC".into(), "1G08".into())));
        assert_eq!(part_code("CD74HCT04E"), Some(("HCT".into(), "04".into())));
        assert_eq!(part_code("74LVC2G74"), Some(("LVC".into(), "2G74".into())));
        assert_eq!(part_code("SN74HC595N"), Some(("HC".into(), "595".into())));
        assert_eq!(part_code("7400"), Some(("".into(), "00".into())));
        assert_eq!(part_code("LM7805"), None);
        assert_eq!(part_code("STM32F074"), None);
    }

    #[test]
    fn bus_bits_read_brackets_digits_and_active_low() {
        assert_eq!(bus_bit("D[12]"), Some(("D".into(), 12, "".into())));
        assert_eq!(bus_bit("Q3"), Some(("Q".into(), 3, "".into())));
        assert_eq!(bus_bit("Y7_N"), Some(("Y".into(), 7, "_N".into())));
        assert_eq!(bus_bit("CLK"), None);
        assert_eq!(bus_bit("5V"), None);
        assert_eq!(bus_bit("U1_OUT"), None);
    }

    #[test]
    fn family_recovery_and_removal_follow_the_datasheets() {
        for (family, recovery, removal, source) in [
            ("HC", 8_000, 0, "Nexperia 74HC_HCT74 rev 9 trec at 4.5 V"),
            ("HCT", 8_000, 0, "Nexperia 74HC_HCT74 rev 9 trec at 4.5 V"),
            ("AHC", 3_500, 0, "Nexperia 74AHC_AHCT74 rev 11 trec, AHCT at 4.5 V"),
            ("AC", 2_400, 0, "TI CD74AC74 SCHS231E trec at 5 V"),
            ("LVC", 2_000, 0, "TI SN74LVC74A SCAS287W tsu PRE or CLR inactive at 3.3 V"),
            ("LS", 25_000, 3_000, "TI SN74LS161A SDLS060 tsu CLR inactive, th any input"),
        ] {
            let t = family_timing(family);
            assert_eq!((t.recovery, t.removal), (recovery, removal), "{family} per {source}");
        }
    }

    #[test]
    fn x01_has_the_7401_pinout_in_ttl_and_cmos() {
        let pin = |s: &Slot| s.pin.clone().unwrap_or_default();
        for (part, source) in [
            ("SN7401N", "TI SDLS026 SN7401/SN74LS01"),
            ("SN74LS01N", "TI SDLS026 SN7401/SN74LS01"),
            ("HD74HC01P", "Renesas REJ03D0532 HD74HC01"),
        ] {
            let (_, code) = part_code(part).unwrap();
            let gates: Vec<(String, String, String)> = library_templates(&code)
                .iter()
                .map(|t| (pin(&t.inputs[0]), pin(&t.inputs[1]), pin(&t.outputs[0])))
                .collect();
            let expect = [("2", "3", "1"), ("5", "6", "4"), ("8", "9", "10"), ("11", "12", "13")]
                .map(|(a, b, y)| (a.to_string(), b.to_string(), y.to_string()));
            assert_eq!(gates, expect, "{part} per {source}");
        }
    }

    #[test]
    fn every_library_part_binds() {
        for code in [
            "00", "02", "04", "08", "10", "11", "14", "20", "21", "27", "32", "74", "86", "125",
            "126", "132", "138", "157", "161", "163", "164", "244", "595", "1G00", "1G02", "1G04",
            "1G08", "1G14", "1G17", "1G32", "1G34", "1G74", "1G79", "1G80", "1G86", "1G125",
            "1G126", "1G157", "2G00", "2G02", "2G04", "2G08", "2G14", "2G17", "2G32", "2G34",
            "2G74", "2G86", "2G125", "2G126", "01", "03", "05", "06", "07", "245", "573", "574",
            "1G06", "1G07",
        ] {
            let units = library(code).unwrap();
            assert_eq!(library_templates(code).len(), units.len(), "{code}");
        }
    }

    #[test]
    fn octal_open_drain_and_schmitt_parts_bind() {
        let pin = |s: &Slot| s.pin.clone().unwrap_or_default();
        let latch = library_templates("573");
        assert_eq!(latch.len(), 8);
        assert_eq!(latch[0].prim, Prim::Dlatch);
        let l0: Vec<(String, String, bool)> =
            latch[0].inputs.iter().map(|s| (s.key.clone(), pin(s), s.invert)).collect();
        assert_eq!(l0[0], ("D".into(), "2".into(), false));
        assert_eq!(l0[1], ("EN".into(), "11".into(), false));
        assert_eq!(l0[3], ("OE_N".into(), "1".into(), true));
        assert_eq!(pin(&latch[0].outputs[0]), "19");
        assert_eq!(
            (pin(&latch[7].inputs[0]), pin(&latch[7].outputs[0])),
            ("9".into(), "12".into())
        );
        let reg = library_templates("574");
        assert_eq!(reg[3].prim, Prim::Dff);
        assert_eq!((pin(&reg[3].inputs[1]), pin(&reg[3].outputs[0])), ("11".into(), "16".into()));
        let x = library_templates("245");
        assert_eq!(x.len(), 8);
        assert_eq!(x[0].prim, Prim::Xcvr);
        let ins: Vec<String> = x[0].inputs.iter().map(pin).collect();
        assert_eq!(ins, ["2", "18", "1", "19"]);
        assert!(x[0].inputs[3].invert);
        let outs: Vec<String> = x[7].outputs.iter().map(pin).collect();
        assert_eq!(outs, ["9", "11"]);
        for code in ["01", "03", "05", "06", "07", "1G06", "1G07"] {
            assert!(library_templates(code).iter().all(|t| t.open_drain), "{code}");
        }
        assert!(!library_templates("00")[0].open_drain);
        let nand = Prim::Gate { op: GateOp::And, invert: true };
        assert_eq!(library_templates("03")[0].prim, nand);
        assert_eq!(pin(&library_templates("03")[0].outputs[0]), "3");
        let buf = &library_templates("1G07")[0];
        assert_eq!(buf.prim, Prim::Gate { op: GateOp::And, invert: false });
        assert_eq!((pin(&buf.inputs[0]), pin(&buf.outputs[0])), ("2".into(), "4".into()));
        let schmitt = library_templates("14");
        assert_eq!(schmitt.len(), 6);
        assert!(schmitt.iter().all(|t| t.prim == nand && !t.open_drain));
        assert_eq!(part_code("SN74HC14N"), Some(("HC".into(), "14".into())));
        assert_eq!(part_code("74LVC1G07GW"), Some(("LVC".into(), "1G07".into())));
        assert_eq!(part_code("SN74HC573AN"), Some(("HC".into(), "573".into())));
        let k = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        };
        let od = bind("xor_od", &k(&[("A", "1"), ("B", "2"), ("Y", "3")])).unwrap();
        assert!(od.open_drain);
        assert!(
            bind("dff_od", &k(&[("D", "1"), ("CLK", "2")])).unwrap_err().contains("open-drain")
        );
        let err = bind("xcvr", &k(&[("A", "1"), ("C", "2")])).unwrap_err();
        assert!(err.ends_with("it has A, B, DIR, OE"), "{err}");
    }

    #[test]
    fn keys_bind_with_polarity() {
        let k = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        };
        let t = bind("dff", &k(&[("D", "1"), ("CLK_N", "2"), ("R_N", "3"), ("Q", "4")])).unwrap();
        assert_eq!(t.prim, Prim::Dff);
        assert!(t.inputs[1].invert);
        assert_eq!(t.inputs[3].pin.as_deref(), Some("3"));
        assert_eq!(t.inputs[2].default, Some(Level::L));
        assert!(bind("dff", &k(&[("CLK", "2")])).unwrap_err().contains("D"));
        assert!(bind("dff", &k(&[("D", "1"), ("CLK", "2"), ("FOO", "3")])).is_err());
        let g = bind("nand", &k(&[("A", "1"), ("B", "2"), ("C", "3"), ("Y", "4")])).unwrap();
        assert_eq!(g.prim, Prim::Gate { op: GateOp::And, invert: true });
        assert_eq!(g.inputs.len(), 3);
        assert!(bind("not", &k(&[("A", "1"), ("B", "2"), ("Y", "3")])).is_err());
        assert!(bind("warp", &k(&[])).is_err());
    }

    #[test]
    fn truth_rows_parse() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let t = truth_template(&s(&["1", "2"]), &s(&["3"]), &s(&["0- 0", "10 1", "11 z"])).unwrap();
        let Prim::Truth(rows) = &t.prim else { panic!() };
        assert_eq!(rows[0], (vec![Some(false), None], vec![Level::L]));
        assert_eq!(rows[2].1, vec![Level::Z]);
        assert!(truth_template(&s(&["1"]), &s(&["3"]), &s(&["01 1"])).is_err());
    }

    #[test]
    fn patterns_read_numbers_and_text() {
        use Level::*;
        assert_eq!(
            pattern(&LevelFile::Number(5), 4).unwrap(),
            vec![Some(L), Some(H), Some(L), Some(H)]
        );
        assert_eq!(
            pattern(&LevelFile::Text("1-x_z".into()), 4).unwrap(),
            vec![Some(H), None, Some(X), Some(Z)]
        );
        assert!(pattern(&LevelFile::Number(16), 4).is_err());
        assert!(pattern(&LevelFile::Text("01".into()), 3).is_err());
        assert_eq!(fmt_time(250_000), "250ns");
        assert_eq!(fmt_time(1_500_000), "1.5us");
        assert_eq!(ps("2us"), Some(2_000_000));
    }

    #[test]
    fn generic_symbols_map_by_pin_name() {
        let pin = |n: &str, name: &str, kind: PinType, shape: PinShape| PinInfo {
            number: n.into(),
            name: name.into(),
            kind,
            shape,
            net: None,
        };
        let pins = vec![
            pin("1", "D", PinType::Input, PinShape::Line),
            pin("2", "C", PinType::Input, PinShape::Clock),
            pin("3", "~{R}", PinType::Input, PinShape::Line),
            pin("4", "Q", PinType::Output, PinShape::Line),
            pin("5", "~{Q}", PinType::Output, PinShape::Line),
            pin("6", "VCC", PinType::PowerIn, PinShape::Line),
        ];
        let keys = keys_by_name("dff", &pins);
        assert_eq!(
            keys,
            vec![
                ("D".to_string(), "1".to_string()),
                ("CLK".into(), "2".into()),
                ("R_N".into(), "3".into()),
                ("Q".into(), "4".into()),
                ("QN".into(), "5".into())
            ]
        );
        assert!(bind("dff", &keys).is_ok());
        let gate = vec![
            pin("1", "A", PinType::Input, PinShape::Line),
            pin("2", "B", PinType::Input, PinShape::Line),
            pin("3", "Y", PinType::Output, PinShape::Inverted),
        ];
        let keys = keys_by_name("and", &gate);
        assert_eq!(keys[2].0, "Y_N");
        assert_eq!(generic("NAND3"), Some("nand"));
        assert_eq!(generic("D_FF"), Some("dff"));
        assert_eq!(generic("LM358"), None);
        assert_eq!(generic("DFF"), Some("dff"));
    }

    fn fixture(tag: &str, sim: &str) -> crate::Project {
        let dir = std::env::temp_dir().join(format!("agentee-logic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        w(
            "74HC00.sym.toml",
            r#"name = "74HC00"
reference = "U"
[[bodies]]
left = [{ number = "1", name = "1A", type = "input" }, { number = "2", name = "1B", type = "input" }, { number = "4", name = "2A", type = "input" }, { number = "5", name = "2B", type = "input" }]
right = [{ number = "3", name = "1Y", type = "output" }, { number = "6", name = "2Y", type = "output" }]
top = [{ number = "14", name = "VCC", type = "power_in" }]
bottom = [{ number = "7", name = "GND", type = "power_in" }]
"#,
        );
        w(
            "CHIP.sym.toml",
            r#"name = "CHIP"
reference = "U"
[[bodies]]
left = [{ number = "1", name = "P", type = "input" }, { number = "2", name = "Q", type = "input" }]
right = [{ number = "3", name = "O", type = "output" }]
"#,
        );
        w(
            "OCNAND.sym.toml",
            r#"name = "OCNAND"
reference = "U"
[[bodies]]
left = [{ number = "1", name = "A", type = "input" }, { number = "2", name = "B", type = "input" }]
right = [{ number = "3", name = "Y", type = "open_collector", shape = "inverted" }]
"#,
        );
        w(
            "R.sym.toml",
            r#"name = "R"
reference = "R"
[[bodies]]
left = [{ number = "1", name = "~", type = "passive" }]
right = [{ number = "2", name = "~", type = "passive" }]
"#,
        );
        w(
            "logic.sch.toml",
            r#"name = "logic"
[[parts]]
ref = "U1"
symbol = "74HC00"
at = [50.8, 50.8]
[[parts]]
ref = "U2"
symbol = "CHIP"
value = "MYXOR"
at = [101.6, 50.8]
[[parts]]
ref = "U3"
symbol = "CHIP"
value = "MYNAND"
at = [101.6, 101.6]
[[parts]]
ref = "U4"
symbol = "CHIP"
value = "MYSTERY"
at = [152.4, 101.6]
[[parts]]
ref = "U5"
symbol = "OCNAND"
value = "AND"
at = [152.4, 50.8]
[[parts]]
ref = "R1"
symbol = "R"
value = "10k"
at = [25.4, 25.4]
[[parts]]
ref = "R2"
symbol = "R"
value = "0"
at = [25.4, 76.2]
[[nets]]
name = "VCC"
style = "power"
pins = ["U1.14", "R1.1"]
[[nets]]
name = "GND"
style = "power"
pins = ["U1.7"]
[[nets]]
name = "A"
pins = ["U1.1", "U2.1", "U3.1", "U4.1", "U5.1"]
[[nets]]
name = "B_SRC"
pins = ["R2.1"]
[[nets]]
name = "B"
pins = ["R2.2", "U1.2", "U2.2", "U3.2", "U4.2", "U5.2"]
[[nets]]
name = "PULLED"
pins = ["R1.2", "U1.4", "U1.5", "U5.3"]
[[nets]]
name = "Y"
pins = ["U1.3"]
[[nets]]
name = "Y2"
pins = ["U1.6"]
[[nets]]
name = "X"
pins = ["U2.3"]
[[nets]]
name = "N"
pins = ["U3.3"]
[[nets]]
name = "M"
pins = ["U4.3"]
"#,
        );
        w("t.sim.toml", sim);
        crate::Project::load(&dir).unwrap()
    }

    const PARTS: &str = r#"
[[parts]]
value = "MYXOR"
inputs = ["1", "2"]
outputs = ["3"]
truth = ["00 0", "01 1", "10 1", "11 0"]
delay = "2ns"

[[parts]]
ref = "U3"
primitive = "nand"
pins = { A = "1", B = "2", Y = "O" }
"#;

    #[test]
    fn logic_sim_file_resolves_against_the_schematic() {
        let sim = format!(
            r#"name = "t"
kind = "logic"
duration = "1us"
ignore = ["U4"]
record = ["A", "B", "Y"]

[[stimulus]]
net = "A"
clock = {{ period = "100ns", duty = 0.25, phase = "10ns" }}

[[stimulus]]
net = "B_SRC"
steps = [["0ns", 0], ["500ns", "1"]]

[[expect]]
net = "Y"
at = "200ns"
value = 1

[[expect]]
nets = ["X", "N"]
clock = "A"
edge = "falling"
from = "100ns"
sequence = ["1-", 3]
{PARTS}"#
        );
        let p = fixture("ok", &sim);
        assert!(p.failures.is_empty(), "{:?}", p.failures);
        let e = &p.sims[0];
        let errors: Vec<&String> = e
            .diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .map(|d| &d.message)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let l = e.item.logic.as_ref().unwrap();
        assert_eq!(l.duration, 1_000_000);
        let net = |n: &str| l.circuit.nets.iter().position(|x| x == n).unwrap();
        assert!(!l.circuit.nets.contains(&"B".to_string()));
        assert_eq!(l.stimuli[1].net, net("B_SRC"));
        assert_eq!(l.record[1], ("B".to_string(), net("B_SRC")));
        assert_eq!(l.stimuli[0].wave, Wave::Clock { period: 100_000, high: 25_000, phase: 10_000 });
        assert_eq!(l.stimuli[1].wave, Wave::Steps(vec![(0, Level::L), (500_000, Level::H)]));
        let pull = l.circuit.cells.iter().find(|c| c.part == "R1").unwrap();
        assert!(pull.weak);
        assert_eq!(pull.prim, Prim::Const(Level::H));
        assert_eq!(pull.outputs[0].net, Some(net("PULLED")));
        let nands: Vec<&Cell> = l.circuit.cells.iter().filter(|c| c.part == "U1").collect();
        assert_eq!(nands.len(), 2);
        assert_eq!(nands[0].outputs[0].delay, 10_000);
        assert_eq!(nands[0].setup, 15_000);
        assert!(nands.iter().all(|c| c.prim == Prim::Gate { op: GateOp::And, invert: true }));
        let xor = l.circuit.cells.iter().find(|c| c.part == "U2").unwrap();
        assert!(matches!(xor.prim, Prim::Truth(_)));
        assert_eq!(xor.outputs[0].delay, 2_000);
        let u3 = l.circuit.cells.iter().find(|c| c.part == "U3").unwrap();
        assert_eq!(u3.outputs[0].net, Some(net("N")));
        let u5 = l.circuit.cells.iter().find(|c| c.part == "U5").unwrap();
        assert!(u5.open_drain && u5.outputs[0].invert);
        assert!(!nands[0].open_drain);
        assert_eq!(l.on_violation, OnViolation::X);
        assert!(l.buses.is_empty());
        assert!(!l.circuit.cells.iter().any(|c| c.part == "U4"));
        assert_eq!(l.record.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["A", "B", "Y"]);
        assert_eq!(l.expects[0].check, Check::At { time: 200_000, value: vec![Some(Level::H)] });
        let Check::Sequence { edge, from, values, .. } = &l.expects[1].check else { panic!() };
        assert_eq!((*edge, *from), (Edge::Falling, 100_000));
        assert_eq!(values[0], vec![Some(Level::H), None]);
        assert_eq!(values[1], vec![Some(Level::H), Some(Level::H)]);
    }

    #[test]
    fn logic_sim_file_reads_buses_violation_mode_and_reset_timing() {
        let sim = format!(
            r#"name = "t"
kind = "logic"
duration = "1us"
ignore = ["U4"]
on_violation = "keep"
record = ["A", {{ name = "OUT", nets = ["X", "N"] }}, "X", {{ name = "BAD", nets = ["A", "NOPE"] }}]

[[parts]]
ref = "U1"
recovery = "7ns"
removal = "2ns"
{PARTS}"#
        );
        let p = fixture("bus", &sim);
        let e = &p.sims[0];
        let errors: Vec<&String> = e
            .diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .map(|d| &d.message)
            .collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("no net `NOPE`"));
        let l = e.item.logic.as_ref().unwrap();
        assert_eq!(l.on_violation, OnViolation::Keep);
        assert_eq!(l.buses, vec![Bus { name: "OUT".into(), nets: vec!["X".into(), "N".into()] }]);
        let names: Vec<&str> = l.record.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, ["A", "X", "N"]);
        let u1 = l.circuit.cells.iter().find(|c| c.part == "U1").unwrap();
        assert_eq!((u1.setup, u1.hold, u1.recovery, u1.removal), (15_000, 3_000, 7_000, 2_000));
        let u3 = l.circuit.cells.iter().find(|c| c.part == "U3").unwrap();
        assert_eq!((u3.recovery, u3.removal), (0, 0));
    }

    #[test]
    fn logic_sim_file_reports_what_it_cannot_use() {
        let sim = format!(
            r#"name = "t"
kind = "logic"
duration = "soon"
record = ["NOPE"]

[[stimulus]]
net = "A"
constant = 1
clock = {{ period = "10ns" }}

[[stimulus]]
net = "B"
steps = [["5ns", 1], ["2ns", 0]]

[[expect]]
net = "Y"
at = "20ns"

[[expect]]
nets = ["X", "N"]
at = "20ns"
value = 7
{PARTS}"#
        );
        let p = fixture("bad", &sim);
        let msgs: Vec<String> = p.sims[0]
            .diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .map(|d| format!("{}: {}", d.at, d.message))
            .collect();
        let has = |s: &str| msgs.iter().any(|m| m.contains(s));
        assert!(has("U4 (MYSTERY) has no logic model"), "{msgs:?}");
        assert!(has("duration: cannot read `soon`"), "{msgs:?}");
        assert!(has("record: no net `NOPE`"), "{msgs:?}");
        assert!(has("exactly one of `clock`, `steps` or `constant`"), "{msgs:?}");
        assert!(has("steps must be in time order"), "{msgs:?}");
        assert!(has("give `at` and `value`"), "{msgs:?}");
        assert!(has("7 does not fit in 2 bit(s)"), "{msgs:?}");
        let stray = fixture(
            "stray",
            "name = \"t\"\nkind = \"logic\"\nlayout = \"x\"\nduration = \"1us\"\nignore = [\"U2\", \"U3\", \"U4\"]\n",
        );
        let msgs: Vec<&str> = stray.sims[0].diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.starts_with("a logic sim runs on the schematic")),
            "{msgs:?}"
        );
    }
}

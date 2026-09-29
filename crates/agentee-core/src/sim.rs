use crate::diag::Diags;
use crate::layout::Layout;
use crate::units::Length;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrequencyFile {
    pub start: String,
    pub stop: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortFile {
    pub name: String,
    pub pad: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impedance: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacitor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inductor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistor: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SimKind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<FrequencyFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excite: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<PortFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_top: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_bottom: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<HeatFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supplies: Vec<SupplyFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loads: Vec<LoadFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub far_field: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<DeviceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub report: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<StageFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceFile {
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub file: String,
    pub ports: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<crate::rf::Mount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datasheet: Vec<DatasheetFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasheetFile {
    pub freq: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nf: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oip3: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p1db: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DatasheetRow {
    pub freq: f64,
    pub nf: Option<f64>,
    pub oip3: Option<f64>,
    pub p1db: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageFile {
    pub nf: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iip3: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p1db_in: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub reference: String,
    pub file: String,
    pub ports: Vec<String>,
    pub mount: Option<crate::rf::Mount>,
    pub datasheet: Vec<DatasheetRow>,
}

pub fn datasheet_at(
    rows: &[DatasheetRow],
    f: f64,
    pick: fn(&DatasheetRow) -> Option<f64>,
) -> Option<f64> {
    let pts: Vec<(f64, f64)> = rows.iter().filter_map(|r| pick(r).map(|v| (r.freq, v))).collect();
    let (first, last) = (pts.first()?, pts.last()?);
    if f < first.0 * (1.0 - 1e-9) || f > last.0 * (1.0 + 1e-9) {
        return None;
    }
    let k = pts.partition_point(|p| p.0 < f).min(pts.len() - 1);
    if k == 0 || pts[k].0 == f {
        return Some(pts[k].1);
    }
    let ((f0, a), (f1, b)) = (pts[k - 1], pts[k]);
    Some(a + (b - a) * (f - f0) / (f1 - f0))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimKind {
    #[default]
    Fdtd,
    Dc,
    Thermal,
    Cascade,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatFile {
    #[serde(rename = "ref")]
    pub reference: String,
    pub power: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theta_jc: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyFile {
    pub pad: String,
    pub voltage: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadFile {
    pub pad: String,
    pub current: String,
    #[serde(default, rename = "return", skip_serializing_if = "Option::is_none")]
    pub return_pad: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkFile {
    #[serde(rename = "ref")]
    pub reference: String,
    pub resistance: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct HeatSource {
    pub part: usize,
    pub reference: String,
    pub watts: f64,
    pub theta_jc: Option<f64>,
    pub pads: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PadRef {
    pub part: usize,
    pub pad: usize,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Supply {
    pub pad: PadRef,
    pub volts: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Load {
    pub pad: PadRef,
    pub amps: f64,
    pub return_pad: Option<PadRef>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub reference: String,
    pub a: PadRef,
    pub b: PadRef,
    pub ohms: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct MapGrid {
    pub origin: [f64; 2],
    pub cell: f64,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerMap {
    pub layer: String,
    pub quantity: String,
    pub unit: String,
    pub origin: [f64; 2],
    pub cell: f64,
    pub width: usize,
    pub height: usize,
    pub min: f32,
    pub max: f32,
    pub data: String,
}

impl LayerMap {
    pub fn encode(
        layer: &str,
        quantity: &str,
        unit: &str,
        grid: MapGrid,
        values: &[f32],
    ) -> LayerMap {
        let MapGrid { origin, cell, width, height } = grid;
        let finite = values.iter().filter(|v| v.is_finite());
        let min = finite.clone().fold(f32::MAX, |a, b| a.min(*b));
        let max = finite.fold(f32::MIN, |a, b| a.max(*b));
        let (min, max) = if min > max { (0.0, 0.0) } else { (min, max) };
        let span = (max - min).max(1e-30);
        let mut bytes = Vec::with_capacity(values.len() * 2);
        for v in values {
            let q: u16 = if v.is_finite() {
                1 + (((v - min) / span) * 65534.0).round().clamp(0.0, 65534.0) as u16
            } else {
                0
            };
            bytes.extend_from_slice(&q.to_le_bytes());
        }
        LayerMap {
            layer: layer.into(),
            quantity: quantity.into(),
            unit: unit.into(),
            origin,
            cell,
            width,
            height,
            min,
            max,
            data: base64(&bytes),
        }
    }

    pub fn decode(&self) -> Vec<f32> {
        let bytes = unbase64(&self.data);
        let span = self.max - self.min;
        bytes
            .chunks_exact(2)
            .map(|b| {
                let q = u16::from_le_bytes([b[0], b[1]]);
                if q == 0 { f32::NAN } else { self.min + (q - 1) as f32 / 65534.0 * span }
            })
            .collect()
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                out.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn unbase64(s: &str) -> Vec<u8> {
    let val = |c: u8| B64.iter().position(|b| *b == c).unwrap_or(0) as u32;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for c in bytes.chunks(4) {
        if c.len() < 4 {
            break;
        }
        let n =
            c.iter().take(4).fold(0u32, |acc, b| acc << 6 | if *b == b'=' { 0 } else { val(*b) });
        out.push((n >> 16) as u8);
        if c[2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if c[3] != b'=' {
            out.push(n as u8);
        }
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reading {
    pub label: String,
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapResult {
    pub name: String,
    pub kind: SimKind,
    pub maps: Vec<LayerMap>,
    pub readings: Vec<Reading>,
    pub iterations: usize,
    pub residual: f64,
    pub cells: usize,
    pub seconds: f64,
    pub device: String,
    pub spec_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Model {
    Capacitor(f64),
    Inductor(f64),
    Resistor(f64),
    Open,
}

#[derive(Clone, Debug, Serialize)]
pub struct Port {
    pub name: String,
    pub part: usize,
    pub pad: usize,
    pub at: [f64; 2],
    pub layer: String,
    pub reference: String,
    pub impedance: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Element {
    pub part: usize,
    pub reference: String,
    pub model: Model,
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub layer: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sim {
    pub name: String,
    pub kind: SimKind,
    pub ambient: f64,
    pub h_top: f64,
    pub h_bottom: f64,
    pub sources: Vec<HeatSource>,
    pub supplies: Vec<Supply>,
    pub loads: Vec<Load>,
    pub links: Vec<Link>,
    pub fields: Vec<f64>,
    pub far_field: bool,
    pub board: String,
    pub devices: Vec<Device>,
    pub bandwidth: Option<f64>,
    pub report: Vec<f64>,
    pub after: Option<StageFile>,
    #[serde(skip)]
    pub maps: Option<MapResult>,
    pub description: String,
    pub layout: String,
    pub start: f64,
    pub stop: f64,
    pub points: usize,
    pub cell: f64,
    pub region: Option<[f64; 4]>,
    pub max_steps: usize,
    pub excite: Vec<usize>,
    pub ports: Vec<Port>,
    pub elements: Vec<Element>,
    #[serde(skip)]
    pub result: Option<SimResult>,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimResult {
    pub name: String,
    pub freqs: Vec<f64>,
    pub ports: Vec<String>,
    pub s: Vec<Vec<Vec<[f64; 2]>>>,
    pub excited: Vec<bool>,
    pub cells: usize,
    pub grid: [usize; 3],
    pub steps: Vec<usize>,
    pub dt: f64,
    pub seconds: f64,
    pub device: String,
    pub spec_hash: u64,
    #[serde(default)]
    pub maps: Vec<LayerMap>,
    #[serde(default)]
    pub readings: Vec<Reading>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curves: Vec<Curve>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curve {
    pub name: String,
    pub unit: String,
    pub values: Vec<Option<f64>>,
}

impl SimResult {
    pub fn db(&self, i: usize, j: usize) -> Vec<f64> {
        self.s[i][j].iter().map(|c| 10.0 * (c[0] * c[0] + c[1] * c[1]).max(1e-30).log10()).collect()
    }

    pub fn load(path: &Path) -> Option<SimResult> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimProgress {
    pub run: usize,
    pub runs: usize,
    pub port: String,
    pub steps: usize,
    pub max_steps: usize,
    pub decay_db: f64,
    pub started: u64,
    pub updated: u64,
    pub pid: u32,
}

impl SimProgress {
    pub fn fraction(&self) -> f32 {
        let per = (self.steps as f32 / self.max_steps.max(1) as f32).min(1.0);
        ((self.run as f32 + per) / self.runs.max(1) as f32).min(1.0)
    }

    pub fn load(spec: &Path) -> Option<SimProgress> {
        let p: SimProgress =
            serde_json::from_str(&std::fs::read_to_string(progress_path(spec)).ok()?).ok()?;
        (now() <= p.updated + 20).then_some(p)
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn progress_path(spec: &Path) -> std::path::PathBuf {
    let name = spec.file_name().and_then(|n| n.to_str()).unwrap_or("sim");
    let stem = name.strip_suffix(".sim.toml").unwrap_or(name);
    spec.with_file_name(format!("{stem}.progress.json"))
}

pub fn result_path(spec: &Path) -> std::path::PathBuf {
    let name = spec.file_name().and_then(|n| n.to_str()).unwrap_or("sim");
    let stem = name.strip_suffix(".sim.toml").unwrap_or(name);
    spec.with_file_name(format!("{stem}.result.json"))
}

pub fn parse_value(s: &str) -> Option<f64> {
    let s = s.trim().replace('µ', "u");
    let lower = s.to_ascii_lowercase();
    let body = lower.trim_end_matches(['f', 'h', 'r', 'Ω', ' ']);
    let body = body.strip_suffix("ohm").unwrap_or(body);
    let mult = |c: char| match c {
        'p' => Some(1e-12),
        'n' => Some(1e-9),
        'u' => Some(1e-6),
        'm' => Some(1e-3),
        'k' => Some(1e3),
        'g' => Some(1e9),
        _ => None,
    };
    if let Some(pos) = body.find(|c: char| c.is_ascii_alphabetic()) {
        let c = body[pos..].chars().next()?;
        let m = if &s[pos..pos + 1] == "M" { 1e6 } else { mult(c)? };
        let rest = &body[pos + 1..];
        let whole = &body[..pos];
        let v: f64 = if rest.is_empty() {
            whole.parse().ok()?
        } else {
            format!("{whole}.{rest}").parse().ok()?
        };
        return Some(v * m);
    }
    body.parse().ok()
}

pub fn cascade_hash(src: u64, board: u64, devices: &[String]) -> u64 {
    let mut h = src ^ board.rotate_left(17);
    for d in devices {
        h = h.rotate_left(7) ^ hash(d);
    }
    h
}

pub fn hash(src: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn freq(s: &str) -> Option<f64> {
    let lower = s.trim().to_ascii_lowercase();
    let body = lower.strip_suffix("hz").unwrap_or(&lower).trim();
    let (num, m) = match body.chars().last()? {
        'k' => (&body[..body.len() - 1], 1e3),
        'm' => (&body[..body.len() - 1], 1e6),
        'g' => (&body[..body.len() - 1], 1e9),
        _ => (body, 1.0),
    };
    num.trim().parse::<f64>().ok().map(|v| v * m)
}

impl SimFile {
    pub fn resolve(&self, layout: &Layout, copper: &[String], d: &mut Diags) -> Sim {
        let kind = self.kind.unwrap_or_default();
        let fdtd = kind == SimKind::Fdtd;
        let band = self.frequency.as_ref().map(|f| (freq(&f.start), freq(&f.stop)));
        let (start, stop) = match band {
            None if !fdtd => (1e8, 1e9),
            Some((Some(a), Some(b))) if a > 0.0 && b > a => (a, b),
            _ => {
                d.error(
                    "frequency",
                    "give start and stop like \"100MHz\" and \"4GHz\", stop above start",
                );
                (1e8, 1e9)
            }
        };
        let find_pad = |spec: &str, d: &mut Diags, at: &str| -> Option<(usize, usize)> {
            let Some((r, n)) = spec.rsplit_once('.') else {
                d.error(at, format!("`{spec}` should be REF.PAD, like J1.1"));
                return None;
            };
            let Some(pi) = layout.parts.iter().position(|p| p.reference == r) else {
                d.error(at, format!("{r} is not placed in layout {}", layout.name));
                return None;
            };
            let Some(k) =
                layout.parts[pi].pads.iter().position(|q| q.number == n && !q.copper.is_empty())
            else {
                d.error(at, format!("{r} has no copper pad {n}"));
                return None;
            };
            Some((pi, k))
        };
        let center = |pi: usize, k: usize| {
            let mut b = crate::graphic::Bounds::EMPTY;
            layout.parts[pi].pads[k].outlines.iter().flatten().for_each(|c| b.add(*c));
            b.center()
        };
        let mut ports = Vec::new();
        for (i, p) in self.ports.iter().enumerate() {
            let at = format!("ports[{i}] {}", p.name);
            let Some((pi, k)) = find_pad(&p.pad, d, &at) else { continue };
            let pad = &layout.parts[pi].pads[k];
            let layer = if pad.copper.contains(&"F.Cu".to_string()) {
                "F.Cu".to_string()
            } else {
                pad.copper[0].clone()
            };
            let li = copper.iter().position(|c| *c == layer).unwrap_or(0);
            let reference = match &p.reference {
                Some(r) => r.clone(),
                None => {
                    let next = if li + 1 < copper.len() { li + 1 } else { li.saturating_sub(1) };
                    copper[next].clone()
                }
            };
            if !copper.contains(&reference) || reference == layer {
                d.error(&at, format!("reference `{reference}` must be another copper layer"));
            }
            ports.push(Port {
                name: p.name.clone(),
                part: pi,
                pad: k,
                at: center(pi, k),
                layer,
                reference,
                impedance: p.impedance.unwrap_or(50.0),
            });
        }
        if ports.is_empty() && fdtd {
            d.error("ports", "an FDTD simulation needs at least one port");
        }
        for p in &ports {
            let covered = layout.zones.iter().any(|z| z.layer == p.reference && z.filled(p.at))
                || layout.tracks.iter().any(|t| {
                    t.layer == p.reference
                        && t.points.windows(2).any(|w| {
                            crate::geom::point_segment_distance(p.at, w[0], w[1]) <= t.width / 2.0
                        })
                });
            if fdtd && !covered {
                d.error(
                    format!("ports {}", p.name),
                    format!(
                        "{} has no copper under the pad, the port would float; pick another `reference`",
                        p.reference
                    ),
                );
            }
        }
        let part_of = |r: &str, d: &mut Diags, at: &str| -> Option<usize> {
            let i = layout.parts.iter().position(|p| p.reference == r);
            if i.is_none() {
                d.error(at, format!("{r} is not placed in layout {}", layout.name));
            }
            i
        };
        let pad_ref = |spec: &str, d: &mut Diags, at: &str| {
            find_pad(spec, d, at).map(|(part, pad)| PadRef { part, pad, label: spec.to_string() })
        };
        let unit_value = |s: &str, units: &[(&str, f64)], at: &str, d: &mut Diags| -> Option<f64> {
            let lower = s.trim().to_ascii_lowercase();
            for (u, m) in units {
                if let Some(num) = lower.strip_suffix(u)
                    && let Ok(v) = num.trim().parse::<f64>()
                {
                    return Some(v * m);
                }
            }
            let v = lower.parse::<f64>().ok();
            if v.is_none() {
                d.error(at, format!("cannot read `{s}`"));
            }
            v
        };
        let sources: Vec<HeatSource> = self
            .sources
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let at = format!("sources[{i}] {}", h.reference);
                let part = part_of(&h.reference, d, &at)?;
                let watts = unit_value(&h.power, &[("mw", 1e-3), ("w", 1.0)], &at, d)?;
                for n in &h.pads {
                    if !layout.parts[part].pads.iter().any(|q| &q.number == n) {
                        d.error(&at, format!("{} has no pad {n}", h.reference));
                    }
                }
                Some(HeatSource {
                    part,
                    reference: h.reference.clone(),
                    watts,
                    theta_jc: h.theta_jc,
                    pads: h.pads.clone(),
                })
            })
            .collect();
        let supplies: Vec<Supply> = self
            .supplies
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let at = format!("supplies[{i}] {}", s.pad);
                let pad = pad_ref(&s.pad, d, &at)?;
                let volts = unit_value(&s.voltage, &[("mv", 1e-3), ("v", 1.0)], &at, d)?;
                Some(Supply { pad, volts })
            })
            .collect();
        let loads: Vec<Load> = self
            .loads
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let at = format!("loads[{i}] {}", l.pad);
                let pad = pad_ref(&l.pad, d, &at)?;
                let amps =
                    unit_value(&l.current, &[("ma", 1e-3), ("ua", 1e-6), ("a", 1.0)], &at, d)?;
                let return_pad = l.return_pad.as_ref().and_then(|r| pad_ref(r, d, &at));
                Some(Load { pad, amps, return_pad })
            })
            .collect();
        let mut links: Vec<Link> = Vec::new();
        if kind == SimKind::Dc {
            for (pi, part) in layout.parts.iter().enumerate() {
                let explicit = self.links.iter().find(|l| l.reference == part.reference);
                let prefix: String =
                    part.reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                let ohms = match (explicit, prefix.as_str()) {
                    (Some(l), _) => parse_value(&l.resistance).or_else(|| {
                        d.error(
                            format!("link {}", part.reference),
                            format!("cannot read `{}`", l.resistance),
                        );
                        None
                    }),
                    (None, "R") => parse_value(&part.value),
                    (None, "L") => Some(0.1),
                    _ => None,
                };
                let Some(ohms) = ohms else { continue };
                let a = part.pads.iter().position(|q| q.number == "1" && !q.copper.is_empty());
                let b = part.pads.iter().position(|q| q.number == "2" && !q.copper.is_empty());
                if let (Some(a), Some(b)) = (a, b) {
                    links.push(Link {
                        reference: part.reference.clone(),
                        a: PadRef { part: pi, pad: a, label: format!("{}.1", part.reference) },
                        b: PadRef { part: pi, pad: b, label: format!("{}.2", part.reference) },
                        ohms: ohms.max(1e-6),
                    });
                }
            }
            for l in &self.links {
                if !layout.parts.iter().any(|p| p.reference == l.reference) {
                    d.error(
                        format!("link {}", l.reference),
                        format!("{} is not placed", l.reference),
                    );
                }
            }
            if supplies.is_empty() {
                d.error(
                    "supplies",
                    "a DC simulation needs at least one [[supplies]] pad with a voltage",
                );
            }
        }
        if kind == SimKind::Thermal && sources.is_empty() {
            d.error(
                "sources",
                "a thermal simulation needs at least one [[sources]] part with a power",
            );
        }
        let port_parts: Vec<usize> = ports.iter().map(|p| p.part).collect();
        let mut elements = Vec::new();
        for (pi, part) in layout.parts.iter().enumerate() {
            let explicit = self.models.iter().find(|m| m.reference == part.reference);
            let value = |s: &str, at: &str, d: &mut Diags| {
                let v = parse_value(s);
                if v.is_none() {
                    d.error(at, format!("cannot read the value `{s}`"));
                }
                v
            };
            let at = format!("model {}", part.reference);
            let model = match explicit {
                Some(m) if m.open => Model::Open,
                Some(m) => match (&m.capacitor, &m.inductor, &m.resistor) {
                    (Some(c), None, None) => {
                        value(c, &at, d).map(Model::Capacitor).unwrap_or(Model::Open)
                    }
                    (None, Some(l), None) => {
                        value(l, &at, d).map(Model::Inductor).unwrap_or(Model::Open)
                    }
                    (None, None, Some(r)) => {
                        value(r, &at, d).map(Model::Resistor).unwrap_or(Model::Open)
                    }
                    _ => {
                        d.error(
                            &at,
                            "give exactly one of capacitor, inductor, resistor, or open = true",
                        );
                        Model::Open
                    }
                },
                None => {
                    let prefix: String =
                        part.reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                    let v = parse_value(&part.value);
                    match (prefix.as_str(), v) {
                        ("C", Some(v)) => Model::Capacitor(v),
                        ("L", Some(v)) => Model::Inductor(v),
                        ("R", Some(v)) => Model::Resistor(v),
                        _ => Model::Open,
                    }
                }
            };
            if model == Model::Open {
                continue;
            }
            if port_parts.contains(&pi) {
                d.warn(&at, "has a port on one of its pads, its model is dropped");
                continue;
            }
            let pads: Vec<usize> =
                (0..part.pads.len()).filter(|k| !part.pads[*k].copper.is_empty()).collect();
            let numbers: Vec<&str> = pads.iter().map(|k| part.pads[*k].number.as_str()).collect();
            let (Some(a), Some(b)) = (
                pads.iter().find(|k| part.pads[**k].number == "1"),
                pads.iter().find(|k| part.pads[**k].number == "2"),
            ) else {
                d.warn(
                    &at,
                    format!(
                        "a lumped model needs pads 1 and 2, this part has {}",
                        numbers.join(", ")
                    ),
                );
                continue;
            };
            let layer = part.pads[*a].copper[0].clone();
            elements.push(Element {
                part: pi,
                reference: part.reference.clone(),
                model,
                a: center(pi, *a),
                b: center(pi, *b),
                layer,
            });
        }
        let excite: Vec<usize> = if self.excite.is_empty() {
            (0..ports.len()).collect()
        } else {
            self.excite
                .iter()
                .filter_map(|n| {
                    let i = ports.iter().position(|p| &p.name == n);
                    if i.is_none() {
                        d.error("excite", format!("no port named `{n}`"));
                    }
                    i
                })
                .collect()
        };
        let fields: Vec<f64> = self
            .fields
            .iter()
            .filter_map(|f| {
                let v = freq(f);
                if v.is_none() {
                    d.error("fields", format!("cannot read the frequency `{f}`"));
                }
                v
            })
            .collect();
        if fields.len() > 4 {
            d.error("fields", "at most four field frequencies per run");
        }
        if self.far_field && fields.is_empty() {
            d.error("far_field", "far_field needs `fields`, the frequencies to compute it at");
        }
        if kind == SimKind::Cascade {
            if self.board.is_none() {
                d.error(
                    "board",
                    "a cascade needs `board`, the FDTD sim of the board around the devices",
                );
            }
            if self.devices.is_empty() {
                d.error("devices", "a cascade needs at least one [[devices]] entry");
            }
        } else if self.board.is_some() || !self.devices.is_empty() {
            d.error("devices", "`board` and `devices` belong to kind = \"cascade\"");
        }
        let cell = self.cell.map(Length::to_mm).unwrap_or(if kind == SimKind::Thermal {
            0.2
        } else {
            0.05
        });
        if cell < 0.005 {
            d.error("cell", "cell is below 5 um, the run would never finish");
        }
        Sim {
            name: self.name.clone(),
            kind,
            ambient: self.ambient.unwrap_or(25.0),
            h_top: self.h_top.unwrap_or(10.0),
            h_bottom: self.h_bottom.unwrap_or(10.0),
            sources,
            supplies,
            loads,
            links,
            fields,
            far_field: self.far_field,
            board: self.board.clone().unwrap_or_default(),
            devices: self
                .devices
                .iter()
                .map(|x| Device {
                    reference: x.reference.clone().unwrap_or_default(),
                    file: x.file.clone(),
                    ports: x.ports.clone(),
                    mount: x.mount,
                    datasheet: x
                        .datasheet
                        .iter()
                        .filter_map(|r| {
                            let f = freq(&r.freq);
                            if f.is_none() {
                                d.error(
                                    "devices",
                                    format!("cannot read the frequency `{}`", r.freq),
                                );
                            }
                            Some(DatasheetRow { freq: f?, nf: r.nf, oip3: r.oip3, p1db: r.p1db })
                        })
                        .collect(),
                })
                .collect(),
            bandwidth: self.bandwidth.as_deref().and_then(|b| {
                let v = freq(b);
                if v.is_none() {
                    d.error("bandwidth", format!("cannot read `{b}`, give it like \"2MHz\""));
                }
                v
            }),
            report: self
                .report
                .iter()
                .filter_map(|f| {
                    let v = freq(f);
                    if v.is_none() {
                        d.error("report", format!("cannot read the frequency `{f}`"));
                    }
                    v
                })
                .collect(),
            after: self.after.clone(),
            maps: None,
            description: self.description.clone(),
            layout: layout.name.clone(),
            start,
            stop,
            points: self.frequency.as_ref().and_then(|f| f.points).unwrap_or(201).clamp(2, 5001),
            cell,
            region: self.region,
            max_steps: self.max_steps.unwrap_or(150_000),
            excite,
            ports,
            elements,
            result: None,
            stale: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_like_a_bom() {
        assert_eq!(parse_value("100p"), Some(100e-12));
        assert!((parse_value("4k7").unwrap() - 4700.0).abs() < 1e-9);
        assert!((parse_value("10uF").unwrap() - 10e-6).abs() < 1e-18);
        assert!((parse_value("47nH").unwrap() - 47e-9).abs() < 1e-18);
        assert_eq!(parse_value("1M"), Some(1e6));
        assert_eq!(parse_value("green"), None);
        assert_eq!(freq("4GHz"), Some(4e9));
        assert_eq!(freq("100 MHz"), Some(1e8));
    }
}

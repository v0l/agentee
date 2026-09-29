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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    pub frequency: FrequencyFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excite: Vec<String>,
    pub ports: Vec<PortFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelFile>,
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
        let (start, stop) = match (freq(&self.frequency.start), freq(&self.frequency.stop)) {
            (Some(a), Some(b)) if a > 0.0 && b > a => (a, b),
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
        if ports.is_empty() {
            d.error("ports", "a simulation needs at least one port");
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
        let cell = self.cell.map(Length::to_mm).unwrap_or(0.05);
        if cell < 0.005 {
            d.error("cell", "cell is below 5 um, the run would never finish");
        }
        Sim {
            name: self.name.clone(),
            description: self.description.clone(),
            layout: layout.name.clone(),
            start,
            stop,
            points: self.frequency.points.unwrap_or(201).clamp(2, 5001),
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

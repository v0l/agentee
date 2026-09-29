use crate::rf::Cx;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub kind: String,
    pub c_comp: f64,
    pub voltage: Option<f64>,
    pub pulldown: Vec<(f64, f64)>,
    pub pullup: Vec<(f64, f64)>,
    pub ramp_rise: Option<(f64, f64)>,
    pub ramp_fall: Option<(f64, f64)>,
    pub r_load: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Package {
    pub r: f64,
    pub l: f64,
    pub c: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Component {
    pub name: String,
    pub package: Package,
    pub pins: HashMap<String, (String, Option<Package>)>,
}

#[derive(Clone, Debug, Default)]
pub struct Ibis {
    pub models: Vec<Model>,
    pub components: Vec<Component>,
}

pub fn value(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("na") {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let (mut v, suffix) = (1..=lower.len())
        .rev()
        .filter(|k| lower.is_char_boundary(*k))
        .find_map(|k| lower[..k].parse::<f64>().ok().map(|v| (v, &lower[k..])))?;
    let mult = match suffix.chars().next() {
        Some('t') => 1e12,
        Some('g') => 1e9,
        Some('k') => 1e3,
        Some('m') if suffix.starts_with("meg") => 1e6,
        Some('m') => 1e-3,
        Some('u') => 1e-6,
        Some('n') => 1e-9,
        Some('p') => 1e-12,
        Some('f') => 1e-15,
        _ => 1.0,
    };
    v *= mult;
    Some(v)
}

fn typ(rest: &str) -> Option<f64> {
    rest.split_whitespace().next().and_then(value)
}

pub fn parse(text: &str) -> Result<Ibis, String> {
    let mut out = Ibis::default();
    let mut comment = '|';
    let mut section = String::new();
    let mut model: Option<Model> = None;
    let mut component: Option<Component> = None;
    let finish_model = |m: Option<Model>, out: &mut Ibis| {
        if let Some(m) = m {
            out.models.push(m);
        }
    };
    for raw in text.lines() {
        let line = match raw.find(comment) {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else { continue };
            let key = rest[..end].trim().to_ascii_lowercase().replace('_', " ");
            let arg = rest[end + 1..].trim();
            section = key.clone();
            match key.as_str() {
                "comment char" => {
                    if let Some(c) = arg.chars().next() {
                        comment = c;
                    }
                }
                "model" => {
                    finish_model(model.take(), &mut out);
                    if let Some(c) = component.take() {
                        out.components.push(c);
                    }
                    model =
                        Some(Model { name: arg.to_string(), r_load: 50.0, ..Default::default() });
                }
                "component" => {
                    finish_model(model.take(), &mut out);
                    if let Some(c) = component.take() {
                        out.components.push(c);
                    }
                    component = Some(Component { name: arg.to_string(), ..Default::default() });
                }
                "voltage range" => {
                    if let Some(m) = model.as_mut() {
                        m.voltage = typ(arg);
                    }
                }
                "end" => {
                    finish_model(model.take(), &mut out);
                }
                _ => {}
            }
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if let Some(m) = model.as_mut() {
            let first = words[0].to_ascii_lowercase();
            match section.as_str() {
                "pulldown" | "pullup" => {
                    if let (Some(v), Some(i)) =
                        (value(words[0]), words.get(1).and_then(|w| value(w)))
                    {
                        if section == "pulldown" {
                            m.pulldown.push((v, i));
                        } else {
                            m.pullup.push((v, i));
                        }
                    }
                }
                "ramp" => {
                    let ratio = |w: &str| -> Option<(f64, f64)> {
                        let (a, b) = w.split_once('/')?;
                        Some((value(a)?, value(b)?))
                    };
                    if first == "dv/dt_r" {
                        m.ramp_rise = words.get(1).and_then(|w| ratio(w));
                    } else if first == "dv/dt_f" {
                        m.ramp_fall = words.get(1).and_then(|w| ratio(w));
                    } else if first.starts_with("r_load") {
                        let v = line.split('=').nth(1).and_then(typ);
                        if let Some(v) = v {
                            m.r_load = v;
                        }
                    }
                }
                _ => {
                    if first.starts_with("model_type") {
                        m.kind = words.get(1).map(|w| w.to_ascii_lowercase()).unwrap_or_default();
                    } else if first == "c_comp" {
                        m.c_comp = words.get(1).and_then(|w| value(w)).unwrap_or(0.0);
                    }
                }
            }
            continue;
        }
        if let Some(c) = component.as_mut() {
            match section.as_str() {
                "package" => {
                    let v = words.get(1).and_then(|w| value(w)).unwrap_or(0.0);
                    match words[0].to_ascii_lowercase().as_str() {
                        "r_pkg" => c.package.r = v,
                        "l_pkg" => c.package.l = v,
                        "c_pkg" => c.package.c = v,
                        _ => {}
                    }
                }
                "pin" if words.len() >= 3 && !words[0].eq_ignore_ascii_case("signal_name") => {
                    let pin_pkg = (words.len() >= 6).then(|| Package {
                        r: value(words[3]).unwrap_or(0.0),
                        l: value(words[4]).unwrap_or(0.0),
                        c: value(words[5]).unwrap_or(0.0),
                    });
                    c.pins.insert(words[0].to_string(), (words[2].to_string(), pin_pkg));
                }
                _ => {}
            }
        }
    }
    finish_model(model.take(), &mut out);
    if let Some(c) = component.take() {
        out.components.push(c);
    }
    if out.models.is_empty() {
        return Err("no [Model] in the file".into());
    }
    Ok(out)
}

fn slope_at_zero(table: &[(f64, f64)], span: f64) -> Option<f64> {
    let pts: Vec<&(f64, f64)> = table.iter().filter(|(v, _)| *v >= 0.0 && *v <= span).collect();
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f64;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (num, den) = pts
        .iter()
        .fold((0.0, 0.0), |a, p| (a.0 + (p.0 - mx) * (p.1 - my), a.1 + (p.0 - mx).powi(2)));
    (den > 0.0).then(|| num / den)
}

impl Model {
    pub fn output_resistance(&self) -> Option<f64> {
        let span = 0.1 * self.voltage.unwrap_or(3.3);
        let r: Vec<f64> = [&self.pulldown, &self.pullup]
            .into_iter()
            .filter_map(|t| slope_at_zero(t, span))
            .filter(|g| g.abs() > 1e-9)
            .map(|g| 1.0 / g.abs())
            .collect();
        (!r.is_empty()).then(|| r.iter().sum::<f64>() / r.len() as f64)
    }

    pub fn rise_10_90(&self) -> Option<f64> {
        let (_, dt) = self.ramp_rise.or(self.ramp_fall)?;
        Some(dt * 2.563 / 1.683)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Driver {
    pub r_out: f64,
    pub c_comp: f64,
    pub pkg: Package,
}

#[derive(Clone, Copy, Debug)]
pub struct Receiver {
    pub c_comp: f64,
    pub pkg: Package,
}

impl Driver {
    pub fn thevenin(&self, f: f64) -> (Cx, Cx) {
        let w = 2.0 * std::f64::consts::PI * f;
        let zc = |c: f64| if c > 0.0 { Cx::ONE / Cx::new(0.0, w * c) } else { Cx::new(1e30, 0.0) };
        let r = Cx::new(self.r_out, 0.0);
        let die_c = zc(self.c_comp);
        let v_die = die_c / (r + die_c);
        let z_die = r * die_c / (r + die_c);
        let series = Cx::new(self.pkg.r, w * self.pkg.l);
        let z1 = z_die + series;
        let pin_c = zc(self.pkg.c);
        (v_die * pin_c / (z1 + pin_c), z1 * pin_c / (z1 + pin_c))
    }
}

impl Receiver {
    pub fn load(&self, f: f64) -> (Cx, Cx) {
        let w = 2.0 * std::f64::consts::PI * f;
        let zc = |c: f64| if c > 0.0 { Cx::ONE / Cx::new(0.0, w * c) } else { Cx::new(1e30, 0.0) };
        let series = Cx::new(self.pkg.r, w * self.pkg.l);
        let die = zc(self.c_comp);
        let branch = series + die;
        let pin_c = zc(self.pkg.c);
        (branch * pin_c / (branch + pin_c), die / branch)
    }
}

pub fn terminated(s: [[Cx; 2]; 2], z0: f64, zs: Cx, zl: Cx) -> Cx {
    let r = Cx::new(z0, 0.0);
    let gs = (zs - r) / (zs + r);
    let gl = (zl - r) / (zl + r);
    let den =
        ((Cx::ONE - s[0][0] * gs) * (Cx::ONE - s[1][1] * gl) - s[0][1] * s[1][0] * gs * gl) * 2.0;
    s[1][0] * (Cx::ONE + gl) * (Cx::ONE - gs) / den
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_with_ibis_suffixes() {
        assert!((value("3.63pF").unwrap() - 3.63e-12).abs() < 1e-24);
        assert!((value("0.422435nH").unwrap() - 0.422435e-9).abs() < 1e-20);
        assert!((value("250.0m").unwrap() - 0.25).abs() < 1e-12);
        assert!((value("-9.8511e-01").unwrap() + 0.98511).abs() < 1e-12);
        assert!(value("NA").is_none());
    }

    #[test]
    fn a_matched_thru_splits_the_source_in_half() {
        let s = [[Cx::ZERO, Cx::ONE], [Cx::ONE, Cx::ZERO]];
        let h = terminated(s, 50.0, Cx::new(50.0, 0.0), Cx::new(50.0, 0.0));
        assert!((h - Cx::new(0.5, 0.0)).abs() < 1e-12);
        let h = terminated(s, 50.0, Cx::new(20.0, 0.0), Cx::new(1e9, 0.0));
        assert!((h - Cx::ONE).abs() < 1e-6);
        let h = terminated(s, 50.0, Cx::new(20.0, 0.0), Cx::new(50.0, 0.0));
        assert!((h.re - 50.0 / 70.0).abs() < 1e-12);
    }

    #[test]
    fn a_ti_lvc_driver_reads_its_tables() {
        let text = include_str!("../tests/data/sn74lvc1g04.ibs");
        let ibis = parse(text).unwrap();
        let out = ibis.models.iter().find(|m| m.name == "LVC1G04_OUT_33").unwrap();
        assert_eq!(out.kind, "output");
        assert!((out.voltage.unwrap() - 3.3).abs() < 1e-9);
        let r = out.output_resistance().unwrap();
        eprintln!(
            "LVC1G04 at 3.3 V: {r:.1} ohm out, {:.0} ps 10-90",
            out.rise_10_90().unwrap() * 1e12
        );
        assert!(r > 5.0 && r < 60.0, "{r}");
        let (dv, _) = out.ramp_rise.unwrap();
        let into_load = dv / 0.6;
        let divider = 3.3 * out.r_load / (out.r_load + r);
        eprintln!("ramp implies {into_load:.2} V into 50 ohm, the V-I tables give {divider:.2} V");
        assert!((into_load - divider).abs() / divider < 0.05);
        let comp = ibis.components.iter().find(|c| c.name == "LVC1G04_DBV").unwrap();
        assert!(comp.package.l > 0.1e-9 && comp.pins.contains_key("4"));
    }
}

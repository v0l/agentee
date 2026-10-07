use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub const SELV_DC: f64 = 60.0;
pub const SELV_AC: f64 = 30.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Voltage {
    pub dc: f64,
    pub ac: f64,
}

impl Voltage {
    pub const ZERO: Voltage = Voltage { dc: 0.0, ac: 0.0 };

    pub fn dc(v: f64) -> Voltage {
        Voltage { dc: v, ac: 0.0 }
    }

    pub fn ac(rms: f64) -> Voltage {
        Voltage { dc: 0.0, ac: rms }
    }

    pub fn parse(s: &str) -> Result<Voltage, String> {
        let s = s.trim();
        let end = s
            .char_indices()
            .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0)))
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        let v: f64 = s[..end].parse().map_err(|_| {
            format!("`{s}` does not start with a number, write it like 48VDC or 230VAC")
        })?;
        let unit = s[end..].trim().to_ascii_lowercase();
        let (scale, kind) = match unit.strip_prefix('k') {
            Some(rest) => (1000.0, rest.to_string()),
            None => match unit.strip_prefix('m') {
                Some(rest) => (0.001, rest.to_string()),
                None => (1.0, unit.clone()),
            },
        };
        let v = v * scale;
        match kind.as_str() {
            "" | "v" | "vdc" => Ok(Voltage::dc(v)),
            "vac" | "vrms" if v >= 0.0 => Ok(Voltage::ac(v)),
            "vac" | "vrms" => {
                Err(format!("`{s}`: an AC voltage is an rms magnitude, not negative"))
            }
            _ => Err(format!("unknown voltage unit `{}` in `{s}` (use VDC or VAC)", &s[end..])),
        }
    }

    pub fn peak(&self) -> f64 {
        self.dc.abs() + self.ac * std::f64::consts::SQRT_2
    }

    pub fn rms(&self) -> f64 {
        (self.dc * self.dc + self.ac * self.ac).sqrt()
    }

    pub fn hazardous(&self) -> bool {
        self.dc.abs() > SELV_DC || self.ac > SELV_AC
    }

    pub fn between(a: Voltage, b: Voltage) -> Voltage {
        Voltage { dc: a.dc - b.dc, ac: a.ac + b.ac }
    }
}

impl fmt::Display for Voltage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = |v: f64| crate::units::trim(v, 3);
        match (self.dc != 0.0, self.ac != 0.0) {
            (_, false) => write!(f, "{}VDC", n(self.dc)),
            (false, true) => write!(f, "{}VAC", n(self.ac)),
            (true, true) => write!(f, "{}VAC on {}VDC", n(self.ac), n(self.dc)),
        }
    }
}

impl Serialize for Voltage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Voltage {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Voltage;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a voltage like \"48VDC\", \"-12VDC\" or \"230VAC\"")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Voltage, E> {
                Ok(Voltage::dc(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Voltage, E> {
                Ok(Voltage::dc(v as f64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Voltage, E> {
                Ok(Voltage::dc(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Voltage, E> {
                Voltage::parse(v).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Functional,
    Reinforced,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Spacing {
    pub grade: Grade,
    pub working: Voltage,
    pub clearance: f64,
    pub inner: f64,
    pub creepage: f64,
}

const IPC: [(f64, f64, f64); 9] = [
    (15.0, 0.05, 0.1),
    (30.0, 0.05, 0.1),
    (50.0, 0.1, 0.6),
    (100.0, 0.1, 0.6),
    (150.0, 0.2, 0.6),
    (170.0, 0.2, 1.25),
    (250.0, 0.2, 1.25),
    (300.0, 0.2, 1.25),
    (500.0, 0.25, 2.5),
];

pub fn ipc_clearance(peak: f64) -> (f64, f64) {
    let peak = peak.abs();
    if let Some(&(_, inner, outer)) = IPC.iter().find(|(v, _, _)| peak <= *v + 1e-9) {
        return (inner, outer);
    }
    let over = peak - 500.0;
    (0.25 + over * 0.0025, 2.5 + over * 0.005)
}

const VOLTS: [f64; 21] = [
    10.0, 12.5, 16.0, 20.0, 25.0, 32.0, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0,
    320.0, 400.0, 500.0, 630.0, 800.0, 1000.0,
];
const PRINTED_PD1: [f64; 21] = [
    0.025, 0.025, 0.025, 0.025, 0.025, 0.025, 0.025, 0.025, 0.04, 0.063, 0.1, 0.16, 0.25, 0.4,
    0.56, 0.75, 1.0, 1.3, 1.8, 2.4, 3.2,
];
const PRINTED_PD2: [f64; 21] = [
    0.04, 0.04, 0.04, 0.04, 0.04, 0.04, 0.04, 0.04, 0.063, 0.1, 0.16, 0.25, 0.4, 0.63, 1.0, 1.6,
    2.0, 2.5, 3.2, 4.0, 5.0,
];
const GROUP_III_PD1: [f64; 21] = [
    0.08, 0.09, 0.1, 0.11, 0.125, 0.14, 0.16, 0.18, 0.2, 0.22, 0.25, 0.28, 0.32, 0.42, 0.56, 0.75,
    1.0, 1.3, 1.8, 2.4, 3.2,
];
const GROUP_III_PD2: [f64; 21] = [
    0.4, 0.42, 0.45, 0.48, 0.5, 0.53, 1.1, 1.2, 1.25, 1.3, 1.4, 1.5, 1.6, 2.0, 2.5, 3.2, 4.0, 5.0,
    6.3, 8.0, 10.0,
];
const GROUP_III_PD3: [f64; 21] = [
    1.0, 1.05, 1.1, 1.2, 1.25, 1.3, 1.8, 1.9, 2.0, 2.1, 2.2, 2.4, 2.5, 3.2, 4.0, 5.0, 6.3, 8.0,
    10.0, 12.5, 16.0,
];

fn interpolate(table: &[f64; 21], rms: f64) -> f64 {
    let rms = rms.abs();
    if rms <= VOLTS[0] {
        return table[0];
    }
    for i in 1..VOLTS.len() {
        if rms <= VOLTS[i] {
            let t = (rms - VOLTS[i - 1]) / (VOLTS[i] - VOLTS[i - 1]);
            return table[i - 1] + t * (table[i] - table[i - 1]);
        }
    }
    table[20] * rms / VOLTS[20]
}

pub fn creepage(rms: f64, pollution_degree: u8, grade: Grade) -> f64 {
    match grade {
        Grade::Functional => match pollution_degree {
            1 => interpolate(&PRINTED_PD1, rms),
            2 => interpolate(&PRINTED_PD2, rms),
            _ => interpolate(&GROUP_III_PD3, rms),
        },
        Grade::Reinforced => {
            2.0 * match pollution_degree {
                1 => interpolate(&GROUP_III_PD1, rms),
                2 => interpolate(&GROUP_III_PD2, rms),
                _ => interpolate(&GROUP_III_PD3, rms),
            }
        }
    }
}

const IMPULSE: [(f64, f64); 13] = [
    (330.0, 0.01),
    (500.0, 0.04),
    (800.0, 0.1),
    (1500.0, 0.5),
    (2500.0, 1.5),
    (4000.0, 3.0),
    (6000.0, 5.5),
    (8000.0, 8.0),
    (12000.0, 14.0),
    (15000.0, 18.0),
    (20000.0, 25.0),
    (25000.0, 33.0),
    (30000.0, 40.0),
];

const MAINS: [(f64, f64); 6] = [
    (50.0, 500.0),
    (100.0, 800.0),
    (150.0, 1500.0),
    (300.0, 2500.0),
    (600.0, 4000.0),
    (1000.0, 6000.0),
];

fn impulse_step(volts: f64) -> usize {
    IMPULSE.iter().position(|(v, _)| volts <= *v + 1e-9).unwrap_or(IMPULSE.len() - 1)
}

pub fn withstand(working: Voltage) -> f64 {
    if working.ac > 0.0 {
        let line = working.rms();
        MAINS.iter().find(|(v, _)| line <= *v + 1e-9).map_or(8000.0, |(_, u)| *u)
    } else {
        working.peak()
    }
}

pub fn reinforced_clearance(working: Voltage, pollution_degree: u8) -> f64 {
    let step = (impulse_step(withstand(working)) + 1).min(IMPULSE.len() - 1);
    let floor = match pollution_degree {
        1 => 0.0,
        2 => 0.2,
        _ => 0.8,
    };
    IMPULSE[step].1.max(floor)
}

pub const MAINS_TOLERANCE: f64 = 1.1;

pub fn own_clearance(v: Voltage) -> f64 {
    ipc_clearance(Voltage { ac: v.ac * MAINS_TOLERANCE, ..v }.peak()).1
}

pub fn spacing(a: Voltage, b: Voltage, pollution_degree: u8) -> Spacing {
    let working = Voltage::between(a, b);
    let worst = Voltage { ac: working.ac * MAINS_TOLERANCE, ..working };
    let grade = if a.hazardous() != b.hazardous() { Grade::Reinforced } else { Grade::Functional };
    let (inner, outer) = ipc_clearance(worst.peak());
    let (inner, clearance) = match grade {
        Grade::Functional => (inner, outer),
        Grade::Reinforced => {
            let safe = reinforced_clearance(worst, pollution_degree);
            (inner.max(safe), outer.max(safe))
        }
    };
    let creepage = creepage(worst.rms(), pollution_degree, grade).max(clearance);
    Spacing { grade, working, clearance, inner, creepage }
}

const RATING_FIELDS: [&str; 2] = ["rated_voltage", "voltage_rating"];

pub fn rating(
    reference: &str,
    value: &str,
    fields: &std::collections::BTreeMap<String, String>,
) -> Option<f64> {
    let capacitor =
        reference.strip_prefix('C').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    let from_value = value
        .split(['/', ' '])
        .skip(1)
        .find(|p| p.trim_end().ends_with(['V', 'v']))
        .filter(|_| capacitor);
    let field = RATING_FIELDS
        .iter()
        .find_map(|k| fields.get(*k))
        .or_else(|| fields.get("voltage").filter(|_| capacitor));
    let text = field.map(String::as_str).or(from_value)?;
    Voltage::parse(text).ok().map(|v| v.peak())
}

pub fn check_ratings(
    s: &crate::schematic::Schematic,
    board: &crate::board::Board,
    d: &mut crate::diag::Diags,
) {
    if board.netclasses.iter().all(|c| c.voltage.is_none()) {
        return;
    }
    for (pi, part) in s.parts.iter().enumerate() {
        let Some(rated) = rating(&part.reference, &part.value, &part.fields) else { continue };
        let nets: Vec<&crate::schematic::Net> =
            s.nets.iter().filter(|n| n.pins.iter().any(|r| r.part == pi)).collect();
        let volts = |n: &crate::schematic::Net| board.voltage_of(&n.class);
        if nets.len() < 2 || nets.iter().all(|n| volts(n).is_none()) {
            continue;
        }
        let mut worst: Option<(f64, &str, &str)> = None;
        for (i, a) in nets.iter().enumerate() {
            for b in &nets[i + 1..] {
                let across = Voltage::between(
                    volts(a).unwrap_or(Voltage::ZERO),
                    volts(b).unwrap_or(Voltage::ZERO),
                );
                let v = Voltage { ac: across.ac * MAINS_TOLERANCE, ..across }.peak();
                if worst.is_none_or(|w| v > w.0) {
                    worst = Some((v, &a.name, &b.name));
                }
            }
        }
        let Some((v, a, b)) = worst else { continue };
        let n = |x: f64| crate::units::trim(x, 1);
        let at = format!("part {}", part.reference);
        if rated < v {
            d.error(
                &at,
                format!(
                    "{} is rated {}V but sees {}V peak between {a} and {b} from their netclass voltages",
                    part.reference,
                    n(rated),
                    n(v)
                ),
            );
        } else if rated < v * RATING_HEADROOM {
            d.warn(
                &at,
                format!(
                    "{} is rated {}V and sees {}V peak between {a} and {b}: less than {}% headroom",
                    part.reference,
                    n(rated),
                    n(v),
                    n((RATING_HEADROOM - 1.0) * 100.0)
                ),
            );
        }
    }
}

pub const RATING_HEADROOM: f64 = 1.25;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rating_comes_from_the_value_or_a_field() {
        let mut fields = std::collections::BTreeMap::new();
        assert_eq!(rating("C1", "100n/50V", &fields), Some(50.0));
        assert_eq!(rating("C12", "10u 25V X7R", &fields), Some(25.0));
        assert_eq!(rating("C1", "100n", &fields), None);
        assert_eq!(
            rating("U3", "AMS1117 3.3V", &fields),
            None,
            "a regulator's output is no rating"
        );
        fields.insert("voltage".to_string(), "3.3V".to_string());
        assert_eq!(rating("U3", "x", &fields), None, "a regulator's voltage field is its output");
        assert_eq!(rating("C3", "x", &fields), Some(3.3));
        fields.insert("rated_voltage".to_string(), "1kV".to_string());
        assert_eq!(rating("U3", "x", &fields), Some(1000.0));
    }

    #[test]
    fn voltages_read_dc_and_ac() {
        assert_eq!(Voltage::parse("38VDC").unwrap(), Voltage::dc(38.0));
        assert_eq!(Voltage::parse("-12VDC").unwrap(), Voltage::dc(-12.0));
        assert_eq!(Voltage::parse("3.3V").unwrap(), Voltage::dc(3.3));
        assert_eq!(Voltage::parse("230VAC").unwrap(), Voltage::ac(230.0));
        assert_eq!(Voltage::parse("1.5kVDC").unwrap(), Voltage::dc(1500.0));
        assert_eq!(Voltage::parse("800mV").unwrap(), Voltage::dc(0.8));
        assert!(Voltage::parse("12 amps").is_err());
        assert!(Voltage::parse("-5VAC").is_err());
    }

    #[test]
    fn selv_stops_at_60vdc_and_30vac() {
        assert!(!Voltage::dc(60.0).hazardous());
        assert!(Voltage::dc(-61.0).hazardous());
        assert!(!Voltage::ac(30.0).hazardous());
        assert!(Voltage::ac(100.0).hazardous());
    }

    #[test]
    fn ipc_2221b_steps_by_peak_voltage() {
        assert_eq!(ipc_clearance(3.3), (0.05, 0.1));
        assert_eq!(ipc_clearance(48.0), (0.1, 0.6));
        assert_eq!(ipc_clearance(325.0), (0.25, 2.5));
        let (_, outer) = ipc_clearance(1000.0);
        assert!((outer - 5.0).abs() < 1e-9);
    }

    #[test]
    fn mains_to_selv_is_reinforced_at_the_textbook_figures() {
        let s = spacing(Voltage::ac(230.0), Voltage::ZERO, 2);
        assert_eq!(s.grade, Grade::Reinforced);
        assert!(s.creepage >= 5.0 && s.creepage < 5.2, "{s:?}");
        assert!(s.clearance >= 3.0, "{s:?}");
    }

    #[test]
    fn two_low_voltage_rails_are_functional_and_small() {
        let s = spacing(Voltage::dc(12.0), Voltage::dc(-12.0), 2);
        assert_eq!(s.grade, Grade::Functional);
        assert_eq!(s.clearance, 0.1);
        assert!(s.creepage <= 0.1);
    }

    #[test]
    fn opposite_dc_rails_add_and_ac_adds_to_dc() {
        assert_eq!(Voltage::between(Voltage::dc(48.0), Voltage::dc(-48.0)).peak(), 96.0);
        let w = Voltage::between(Voltage::ac(230.0), Voltage::dc(400.0));
        assert!((w.peak() - (400.0 + 230.0 * std::f64::consts::SQRT_2)).abs() < 1e-9);
    }

    #[test]
    fn two_hazardous_classes_are_functional() {
        let s = spacing(Voltage::ac(230.0), Voltage::dc(400.0), 2);
        assert_eq!(s.grade, Grade::Functional);
        assert!(s.clearance > 2.5);
    }
}

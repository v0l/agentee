use crate::offer::Offer;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Spec {
    Resistor {
        ohms: f64,
        tolerance: f64,
        package: String,
    },
    Capacitor {
        farads: f64,
        volts: Option<f64>,
        dielectric: Option<String>,
        tolerance: Option<f64>,
        package: String,
    },
    Generic {
        name: String,
        package: String,
    },
    Led {
        color: String,
        package: String,
    },
    Specific,
}

const CHIP_SIZES: [&str; 9] =
    ["0201", "0402", "0603", "0805", "1206", "1210", "1812", "2010", "2512"];
const GENERIC: [&str; 24] = [
    "2N7002", "BSS138", "BSS84", "BSS123", "1N4148", "BAT54", "BAT43", "BZT52", "MMSZ", "BZX84",
    "S1", "SS1", "SS2", "SS3", "SMAJ", "SMBJ", "BC847", "BC857", "MMBT3904", "MMBT3906", "LL4148",
    "US1", "ES1", "1N400",
];
const COLORS: [&str; 8] =
    ["red", "green", "blue", "yellow", "orange", "white", "amber", "yellow-green"];

pub fn si(s: &str) -> Option<f64> {
    let chars: Vec<char> = s.trim().chars().collect();
    let start = chars.iter().position(|c| c.is_ascii_digit() || *c == '.')?;
    let mut i = start;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
        i += 1;
    }
    let mut num: String = chars[start..i].iter().collect::<String>().replace(',', ".");
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let mult = match chars.get(i) {
        Some('p') => 1e-12,
        Some('n') => 1e-9,
        Some('u') | Some('µ') | Some('μ') => 1e-6,
        Some('m') => 1e-3,
        Some('k') | Some('K') => 1e3,
        Some('M') => 1e6,
        Some('G') => 1e9,
        Some('R') | Some('r') => 1.0,
        _ => 1.0,
    };
    if mult != 1.0 || matches!(chars.get(i), Some('R') | Some('r')) {
        let frac: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if !frac.is_empty() && !num.contains('.') {
            num = format!("{num}.{frac}");
        }
    }
    num.parse::<f64>().ok().map(|n| n * mult)
}

pub fn percent(s: &str) -> Option<f64> {
    let i = s.find('%')?;
    let head = &s[..i];
    let start = head
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == ' '))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    head[start..].trim().parse().ok()
}

pub fn chip_size(footprint: &str) -> Option<&'static str> {
    footprint.split('_').find_map(|t| CHIP_SIZES.iter().copied().find(|s| *s == t))
}

pub fn discrete_package(footprint: &str) -> String {
    let f = footprint.strip_prefix("D_").unwrap_or(footprint);
    f.split('_').next().unwrap_or(f).to_uppercase()
}

fn aliases(package: &str) -> Vec<&'static str> {
    match package {
        "SOT-23" => vec!["SOT-23", "SOT23", "TO-236", "TO-236AB"],
        "SOT-23-5" => vec!["SOT-23-5", "SOT-25", "SOT-753", "SC-74A"],
        "SOT-23-6" => vec!["SOT-23-6", "SOT-26", "SC-74"],
        "SOT-323" => vec!["SOT-323", "SC-70"],
        "SMA" => vec!["SMA", "DO-214AC"],
        "SMB" => vec!["SMB", "DO-214AA"],
        "SMC" => vec!["SMC", "DO-214AB"],
        "SOD-123" => vec!["SOD-123"],
        "SOD-323" => vec!["SOD-323", "SC-76"],
        "MINIMELF" => vec!["MINIMELF", "SOD-80", "LL-34"],
        _ => Vec::new(),
    }
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || ",()/[];".contains(c))
        .filter(|t| !t.is_empty())
        .flat_map(|t| {
            let t = t.to_uppercase();
            let stripped =
                t.strip_suffix("-2").or_else(|| t.strip_suffix("-3")).map(str::to_string);
            std::iter::once(t).chain(stripped)
        })
        .collect()
}

fn alnum(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

pub fn classify(reference: &str, value: &str, footprint: &str, current: Option<&Offer>) -> Spec {
    let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let chip = chip_size(footprint);
    match (prefix.as_str(), chip) {
        ("R", Some(size)) if footprint.starts_with("R_") => {
            if let Some(ohms) = si(value) {
                let tolerance = current
                    .and_then(|o| o.attribute(&["tolerance"]))
                    .and_then(percent)
                    .unwrap_or(1.0);
                return Spec::Resistor { ohms, tolerance, package: size.into() };
            }
        }
        ("C", Some(size)) if footprint.starts_with("C_") => {
            let mut parts = value.split('/');
            if let Some(farads) = parts.next().and_then(si) {
                let volts = parts
                    .next()
                    .and_then(si)
                    .or_else(|| current.and_then(|o| o.attribute(&["voltage"])).and_then(si));
                let dielectric =
                    current.and_then(|o| o.attribute(&["dielectric"])).and_then(dielectric);
                let tolerance = current.and_then(|o| o.attribute(&["tolerance"])).and_then(percent);
                return Spec::Capacitor {
                    farads,
                    volts,
                    dielectric,
                    tolerance,
                    package: size.into(),
                };
            }
        }
        ("D", Some(size)) if footprint.starts_with("LED_") => {
            let v = value.to_lowercase();
            if let Some(color) = COLORS.iter().rev().find(|c| v.contains(*c)) {
                return Spec::Led { color: color.to_string(), package: size.into() };
            }
        }
        _ => {}
    }
    if matches!(prefix.as_str(), "D" | "Q") {
        let name = alnum(value);
        let package = discrete_package(footprint);
        if GENERIC.iter().any(|g| name.starts_with(g)) && !aliases(&package).is_empty() {
            return Spec::Generic { name, package };
        }
    }
    Spec::Specific
}

pub fn dielectric(s: &str) -> Option<String> {
    let u = s.to_uppercase();
    ["C0G", "NP0", "X8R", "X7T", "X7S", "X7R", "X6S", "X5R", "Y5V", "Z5U"]
        .iter()
        .find(|d| u.contains(*d))
        .map(|d| if *d == "NP0" { "C0G".to_string() } else { d.to_string() })
}

fn dielectric_rank(d: &str) -> i32 {
    match d {
        "C0G" => 3,
        "X8R" | "X7R" | "X7S" | "X7T" => 2,
        "X5R" | "X6S" => 1,
        _ => 0,
    }
}

fn case_matches(o: &Offer, size: &str) -> bool {
    if let Some(v) = o.attribute(&["case code", "in"]) {
        return tokens(v).iter().any(|t| t == size);
    }
    o.attributes.iter().any(|(k, v)| {
        let k = k.to_lowercase();
        (k.contains("case") || k.contains("package") || k.contains("size"))
            && !k.contains("mm")
            && !k.contains("metric")
            && tokens(v.split('[').next().unwrap_or("")).iter().any(|t| t == size)
    })
}

fn package_matches(o: &Offer, package: &str) -> bool {
    let wanted = aliases(package);
    let hit = |v: &str| tokens(v).iter().any(|t| wanted.contains(&t.as_str()));
    o.attributes.iter().any(|(k, v)| {
        let k = k.to_lowercase();
        (k.contains("package") || k.contains("case")) && hit(v)
    })
}

fn kind_text(o: &Offer) -> String {
    let mut t = o.description.to_lowercase();
    for k in ["Product Category", "Category", "Product", "Product Type"] {
        if let Some(v) = o.attributes.get(k) {
            t.push(' ');
            t.push_str(&v.to_lowercase());
        }
    }
    t
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(f64::MIN_POSITIVE)
}

pub fn matches(spec: &Spec, o: &Offer) -> bool {
    match spec {
        Spec::Resistor { ohms, tolerance, package } => {
            let kind = kind_text(o);
            kind.contains("resistor")
                && ![
                    "thermistor",
                    "array",
                    "network",
                    "varistor",
                    "fuse",
                    "jumper",
                    "current sense",
                ]
                .iter()
                .any(|x| kind.contains(x))
                && o.attribute(&["resistance"]).and_then(si).is_some_and(|r| close(r, *ohms, 0.005))
                && o.attribute(&["tolerance"])
                    .and_then(percent)
                    .is_some_and(|t| t <= *tolerance + 1e-9)
                && case_matches(o, package)
        }
        Spec::Capacitor { farads, volts, dielectric: want, tolerance, package } => {
            let kind = kind_text(o);
            let got = o.attribute(&["dielectric"]).and_then(dielectric);
            let dielectric_ok = match (want.as_deref(), got.as_deref()) {
                (Some("C0G"), Some(g)) => g == "C0G",
                (Some(w), Some(g)) => dielectric_rank(g) >= dielectric_rank(w),
                (None, Some(g)) => dielectric_rank(g) >= 1,
                (_, None) => false,
            };
            kind.contains("capacitor")
                && (kind.contains("ceramic") || kind.contains("mlcc"))
                && !["electrolytic", "tantalum", "polymer", "film", "array", "feed"]
                    .iter()
                    .any(|x| kind.contains(x))
                && o.attribute(&["capacitance"])
                    .and_then(si)
                    .is_some_and(|c| close(c, *farads, 0.01))
                && volts.is_none_or(|v| {
                    o.attribute(&["voltage"]).and_then(si).is_some_and(|g| g >= v - 1e-9)
                })
                && dielectric_ok
                && o.attribute(&["tolerance"])
                    .and_then(percent)
                    .is_some_and(|t| t <= tolerance.unwrap_or(20.0) + 1e-9)
                && case_matches(o, package)
        }
        Spec::Generic { name, package } => {
            alnum(&o.mpn).contains(name.as_str()) && package_matches(o, package)
        }
        Spec::Led { color, package } => {
            let kind = kind_text(o);
            kind.contains("led")
                && o.attributes.iter().any(|(k, v)| {
                    let k = k.to_lowercase();
                    (k.contains("colour") || k.contains("color"))
                        && v.to_lowercase().contains(color.as_str())
                })
                && case_matches(o, package)
        }
        Spec::Specific => false,
    }
}

fn eng(v: f64, unit: &str) -> String {
    let (scale, prefix) = [
        (1e9, "G"),
        (1e6, "M"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "u"),
        (1e-9, "n"),
        (1e-12, "p"),
    ]
    .into_iter()
    .find(|(s, _)| v >= *s * 0.9999)
    .unwrap_or((1e-12, "p"));
    let n = v / scale;
    let txt =
        if (n - n.round()).abs() < 1e-6 { format!("{}", n.round()) } else { format!("{n:.3}") };
    let txt = if txt.contains('.') {
        txt.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        txt
    };
    format!("{txt}{prefix}{unit}")
}

pub fn keyword(spec: &Spec) -> Option<String> {
    match spec {
        Spec::Resistor { ohms, tolerance, package } => {
            Some(format!("{} {}% {package} resistor", eng(*ohms, "ohm"), tolerance))
        }
        Spec::Capacitor { farads, volts, dielectric, package, .. } => {
            let mut k = eng(*farads, "F");
            if let Some(v) = volts {
                k.push_str(&format!(" {v}V"));
            }
            if let Some(d) = dielectric {
                k.push_str(&format!(" {d}"));
            }
            Some(format!("{k} {package} ceramic capacitor"))
        }
        Spec::Generic { name, package } => Some(format!("{name} {package}")),
        Spec::Led { color, package } => Some(format!("{color} LED {package}")),
        Spec::Specific => None,
    }
}

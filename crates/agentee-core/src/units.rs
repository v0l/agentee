use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

pub const NM_PER_MM: f64 = 1_000_000.0;
pub const OZ_COPPER_MM: f64 = 0.035;
pub const MM_PER_MIL: f64 = 0.0254;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Length(pub i64);

impl Length {
    pub const ZERO: Length = Length(0);

    pub fn mm(v: f64) -> Self {
        Length((v * NM_PER_MM).round() as i64)
    }

    pub fn mil(v: f64) -> Self {
        Self::mm(v * MM_PER_MIL)
    }

    pub fn to_mm(self) -> f64 {
        self.0 as f64 / NM_PER_MM
    }

    pub fn to_mil(self) -> f64 {
        self.to_mm() / MM_PER_MIL
    }

    pub fn abs(self) -> Self {
        Length(self.0.abs())
    }

    pub fn max(self, o: Self) -> Self {
        Length(self.0.max(o.0))
    }

    pub fn min(self, o: Self) -> Self {
        Length(self.0.min(o.0))
    }

    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let (v, unit) = split_number(s)?;
        let mm = match unit.to_ascii_lowercase().as_str() {
            "" | "mm" => v,
            "um" | "µm" | "μm" => v / 1000.0,
            "cm" => v * 10.0,
            "mil" | "mils" | "thou" => v * MM_PER_MIL,
            "in" | "inch" | "\"" => v * 25.4,
            "oz" => v * OZ_COPPER_MM,
            u => {
                return Err(format!(
                    "unknown length unit `{u}` in `{s}` (use mm, um, mil, in, oz)"
                ));
            }
        };
        Ok(Length::mm(mm))
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}mm", trim(self.to_mm(), 4))
    }
}

pub fn trim(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".into() } else { s.into() }
}

impl Add for Length {
    type Output = Length;
    fn add(self, o: Length) -> Length {
        Length(self.0 + o.0)
    }
}

impl Sub for Length {
    type Output = Length;
    fn sub(self, o: Length) -> Length {
        Length(self.0 - o.0)
    }
}

impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length {
        Length(-self.0)
    }
}

impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, k: f64) -> Length {
        Length((self.0 as f64 * k).round() as i64)
    }
}

impl Div<f64> for Length {
    type Output = Length;
    fn div(self, k: f64) -> Length {
        Length((self.0 as f64 / k).round() as i64)
    }
}

impl std::iter::Sum for Length {
    fn sum<I: Iterator<Item = Length>>(iter: I) -> Length {
        iter.fold(Length::ZERO, |a, b| a + b)
    }
}

impl Serialize for Length {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(trim(self.to_mm(), 6).parse::<f64>().unwrap_or(0.0))
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(QuantityVisitor {
            what: "a length",
            parse: Length::parse,
            num: Length::mm,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Point(pub Length, pub Length);

impl Point {
    pub const ZERO: Point = Point(Length::ZERO, Length::ZERO);

    pub fn mm(x: f64, y: f64) -> Self {
        Point(Length::mm(x), Length::mm(y))
    }

    pub fn x(&self) -> Length {
        self.0
    }

    pub fn y(&self) -> Length {
        self.1
    }

    pub fn to_mm(self) -> [f64; 2] {
        [self.0.to_mm(), self.1.to_mm()]
    }

    pub fn rotated(self, deg: f64) -> Point {
        if deg == 0.0 {
            return self;
        }
        let (s, c) = deg.to_radians().sin_cos();
        let [x, y] = self.to_mm();
        Point::mm(x * c - y * s, x * s + y * c)
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point(self.0 + o.0, self.1 + o.1)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point(self.0 - o.0, self.1 - o.1)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}]", trim(self.0.to_mm(), 4), trim(self.1.to_mm(), 4))
    }
}

macro_rules! scalar {
    ($name:ident, $what:literal, $parse:expr, $unit:literal) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
        pub struct $name(pub f64);

        impl $name {
            pub fn parse(s: &str) -> Result<Self, String> {
                let f: fn(&str) -> Result<f64, String> = $parse;
                f(s).map($name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", trim(self.0, 3), $unit)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                d.deserialize_any(QuantityVisitor { what: $what, parse: $name::parse, num: $name })
            }
        }
    };
}

scalar!(
    Amps,
    "a current",
    |s| {
        let (v, u) = split_number(s)?;
        match u.to_ascii_lowercase().as_str() {
            "" | "a" => Ok(v),
            "ma" => Ok(v / 1000.0),
            u => Err(format!("unknown current unit `{u}` (use A or mA)")),
        }
    },
    "A"
);

scalar!(
    Ohms,
    "an impedance",
    |s| {
        let (v, u) = split_number(s)?;
        match u.to_ascii_lowercase().as_str() {
            "" | "ohm" | "ohms" | "r" | "Ω" | "ω" => Ok(v),
            u => Err(format!("unknown impedance unit `{u}` (use ohm)")),
        }
    },
    "ohm"
);

scalar!(
    Kelvin,
    "a temperature rise",
    |s| {
        let (v, u) = split_number(s)?;
        match u.to_ascii_lowercase().as_str() {
            "" | "c" | "k" | "°c" | "degc" => Ok(v),
            u => Err(format!("unknown temperature unit `{u}` (use C)")),
        }
    },
    "C"
);

scalar!(
    Percent,
    "a percentage",
    |s| {
        let (v, u) = split_number(s)?;
        match u {
            "" | "%" => Ok(v),
            u => Err(format!("unknown percentage unit `{u}` (use %)")),
        }
    },
    "%"
);

scalar!(
    Picos,
    "a time",
    |s| {
        let (v, u) = split_number(s)?;
        match u.to_ascii_lowercase().as_str() {
            "" | "ps" => Ok(v),
            "fs" => Ok(v / 1000.0),
            "ns" => Ok(v * 1000.0),
            "us" => Ok(v * 1e6),
            u => Err(format!("unknown time unit `{u}` (use ps or ns)")),
        }
    },
    "ps"
);

fn split_number(s: &str) -> Result<(f64, &str), String> {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|&(i, c)| {
            !(c.is_ascii_digit()
                || c == '.'
                || ((c == '-' || c == '+') && i == 0)
                || ((c == 'e' || c == 'E')
                    && s[i + 1..].starts_with(|n: char| n.is_ascii_digit() || n == '-')))
        })
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let v: f64 = s[..end].parse().map_err(|_| format!("`{s}` does not start with a number"))?;
    Ok((v, s[end..].trim()))
}

struct QuantityVisitor<T> {
    what: &'static str,
    parse: fn(&str) -> Result<T, String>,
    num: fn(f64) -> T,
}

impl<T> Visitor<'_> for QuantityVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} as a number or a string with a unit", self.what)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<T, E> {
        Ok((self.num)(v as f64))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<T, E> {
        Ok((self.num)(v as f64))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<T, E> {
        Ok((self.num)(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<T, E> {
        (self.parse)(v).map_err(E::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_take_units() {
        assert_eq!(Length::parse("0.2mm").unwrap(), Length(200_000));
        assert_eq!(Length::parse("8mil").unwrap(), Length(203_200));
        assert_eq!(Length::parse("1oz").unwrap(), Length(35_000));
        assert_eq!(Length::parse("35um").unwrap(), Length(35_000));
        assert_eq!(Length::parse("1.6").unwrap(), Length(1_600_000));
        assert_eq!(Length::parse("-2.54").unwrap(), Length(-2_540_000));
        assert!(Length::parse("3furlong").is_err());
    }

    #[test]
    fn scalars_take_units() {
        assert_eq!(Amps::parse("500mA").unwrap().0, 0.5);
        assert_eq!(Ohms::parse("90 ohm").unwrap().0, 90.0);
        assert_eq!(Kelvin::parse("10C").unwrap().0, 10.0);
    }

    #[test]
    fn lengths_round_trip_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct T {
            a: Length,
            b: Point,
        }
        let t: T = toml::from_str("a = \"0.127mm\"\nb = [1, \"2.54mm\"]").unwrap();
        assert_eq!(t.a, Length(127_000));
        assert_eq!(t.b, Point::mm(1.0, 2.54));
        let back: T = toml::from_str(&toml::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn display_trims() {
        assert_eq!(Length::mm(0.2).to_string(), "0.2mm");
        assert_eq!(Length::mm(1.0).to_string(), "1mm");
    }
}

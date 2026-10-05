use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Break {
    pub qty: u32,
    pub price: f64,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Offer {
    pub distributor: String,
    pub sku: String,
    pub manufacturer: String,
    pub mpn: String,
    pub description: String,
    pub stock: u64,
    pub lifecycle: Option<String>,
    pub currency: String,
    pub breaks: Vec<Break>,
    pub min: u32,
    pub mult: u32,
    pub url: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub replacement: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Cost {
    pub qty: u32,
    pub unit: f64,
    pub total: f64,
}

impl Offer {
    pub fn order_qty(&self, need: u32) -> u32 {
        let q = need.max(self.min.max(1));
        let m = self.mult.max(1);
        q.div_ceil(m) * m
    }

    pub fn unit_price(&self, qty: u32) -> Option<f64> {
        self.breaks.iter().filter(|b| b.qty <= qty).max_by_key(|b| b.qty).map(|b| b.price)
    }

    pub fn cost(&self, need: u32) -> Option<Cost> {
        let base = self.order_qty(need);
        let mut candidates = vec![base];
        candidates.extend(self.breaks.iter().map(|b| self.order_qty(b.qty)).filter(|&q| q > base));
        candidates
            .into_iter()
            .filter_map(|qty| {
                let unit = self.unit_price(qty)?;
                Some(Cost { qty, unit, total: unit * qty as f64 })
            })
            .min_by(|a, b| a.total.total_cmp(&b.total).then(a.qty.cmp(&b.qty)))
    }

    pub fn attribute(&self, name_contains: &[&str]) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| {
                let k = k.to_lowercase();
                name_contains.iter().all(|n| k.contains(&n.to_lowercase()))
            })
            .map(|(_, v)| v.as_str())
    }

    pub fn active(&self) -> bool {
        !self.lifecycle.as_deref().is_some_and(|l| {
            let l = l.to_lowercase();
            l.contains("obsolete")
                || l.contains("end of life")
                || l.contains("discontinued")
                || l == "eol"
        })
    }
}

pub fn parse_price(s: &str) -> Option<f64> {
    let digits: String =
        s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    if digits.is_empty() {
        return None;
    }
    let normalised = match (digits.rfind('.'), digits.rfind(',')) {
        (Some(d), Some(c)) if c > d => digits.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => digits.replace(',', ""),
        (None, Some(c)) if &digits[..c] == "0" || digits.len() - c - 1 != 3 || s.contains('€') => {
            digits.replace(',', ".")
        }
        (None, Some(_)) => digits.replace(',', ""),
        _ => digits,
    };
    normalised.parse().ok()
}

pub fn parse_count(s: &str) -> u64 {
    let digits: String =
        s.split_whitespace().next().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

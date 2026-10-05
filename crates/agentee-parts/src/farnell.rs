use crate::Distributor;
use crate::offer::{Break, Offer};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://api.element14.com/catalog/products";
const GAP: Duration = Duration::from_millis(550);

pub struct Farnell {
    key: String,
    pub store: String,
    currency: String,
    agent: ureq::Agent,
    last: Mutex<Option<Instant>>,
    calls: Mutex<u32>,
}

pub fn store_currency(store: &str) -> &'static str {
    let s = store.to_lowercase();
    let country = s.split('.').next().unwrap_or("");
    if s.contains("newark") {
        return if country == "canada" { "CAD" } else { "USD" };
    }
    match country {
        "uk" => "GBP",
        "ch" => "CHF",
        "se" => "SEK",
        "dk" => "DKK",
        "no" => "NOK",
        "pl" => "PLN",
        "cz" => "CZK",
        "hu" => "HUF",
        "ro" => "RON",
        "bg" => "BGN",
        "il" => "ILS",
        "au" => "AUD",
        "nz" => "NZD",
        "sg" => "SGD",
        "my" => "MYR",
        "in" => "INR",
        "cn" => "CNY",
        "hk" => "HKD",
        "tw" => "TWD",
        "kr" => "KRW",
        "th" => "THB",
        "ph" => "PHP",
        "vn" => "VND",
        "id" => "IDR",
        _ => "EUR",
    }
}

impl Farnell {
    pub fn new(key: &str, store: &str, currency: Option<&str>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            key: key.to_string(),
            store: store.to_string(),
            currency: currency
                .map(str::to_string)
                .unwrap_or_else(|| store_currency(store).to_string()),
            agent,
            last: Mutex::new(None),
            calls: Mutex::new(0),
        }
    }

    fn get(&self, term: &str, results: u32) -> Result<Value, String> {
        {
            let mut last = self.last.lock().unwrap();
            if let Some(t) = *last {
                std::thread::sleep(GAP.saturating_sub(t.elapsed()));
            }
            *last = Some(Instant::now());
        }
        *self.calls.lock().unwrap() += 1;
        let results = results.to_string();
        let mut resp = self
            .agent
            .get(BASE)
            .query("term", term)
            .query("storeInfo.id", &self.store)
            .query("resultsSettings.offset", "0")
            .query("resultsSettings.numberOfResults", &results)
            .query("resultsSettings.responseGroup", "large")
            .query("callInfo.omitXmlSchema", "false")
            .query("callInfo.responseDataFormat", "json")
            .query("callInfo.apiKey", &self.key)
            .call()
            .map_err(|e| format!("Farnell: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(|e| format!("Farnell: {e}"))?;
        if status != 200 {
            let snippet: String = body.chars().take(200).collect();
            return Err(format!("Farnell: HTTP {status}: {snippet}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("Farnell: {e}"))
    }
}

impl Distributor for Farnell {
    fn name(&self) -> &'static str {
        "Farnell"
    }

    fn by_mpn(&self, mpns: &[String]) -> Result<Vec<Offer>, String> {
        let mut out = Vec::new();
        for mpn in mpns {
            let v = self.get(&format!("manuPartNum:{mpn}"), 10)?;
            out.extend(parse(&v, &self.store, &self.currency));
        }
        Ok(out)
    }

    fn search(&self, keyword: &str) -> Result<Vec<Offer>, String> {
        let v = self.get(&format!("any:{keyword}"), 50)?;
        Ok(parse(&v, &self.store, &self.currency).into_iter().filter(|o| o.stock > 0).collect())
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn parse(v: &Value, store: &str, currency: &str) -> Vec<Offer> {
    let Some(obj) = v.as_object() else { return Vec::new() };
    let Some(ret) = obj.iter().find(|(k, _)| k.ends_with("Return")).map(|(_, v)| v) else {
        return Vec::new();
    };
    let Some(products) = ret.get("products").and_then(Value::as_array) else {
        return Vec::new();
    };
    products.iter().map(|p| product(p, store, currency)).collect()
}

fn product(p: &Value, store: &str, currency: &str) -> Offer {
    let mut breaks: Vec<Break> = p
        .get("prices")
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .filter_map(|b| {
                    Some(Break { qty: num(b.get("from")?)? as u32, price: num(b.get("cost")?)? })
                })
                .collect()
        })
        .unwrap_or_default();
    breaks.sort_by_key(|b| b.qty);
    let stock = p
        .pointer("/stock/level")
        .and_then(num)
        .or_else(|| p.get("inv").and_then(num))
        .unwrap_or(0.0) as u64;
    let mut attributes = std::collections::BTreeMap::new();
    for a in p.get("attributes").and_then(Value::as_array).into_iter().flatten() {
        let label = s(a, "attributeLabel");
        if label.is_empty() {
            continue;
        }
        let mut val = s(a, "attributeValue").to_string();
        let unit = s(a, "attributeUnit");
        if !unit.is_empty() && !val.ends_with(unit) {
            val.push_str(unit);
        }
        attributes.insert(label.to_string(), val);
    }
    let sku = s(p, "sku").to_string();
    let min = p
        .get("translatedMinimumOrderQuality")
        .and_then(num)
        .map(|m| m as u32)
        .filter(|&m| m > 0)
        .or_else(|| breaks.first().map(|b| b.qty))
        .unwrap_or(1);
    let status = s(p, "productStatus");
    Offer {
        distributor: "Farnell".into(),
        url: (!sku.is_empty()).then(|| format!("https://{store}/search?st={sku}")),
        sku,
        manufacturer: match s(p, "brandName") {
            "" => s(p, "vendorName").into(),
            b => b.into(),
        },
        mpn: s(p, "translatedManufacturerPartNumber").into(),
        description: s(p, "displayName").into(),
        stock,
        lifecycle: match status {
            "" | "STOCKED" => None,
            other => Some(other.to_string()),
        },
        currency: currency.to_string(),
        breaks,
        min,
        mult: 1,
        attributes,
        replacement: None,
    }
}

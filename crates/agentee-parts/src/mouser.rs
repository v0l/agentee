use crate::Distributor;
use crate::offer::{Break, Offer, parse_count, parse_price};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://api.mouser.com/api/v1/search";
const GAP: Duration = Duration::from_millis(2100);
const BATCH: usize = 10;

pub struct Mouser {
    key: String,
    agent: ureq::Agent,
    last: Mutex<Option<Instant>>,
    pub calls: Mutex<u32>,
}

impl Mouser {
    pub fn new(key: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { key: key.to_string(), agent, last: Mutex::new(None), calls: Mutex::new(0) }
    }

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        {
            let mut last = self.last.lock().unwrap();
            if let Some(t) = *last {
                let wait = GAP.saturating_sub(t.elapsed());
                std::thread::sleep(wait);
            }
            *last = Some(Instant::now());
        }
        *self.calls.lock().unwrap() += 1;
        let mut resp = self
            .agent
            .post(&format!("{BASE}/{path}?apiKey={}", self.key))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| format!("Mouser: {e}"))?;
        let status = resp.status().as_u16();
        let text =
            resp.body_mut().read_to_string().map_err(|e| format!("Mouser: HTTP {status}, {e}"))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("Mouser: HTTP {status}, {e}"))?;
        if let Some(err) = errors(&v) {
            return Err(format!("Mouser: {err}"));
        }
        if status != 200 {
            return Err(format!("Mouser: HTTP {status}"));
        }
        Ok(v)
    }
}

impl Distributor for Mouser {
    fn name(&self) -> &'static str {
        "Mouser"
    }

    fn by_mpn(&self, mpns: &[String]) -> Result<Vec<Offer>, String> {
        let mut out = Vec::new();
        for chunk in mpns.chunks(BATCH) {
            let v = self.post(
                "partnumber",
                json!({ "SearchByPartRequest": { "mouserPartNumber": chunk.join("|"), "partSearchOptions": "Exact" } }),
            )?;
            out.extend(parse(&v));
        }
        Ok(out)
    }

    fn search(&self, keyword: &str) -> Result<Vec<Offer>, String> {
        let v = self.post(
            "keyword",
            json!({ "SearchByKeywordRequest": { "keyword": keyword, "records": 50, "startingRecord": 0, "searchOptions": "InStock" } }),
        )?;
        Ok(parse(&v))
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

fn errors(v: &Value) -> Option<String> {
    let errs = v.get("Errors")?.as_array()?;
    let msgs: Vec<String> = errs
        .iter()
        .map(|e| {
            let msg = e.get("Message").and_then(Value::as_str).unwrap_or("error");
            match e.get("PropertyName").and_then(Value::as_str) {
                Some(p) if !p.is_empty() => format!("{msg} ({p})"),
                _ => msg.to_string(),
            }
        })
        .collect();
    (!msgs.is_empty()).then(|| msgs.join("; "))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

pub fn parse(v: &Value) -> Vec<Offer> {
    let Some(parts) = v.pointer("/SearchResults/Parts").and_then(Value::as_array) else {
        return Vec::new();
    };
    parts.iter().map(part).collect()
}

fn part(p: &Value) -> Offer {
    let breaks: Vec<Break> = p
        .get("PriceBreaks")
        .and_then(Value::as_array)
        .map(|bs| {
            bs.iter()
                .filter_map(|b| {
                    let qty = b.get("Quantity").and_then(Value::as_u64)? as u32;
                    let price = parse_price(b.get("Price").and_then(Value::as_str)?)?;
                    Some(Break { qty, price })
                })
                .collect()
        })
        .unwrap_or_default();
    let currency =
        p.pointer("/PriceBreaks/0/Currency").and_then(Value::as_str).unwrap_or("").to_string();
    let stock = match parse_count(s(p, "AvailabilityInStock")) {
        0 => parse_count(s(p, "Availability")),
        n => n,
    };
    let mut attributes = std::collections::BTreeMap::new();
    for a in p.get("ProductAttributes").and_then(Value::as_array).into_iter().flatten() {
        let (k, val) = (s(a, "AttributeName"), s(a, "AttributeValue"));
        if k.is_empty() {
            continue;
        }
        attributes
            .entry(k.to_string())
            .and_modify(|e: &mut String| {
                if !e.split(", ").any(|x| x == val) {
                    e.push_str(", ");
                    e.push_str(val);
                }
            })
            .or_insert_with(|| val.to_string());
    }
    let mut lifecycle = Some(s(p, "LifecycleStatus").to_string()).filter(|l| !l.is_empty());
    if s(p, "IsDiscontinued").eq_ignore_ascii_case("true") {
        lifecycle = Some("Discontinued".into());
    }
    Offer {
        distributor: "Mouser".into(),
        sku: s(p, "MouserPartNumber").into(),
        manufacturer: s(p, "Manufacturer").into(),
        mpn: s(p, "ManufacturerPartNumber").into(),
        description: s(p, "Description").into(),
        stock,
        lifecycle,
        currency,
        breaks,
        min: s(p, "Min").parse().unwrap_or(1),
        mult: s(p, "Mult").parse().unwrap_or(1),
        url: Some(s(p, "ProductDetailUrl").to_string()).filter(|u| !u.is_empty()),
        attributes,
        replacement: Some(s(p, "SuggestedReplacement").to_string()).filter(|r| !r.is_empty()),
    }
}

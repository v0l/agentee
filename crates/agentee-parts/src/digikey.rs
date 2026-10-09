use crate::offer::{Break, Offer};
use crate::{Distributor, same_mpn};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const GAP: Duration = Duration::from_millis(600);

pub struct DigiKey {
    client_id: String,
    secret: String,
    host: String,
    site: String,
    language: String,
    currency: String,
    agent: ureq::Agent,
    last: Mutex<Option<Instant>>,
    calls: Mutex<u32>,
}

struct Token {
    client_id: String,
    host: String,
    bearer: String,
    until: Instant,
}

static TOKEN: Mutex<Option<Token>> = Mutex::new(None);

pub struct Locale<'a> {
    pub site: Option<&'a str>,
    pub language: Option<&'a str>,
    pub currency: Option<&'a str>,
    pub sandbox: bool,
}

pub fn site_currency(site: &str) -> &'static str {
    match site.to_uppercase().as_str() {
        "US" | "PR" => "USD",
        "CA" => "CAD",
        "UK" | "GB" => "GBP",
        "CH" => "CHF",
        "SE" => "SEK",
        "DK" => "DKK",
        "NO" => "NOK",
        "PL" => "PLN",
        "CZ" => "CZK",
        "HU" => "HUF",
        "IL" => "ILS",
        "AU" => "AUD",
        "NZ" => "NZD",
        "SG" => "SGD",
        "MY" => "MYR",
        "IN" => "INR",
        "CN" => "CNY",
        "HK" => "HKD",
        "TW" => "TWD",
        "KR" => "KRW",
        "JP" => "JPY",
        "TH" => "THB",
        "PH" => "PHP",
        "MX" => "USD",
        "AT" | "BE" | "DE" | "EE" | "ES" | "FI" | "FR" | "GR" | "IE" | "IT" | "LT" | "LU"
        | "LV" | "NL" | "PT" | "SI" | "SK" => "EUR",
        _ => "USD",
    }
}

impl DigiKey {
    pub fn new(client_id: &str, secret: &str, locale: Locale) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let site = locale.site.unwrap_or("US").to_uppercase();
        Self {
            client_id: client_id.to_string(),
            secret: secret.to_string(),
            host: if locale.sandbox { "sandbox-api.digikey.com" } else { "api.digikey.com" }
                .to_string(),
            currency: locale
                .currency
                .map(str::to_uppercase)
                .unwrap_or_else(|| site_currency(&site).to_string()),
            language: locale.language.unwrap_or("en").to_string(),
            site,
            agent,
            last: Mutex::new(None),
            calls: Mutex::new(0),
        }
    }

    fn pace(&self) {
        let mut last = self.last.lock().unwrap();
        if let Some(t) = *last {
            std::thread::sleep(GAP.saturating_sub(t.elapsed()));
        }
        *last = Some(Instant::now());
        *self.calls.lock().unwrap() += 1;
    }

    fn token(&self) -> Result<String, String> {
        let mut cached = TOKEN.lock().unwrap();
        if let Some(t) = cached.as_ref()
            && t.client_id == self.client_id
            && t.host == self.host
            && Instant::now() < t.until
        {
            return Ok(t.bearer.clone());
        }
        self.pace();
        let mut resp = self
            .agent
            .post(&format!("https://{}/v1/oauth2/token", self.host))
            .send_form([
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.secret.as_str()),
                ("grant_type", "client_credentials"),
            ])
            .map_err(|e| format!("DigiKey: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(|e| format!("DigiKey: {e}"))?;
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let Some(bearer) = v.get("access_token").and_then(Value::as_str) else {
            return Err(format!("DigiKey: token refused, HTTP {status}: {}", problem(&v, &body)));
        };
        let life = v.get("expires_in").and_then(Value::as_u64).unwrap_or(600);
        *cached = Some(Token {
            client_id: self.client_id.clone(),
            host: self.host.clone(),
            bearer: bearer.to_string(),
            until: Instant::now() + Duration::from_secs(life.saturating_sub(30)),
        });
        Ok(bearer.to_string())
    }

    fn keyword(&self, keywords: &str, limit: u32) -> Result<Value, String> {
        let bearer = self.token()?;
        self.pace();
        let mut resp = self
            .agent
            .post(&format!("https://{}/products/v4/search/keyword", self.host))
            .header("Authorization", &format!("Bearer {bearer}"))
            .header("X-DIGIKEY-Client-Id", &self.client_id)
            .header("X-DIGIKEY-Locale-Site", &self.site)
            .header("X-DIGIKEY-Locale-Language", &self.language)
            .header("X-DIGIKEY-Locale-Currency", &self.currency)
            .header("Content-Type", "application/json")
            .send(json!({ "Keywords": keywords, "Limit": limit, "Offset": 0 }).to_string())
            .map_err(|e| format!("DigiKey: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(|e| format!("DigiKey: {e}"))?;
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if status == 401 {
            TOKEN.lock().unwrap().take();
        }
        if status != 200 {
            return Err(format!("DigiKey: HTTP {status}: {}", problem(&v, &body)));
        }
        Ok(v)
    }
}

fn problem(v: &Value, body: &str) -> String {
    ["detail", "title", "ErrorMessage", "error_description", "error"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(200).collect())
}

impl Distributor for DigiKey {
    fn name(&self) -> &'static str {
        "DigiKey"
    }

    fn by_mpn(&self, mpns: &[String]) -> Result<Vec<Offer>, String> {
        let mut out = Vec::new();
        for mpn in mpns {
            let v = self.keyword(mpn, 10)?;
            out.extend(parse(&v, &self.currency).into_iter().filter(|o| same_mpn(&o.mpn, mpn)));
        }
        Ok(out)
    }

    fn search(&self, keyword: &str) -> Result<Vec<Offer>, String> {
        let v = self.keyword(keyword, 50)?;
        Ok(parse(&v, &self.currency).into_iter().filter(|o| o.stock > 0).collect())
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

fn s<'a>(v: &'a Value, path: &str) -> &'a str {
    v.pointer(path).and_then(Value::as_str).unwrap_or("")
}

fn num(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn parse(v: &Value, currency: &str) -> Vec<Offer> {
    let currency = match s(v, "/SearchLocaleUsed/Currency") {
        "" => currency,
        c => c,
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for key in ["ExactMatches", "Products"] {
        for p in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            for o in product(p, currency) {
                if seen.insert(o.sku.clone()) {
                    out.push(o);
                }
            }
        }
    }
    if let Some(p) = v.get("Product") {
        out.extend(product(p, currency));
    }
    out
}

fn imperial_only(v: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    let mut held = String::new();
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                held.clear();
            }
            ')' if depth > 0 => {
                depth -= 1;
                if !held.to_lowercase().contains("metric") {
                    out.push('(');
                    out.push_str(&held);
                    out.push(')');
                }
            }
            _ if depth > 0 => held.push(c),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lifecycle(p: &Value) -> Option<String> {
    let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("Discontinued") {
        return Some("Discontinued".into());
    }
    if flag("EndOfLife") {
        return Some("End of Life".into());
    }
    match s(p, "/ProductStatus/Status") {
        "" | "Active" => None,
        other => Some(other.to_string()),
    }
}

fn product(p: &Value, currency: &str) -> Vec<Offer> {
    let mut attributes = std::collections::BTreeMap::new();
    for a in p.get("Parameters").and_then(Value::as_array).into_iter().flatten() {
        let (k, val) = (s(a, "/ParameterText"), s(a, "/ValueText"));
        if k.is_empty() || val.is_empty() || val == "-" {
            continue;
        }
        let lower = k.to_lowercase();
        let val = if lower.contains("package") || lower.contains("case") {
            imperial_only(val)
        } else {
            val.to_string()
        };
        if lower == "temperature coefficient" && crate::spec::dielectric(&val).is_some() {
            attributes.insert("Dielectric".into(), val.clone());
        }
        attributes.insert(k.to_string(), val);
    }
    let mut category = p.get("Category");
    let mut names = Vec::new();
    while let Some(c) = category {
        names.push(s(c, "/Name"));
        category = c.pointer("/ChildCategories/0");
    }
    if !names.is_empty() {
        attributes.insert("Category".into(), names.join(" / "));
    }
    let description = match s(p, "/Description/DetailedDescription") {
        "" => s(p, "/Description/ProductDescription"),
        d => d,
    };
    let base = Offer {
        distributor: "DigiKey".into(),
        manufacturer: s(p, "/Manufacturer/Name").into(),
        mpn: s(p, "/ManufacturerProductNumber").into(),
        description: description.into(),
        lifecycle: lifecycle(p),
        currency: currency.to_string(),
        url: Some(s(p, "/ProductUrl").to_string()).filter(|u| !u.is_empty()),
        datasheet: Some(s(p, "/DatasheetUrl").to_string()).filter(|u| u.starts_with("http")),
        attributes,
        ..Default::default()
    };
    let variations = p.get("ProductVariations").and_then(Value::as_array);
    variations
        .into_iter()
        .flatten()
        .filter(|var| !var.get("MarketPlace").and_then(Value::as_bool).unwrap_or(false))
        .filter(|var| !s(var, "/PackageType/Name").starts_with("Digi-Reel"))
        .filter_map(|var| {
            let sku = s(var, "/DigiKeyProductNumber");
            if sku.is_empty() {
                return None;
            }
            let mut breaks: Vec<Break> = var
                .get("StandardPricing")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    Some(Break {
                        qty: num(b.get("BreakQuantity"))? as u32,
                        price: num(b.get("UnitPrice"))?,
                    })
                })
                .collect();
            breaks.sort_by_key(|b| b.qty);
            let min = num(var.get("MinimumOrderQuantity"))
                .map(|m| m as u32)
                .filter(|&m| m > 0)
                .or_else(|| breaks.first().map(|b| b.qty))
                .unwrap_or(1);
            let mut o = base.clone();
            o.sku = sku.to_string();
            o.stock = num(var.get("QuantityAvailableforPackageType"))
                .or_else(|| num(p.get("QuantityAvailable")))
                .unwrap_or(0.0) as u64;
            o.min = min;
            o.mult = 1;
            o.breaks = breaks;
            if let Some(pack) = var.pointer("/PackageType/Name").and_then(Value::as_str) {
                o.attributes.insert("Packaging".into(), pack.to_string());
            }
            Some(o)
        })
        .collect()
}

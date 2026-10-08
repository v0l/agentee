pub mod config;
pub mod cost;
pub mod farnell;
pub mod mouser;
pub mod offer;
pub mod order;
pub mod spec;

use agentee_core::schematic::Schematic;
use offer::{Break, Cost, Offer};
use serde::Serialize;
use spec::Spec;
use std::collections::{BTreeMap, HashMap};

pub trait Distributor {
    fn name(&self) -> &'static str;
    fn by_mpn(&self, mpns: &[String]) -> Result<Vec<Offer>, String>;
    fn search(&self, keyword: &str) -> Result<Vec<Offer>, String>;
    fn calls(&self) -> u32 {
        0
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BomLine {
    pub refs: Vec<String>,
    pub value: String,
    pub footprint: String,
    pub manufacturer: Option<String>,
    pub mpn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spares: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub buy_with: Vec<(String, u32)>,
}

pub fn bom(sch: &Schematic) -> Vec<BomLine> {
    let mut seen = std::collections::HashSet::new();
    let mut lines: Vec<BomLine> = Vec::new();
    for p in &sch.parts {
        if !seen.insert(p.reference.as_str()) {
            continue;
        }
        let units: Vec<_> = sch.parts.iter().filter(|u| u.reference == p.reference).collect();
        let field =
            |k: &str| units.iter().find_map(|u| u.fields.get(k).cloned()).filter(|v| !v.is_empty());
        let footprint = units.iter().find_map(|u| u.footprint.clone()).unwrap_or_default();
        let skip = units.iter().any(|u| u.dnp)
            || field("assembly").as_deref() == Some("no")
            || footprint.starts_with("MountingHole")
            || footprint.starts_with("Fiducial");
        if skip {
            continue;
        }
        let (manufacturer, mpn) =
            match (field("mfr").or_else(|| field("manufacturer")), field("mpn")) {
                (None, Some(m)) => match split_maker(&m) {
                    Some((maker, number)) => (Some(maker), Some(number)),
                    None => (None, Some(m)),
                },
                other => other,
            };
        let same = |l: &BomLine| match (&l.mpn, &mpn) {
            (Some(a), Some(b)) => a == b,
            (None, None) => l.value == p.value && l.footprint == footprint,
            _ => false,
        };
        match lines.iter_mut().find(|l| same(l)) {
            Some(l) => l.refs.push(p.reference.clone()),
            None => lines.push(BomLine {
                refs: vec![p.reference.clone()],
                value: p.value.clone(),
                footprint: footprint.clone(),
                manufacturer,
                mpn,
                lcsc: field("lcsc"),
                spares: field("spares").and_then(|s| s.trim().parse().ok()),
                buy_with: field("buy_with").map(|s| buy_with(&s)).unwrap_or_default(),
            }),
        }
    }
    for l in &mut lines {
        l.refs.sort_by_key(|r| ref_key(r));
    }
    lines.sort_by_key(|l| ref_key(&l.refs[0]));
    lines
}

fn buy_with(s: &str) -> Vec<(String, u32)> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| match t.rsplit_once(" x").and_then(|(m, n)| Some((m, n.trim().parse().ok()?))) {
            Some((m, n)) => (m.trim().to_string(), n),
            None => (t.to_string(), 1),
        })
        .collect()
}

fn split_maker(mpn: &str) -> Option<(String, String)> {
    let (maker, number) = mpn.trim().split_once(char::is_whitespace)?;
    let number = number.trim();
    (!number.contains(char::is_whitespace) && number.chars().any(|c| c.is_ascii_digit()))
        .then(|| (maker.to_string(), number.to_string()))
}

pub(crate) fn ref_key(r: &str) -> (String, u64, String) {
    let prefix: String = r.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let digits: String = r[prefix.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
    (prefix.clone(), digits.parse().unwrap_or(0), r[prefix.len() + digits.len()..].to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct Priced {
    pub distributor: String,
    pub sku: String,
    pub manufacturer: String,
    pub mpn: String,
    pub description: String,
    pub stock: u64,
    pub lifecycle: Option<String>,
    pub currency: String,
    pub buy: u32,
    pub unit: f64,
    pub total: f64,
    pub url: Option<String>,
    pub breaks: Vec<Break>,
    pub min: u32,
    pub mult: u32,
}

impl Priced {
    pub fn at(&self, need: u32) -> Option<Cost> {
        let o = Offer {
            distributor: self.distributor.clone(),
            sku: self.sku.clone(),
            stock: self.stock,
            lifecycle: self.lifecycle.clone(),
            currency: self.currency.clone(),
            breaks: self.breaks.clone(),
            min: self.min,
            mult: self.mult,
            ..Default::default()
        };
        o.cost(need).filter(|c| o.active() && o.stock >= c.qty as u64)
    }

    fn new(o: &Offer, c: Cost) -> Self {
        Self {
            distributor: o.distributor.clone(),
            sku: o.sku.clone(),
            manufacturer: o.manufacturer.clone(),
            mpn: o.mpn.clone(),
            description: o.description.clone(),
            stock: o.stock,
            lifecycle: o.lifecycle.clone(),
            currency: o.currency.clone(),
            buy: c.qty,
            unit: c.unit,
            total: c.total,
            url: o.url.clone(),
            breaks: o.breaks.clone(),
            min: o.min,
            mult: o.mult,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Alternative {
    #[serde(flatten)]
    pub part: Priced,
    pub saves: Option<f64>,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    #[serde(flatten)]
    pub bom: BomLine,
    pub need: u32,
    pub spec: Spec,
    pub chosen: Option<Priced>,
    pub offers: Vec<Priced>,
    pub alternatives: Vec<Alternative>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Total {
    pub chosen: f64,
    pub cheapest: f64,
    pub lines_priced: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub boards: u32,
    pub lines: Vec<Line>,
    pub totals: BTreeMap<String, Total>,
    pub calls: BTreeMap<String, u32>,
    pub errors: Vec<String>,
}

pub struct Options {
    pub boards: u32,
    pub alternatives: bool,
    pub max_alternatives: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { boards: 1, alternatives: true, max_alternatives: 3 }
    }
}

pub(crate) fn same_mpn(a: &str, b: &str) -> bool {
    let n = |s: &str| {
        s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
    };
    n(a) == n(b)
}

fn cheapest_in_stock(offers: &[Offer], need: u32) -> Option<(&Offer, Cost)> {
    offers
        .iter()
        .filter_map(|o| o.cost(need).map(|c| (o, c)))
        .filter(|(o, c)| o.stock >= c.qty as u64 && o.active())
        .min_by(|a, b| a.1.total.total_cmp(&b.1.total))
}

pub fn report(lines: &[BomLine], sources: &[&dyn Distributor], opts: &Options) -> Report {
    let mut errors = Vec::new();
    let mpns: Vec<String> = lines.iter().filter_map(|l| l.mpn.clone()).collect();
    let mut by_source: Vec<Vec<Offer>> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    for s in sources {
        match s.by_mpn(&mpns) {
            Ok(o) => by_source.push(o),
            Err(e) => {
                errors.push(e);
                failed.push(s.name());
                by_source.push(Vec::new());
            }
        }
    }
    let working: Vec<&str> =
        sources.iter().map(|s| s.name()).filter(|n| !failed.contains(n)).collect();
    let mut cache: HashMap<(usize, String), Vec<Offer>> = HashMap::new();
    let mut out = Vec::new();
    for l in lines {
        let need = l.refs.len() as u32 * opts.boards.max(1);
        let mut notes = Vec::new();
        let offers: Vec<Offer> = match &l.mpn {
            Some(m) => {
                by_source.iter().flatten().filter(|o| same_mpn(&o.mpn, m)).cloned().collect()
            }
            None => Vec::new(),
        };
        if l.mpn.is_some() && offers.is_empty() {
            notes.push(if working.is_empty() {
                "not looked up, every distributor call failed".into()
            } else {
                format!("not found at {}", working.join(" or "))
            });
        }
        for o in &offers {
            if !o.active() {
                notes.push(format!(
                    "{} lists it as {}",
                    o.distributor,
                    o.lifecycle.as_deref().unwrap_or("inactive")
                ));
            }
        }
        let chosen = cheapest_in_stock(&offers, need);
        if chosen.is_none() && !offers.is_empty() {
            notes.push(format!("no distributor has {need} in stock"));
        }
        let priced: Vec<Priced> =
            offers.iter().filter_map(|o| o.cost(need).map(|c| Priced::new(o, c))).collect();
        let spec = spec::classify(
            &l.refs[0],
            &l.value,
            &l.footprint,
            chosen
                .as_ref()
                .map(|c| c.0)
                .filter(|o| !o.attributes.is_empty())
                .or_else(|| offers.iter().max_by_key(|o| o.attributes.len())),
        );
        let mut alternatives = Vec::new();
        if opts.alternatives {
            let baseline = chosen.as_ref().map(|(o, c)| (o.currency.clone(), c.total));
            let mut candidates: Vec<(Offer, String)> = Vec::new();
            if let Some(k) = spec::keyword(&spec) {
                for (i, s) in sources.iter().enumerate() {
                    if failed.contains(&s.name()) {
                        continue;
                    }
                    let found = match cache.get(&(i, k.clone())) {
                        Some(f) => f.clone(),
                        None => {
                            let f = s.search(&k).unwrap_or_else(|e| {
                                errors.push(e);
                                Vec::new()
                            });
                            cache.insert((i, k.clone()), f.clone());
                            f
                        }
                    };
                    candidates.extend(
                        found
                            .into_iter()
                            .filter(|o| spec::matches(&spec, o))
                            .map(|o| (o, "same value, package and ratings".to_string())),
                    );
                }
            }
            for o in &offers {
                if let Some(r) = &o.replacement {
                    let src = sources.iter().find(|s| s.name() == o.distributor);
                    if let Some(s) = src
                        && let Ok(found) = s.by_mpn(std::slice::from_ref(r))
                    {
                        candidates.extend(found.into_iter().map(|f| {
                            (f, format!("{} suggests it as the replacement", o.distributor))
                        }));
                    }
                }
            }
            if let Some((c, _)) = &chosen {
                for o in &offers {
                    if o.distributor != c.distributor {
                        candidates.push((o.clone(), "same part at another distributor".into()));
                    }
                }
            }
            let mut seen = std::collections::HashSet::new();
            let mut alts: Vec<Alternative> = candidates
                .into_iter()
                .filter(|(o, _)| o.active())
                .filter_map(|(o, why)| {
                    let c = o.cost(need)?;
                    if o.stock < c.qty as u64 {
                        return None;
                    }
                    if chosen
                        .as_ref()
                        .is_some_and(|(ch, _)| ch.sku == o.sku && ch.distributor == o.distributor)
                    {
                        return None;
                    }
                    if !seen.insert((o.distributor.clone(), o.sku.clone())) {
                        return None;
                    }
                    let saves = baseline
                        .as_ref()
                        .filter(|(cur, _)| *cur == o.currency)
                        .map(|(_, total)| total - c.total);
                    if saves.is_some_and(|s| s <= 1e-9) {
                        return None;
                    }
                    Some(Alternative { part: Priced::new(&o, c), saves, why })
                })
                .collect();
            alts.sort_by(|a, b| {
                b.saves
                    .unwrap_or(0.0)
                    .total_cmp(&a.saves.unwrap_or(0.0))
                    .then(a.part.total.total_cmp(&b.part.total))
            });
            alts.truncate(opts.max_alternatives);
            alternatives = alts;
        }
        out.push(Line {
            bom: l.clone(),
            need,
            spec,
            chosen: chosen.map(|(o, c)| Priced::new(o, c)),
            offers: priced,
            alternatives,
            notes,
        });
    }
    let mut totals: BTreeMap<String, Total> = BTreeMap::new();
    for l in &out {
        if let Some(c) = &l.chosen {
            let t = totals.entry(c.currency.clone()).or_default();
            t.chosen += c.total;
            let best = l
                .alternatives
                .iter()
                .filter(|a| a.part.currency == c.currency)
                .map(|a| a.part.total)
                .fold(c.total, f64::min);
            t.cheapest += best;
            t.lines_priced += 1;
        }
    }
    let calls = sources.iter().map(|s| (s.name().to_string(), s.calls())).collect();
    errors.dedup();
    Report { boards: opts.boards, lines: out, totals, calls, errors }
}

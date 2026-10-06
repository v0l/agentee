use crate::{BomLine, Distributor, ref_key, same_mpn, spec};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub refs: Vec<String>,
    pub value: String,
    pub manufacturer: Option<String>,
    pub mpn: Option<String>,
    pub per_board: u32,
    pub qty: u32,
    pub lcsc: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pick {
    pub sku: String,
    pub manufacturer: String,
    pub buy: u32,
    pub unit: f64,
    pub total: f64,
    pub currency: String,
    pub stock: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ordered {
    #[serde(flatten)]
    pub item: Item,
    pub source: Option<String>,
    pub stock: BTreeMap<String, Option<u64>>,
    pub picks: BTreeMap<String, Pick>,
}

fn with_allowance(line: &BomLine, need: u32, spares: bool) -> u32 {
    if !spares {
        return need;
    }
    if let Some(s) = line.spares {
        return need + s;
    }
    let small_passive = matches!(spec::chip_size(&line.footprint), Some("0201" | "0402" | "0603"))
        && (line.footprint.starts_with("R_") || line.footprint.starts_with("C_"));
    if small_passive {
        return (need + 5).div_ceil(10) * 10;
    }
    let prefix: String = line.refs[0].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if matches!(prefix.as_str(), "D" | "Q" | "U" | "F") { need + 1 } else { need }
}

pub fn items(lines: &[BomLine], boards: u32, spares: bool) -> Vec<Item> {
    let boards = boards.max(1);
    let mut out: Vec<Item> = lines
        .iter()
        .map(|l| {
            let per_board = l.refs.len() as u32;
            Item {
                refs: l.refs.clone(),
                value: l.value.clone(),
                manufacturer: l.manufacturer.clone(),
                mpn: l.mpn.clone(),
                per_board,
                qty: with_allowance(l, per_board * boards, spares),
                lcsc: l.lcsc.clone(),
            }
        })
        .collect();
    let mut extras: Vec<Item> = Vec::new();
    for l in lines {
        for (mpn, count) in &l.buy_with {
            let item = match extras.iter_mut().find(|e| e.mpn.as_deref() == Some(mpn.as_str())) {
                Some(e) => e,
                None => {
                    extras.push(Item {
                        refs: Vec::new(),
                        value: String::new(),
                        manufacturer: None,
                        mpn: Some(mpn.clone()),
                        per_board: 0,
                        qty: 0,
                        lcsc: None,
                    });
                    extras.last_mut().unwrap()
                }
            };
            item.refs.extend(l.refs.iter().cloned());
            item.per_board += count * l.refs.len() as u32;
        }
    }
    for e in &mut extras {
        e.refs.sort_by_key(|r| ref_key(r));
        e.value = format!("for {}", e.refs.join(" "));
        e.qty = e.per_board * boards + u32::from(spares);
    }
    out.extend(extras);
    out
}

pub fn plan(items: Vec<Item>, sources: &[&dyn Distributor]) -> (Vec<Ordered>, Vec<String>) {
    let mut mpns: Vec<String> = items.iter().filter_map(|i| i.mpn.clone()).collect();
    mpns.sort();
    mpns.dedup();
    let mut errors = Vec::new();
    let mut offers = Vec::new();
    for s in sources {
        match s.by_mpn(&mpns) {
            Ok(o) => offers.extend(o),
            Err(e) => errors.push(e),
        }
    }
    let rows = items
        .into_iter()
        .map(|item| {
            let mut stock = BTreeMap::new();
            let mut picks = BTreeMap::new();
            for s in sources {
                let site = s.name().to_string();
                let found: Vec<_> = offers
                    .iter()
                    .filter(|o| o.distributor == site)
                    .filter(|o| item.mpn.as_deref().is_some_and(|m| same_mpn(&o.mpn, m)))
                    .collect();
                stock.insert(site.clone(), found.iter().map(|o| o.stock).max());
                let best = found
                    .iter()
                    .filter(|o| o.active())
                    .filter_map(|o| o.cost(item.qty).map(|c| (o, c)))
                    .filter(|(o, c)| o.stock >= c.qty as u64)
                    .min_by(|a, b| a.1.total.total_cmp(&b.1.total));
                if let Some((o, c)) = best {
                    picks.insert(
                        site,
                        Pick {
                            sku: o.sku.clone(),
                            manufacturer: o.manufacturer.clone(),
                            buy: c.qty,
                            unit: c.unit,
                            total: c.total,
                            currency: o.currency.clone(),
                            stock: o.stock,
                        },
                    );
                }
            }
            let source = picks
                .iter()
                .min_by(|a, b| a.1.total.total_cmp(&b.1.total))
                .map(|(site, _)| site.clone());
            Ordered { item, source, stock, picks }
        })
        .collect();
    (rows, errors)
}

impl Ordered {
    pub fn elsewhere(&self) -> String {
        match (&self.item.lcsc, &self.item.mpn) {
            (Some(c), _) => format!("LCSC {c}"),
            (None, None) => "no mpn".into(),
            (None, Some(_)) => "no stock".into(),
        }
    }

    pub fn status(&self, site: &str) -> String {
        if self.source.as_deref() == Some(site) {
            return "order".into();
        }
        if self.picks.contains_key(site) {
            return format!("buying at {}", self.source.as_deref().unwrap_or(""));
        }
        match self.stock.get(site).copied().flatten() {
            Some(n) => format!("short, {n} in stock"),
            None => "not listed".into(),
        }
    }

    fn band(&self, site: &str) -> u8 {
        if self.source.as_deref() == Some(site) {
            0
        } else if self.picks.contains_key(site) {
            1
        } else {
            2
        }
    }

    fn manufacturer(&self) -> String {
        self.item
            .manufacturer
            .clone()
            .or_else(|| self.picks.values().next().map(|p| p.manufacturer.clone()))
            .unwrap_or_default()
    }

    fn note(&self, site: &str) -> String {
        let refs = self.item.refs.join(" ");
        if self.item.value == format!("for {refs}") {
            return format!("{}: {}", self.status(site), self.item.value);
        }
        let what = [self.item.value.as_str(), &refs]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        format!("{}: {what}", self.status(site))
    }
}

fn cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn row(cells: &[String]) -> String {
    let mut line = cells.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",");
    line.push('\n');
    line
}

const MOUSER_COLUMNS: [&str; 28] = [
    "Mfr Part Number (Input)",
    "Manufacturer Part Number",
    "Mouser Part Number",
    "Manufacturer Name",
    "Description",
    "Quantity 1",
    "Unit Price 1",
    "Quantity 2",
    "Unit Price 2",
    "Quantity 3",
    "Unit Price 3",
    "Quantity 4",
    "Unit Price 4",
    "Quantity 5",
    "Unit Price 5",
    "Order Quantity",
    "Order Unit Price",
    "Min./Mult.",
    "Availability",
    "Lead Time in Days",
    "Lifecycle",
    "NCNR",
    "RoHS",
    "Pb Free",
    "Package Type",
    "Datasheet URL",
    "Product Image",
    "Design Risk",
];

pub fn sorted_for<'a>(rows: &'a [Ordered], site: &str) -> Vec<&'a Ordered> {
    let mut v: Vec<&Ordered> = rows.iter().collect();
    v.sort_by_key(|r| (r.band(site), r.item.refs.first().map(|f| ref_key(f))));
    v
}

pub fn site_sheet(rows: &[Ordered], site: &str) -> String {
    let mouser = site.eq_ignore_ascii_case("mouser");
    let mut out = if mouser {
        row(&MOUSER_COLUMNS.map(String::from))
    } else {
        row(&[
            "Order Code",
            "Quantity",
            "Line Note",
            "Manufacturer",
            "Manufacturer Part Number",
            "Status",
            "Stock",
            "Unit Price",
            "Total",
        ]
        .map(String::from))
    };
    for r in sorted_for(rows, site) {
        let pick = r.picks.get(site);
        let qty = pick.map(|p| p.buy).unwrap_or(r.item.qty).to_string();
        let sku = pick.map(|p| p.sku.clone()).unwrap_or_default();
        let mpn = r.item.mpn.clone().unwrap_or_default();
        let stock = r.stock.get(site).copied().flatten().map(|n| n.to_string()).unwrap_or_default();
        out.push_str(&row(&if mouser {
            let mut cells = vec![String::new(); MOUSER_COLUMNS.len()];
            cells[0] = mpn.clone();
            cells[1] = mpn;
            cells[2] = sku;
            cells[3] = r.manufacturer();
            cells[4] = r.note(site);
            cells[5] = qty.clone();
            cells[15] = qty;
            cells[18] = stock;
            cells
        } else {
            vec![
                sku,
                qty,
                r.note(site),
                r.manufacturer(),
                mpn,
                r.status(site),
                stock,
                pick.map(|p| format!("{:.4}", p.unit)).unwrap_or_default(),
                pick.map(|p| format!("{:.2}", p.total)).unwrap_or_default(),
            ]
        }));
    }
    out
}

pub fn order_sheet(rows: &[Ordered]) -> String {
    let mut out = row(&[
        "Manufacturer",
        "Manufacturer Part Number",
        "Quantity",
        "Per Board",
        "Designators",
        "Value",
        "Source",
        "Total",
        "Currency",
    ]
    .map(String::from));
    let mut v: Vec<&Ordered> = rows.iter().collect();
    v.sort_by_key(|r| r.item.refs.first().map(|f| ref_key(f)));
    for r in v {
        let pick = r.source.as_ref().and_then(|s| r.picks.get(s));
        out.push_str(&row(&[
            r.manufacturer(),
            r.item.mpn.clone().unwrap_or_default(),
            pick.map(|p| p.buy).unwrap_or(r.item.qty).to_string(),
            r.item.per_board.to_string(),
            r.item.refs.join(" "),
            r.item.value.clone(),
            r.source.clone().unwrap_or_else(|| r.elsewhere()),
            pick.map(|p| format!("{:.2}", p.total)).unwrap_or_default(),
            pick.map(|p| p.currency.clone()).unwrap_or_default(),
        ]));
    }
    out
}

use crate::Report;
use serde::Serialize;

#[derive(Serialize)]
pub struct BomCost {
    pub name: String,
    pub boards: Vec<u32>,
    pub currency: String,
    pub rows: Vec<BomRow>,
    pub totals: Vec<BomTotal>,
    pub calls: std::collections::BTreeMap<String, u32>,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct BomRow {
    pub refs: Vec<String>,
    pub value: String,
    pub mpn: Option<String>,
    pub per_board: u32,
    pub costs: Vec<Option<BomBuy>>,
    pub notes: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct BomBuy {
    pub distributor: String,
    pub sku: String,
    pub buy: u32,
    pub unit: f64,
    pub total: f64,
    pub currency: String,
}

#[derive(Serialize)]
pub struct BomTotal {
    pub boards: u32,
    pub currency: String,
    pub total: f64,
    pub per_board: f64,
    pub priced: usize,
    pub unpriced: usize,
}

pub fn counts(boards: &[u32]) -> Vec<u32> {
    let mut boards: Vec<u32> = boards.iter().copied().filter(|&b| b > 0).collect();
    boards.sort_unstable();
    boards.dedup();
    if boards.is_empty() {
        boards.push(1);
    }
    boards
}

pub fn of(name: &str, boards: &[u32], r: &Report) -> BomCost {
    let boards = counts(boards);
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for o in r.lines.iter().flat_map(|l| &l.offers) {
        *seen.entry(o.currency.clone()).or_default() += 1;
    }
    let currency = seen.iter().max_by_key(|(_, n)| **n).map(|(c, _)| c.clone()).unwrap_or_default();
    let mut rows = Vec::new();
    for l in &r.lines {
        let per_board = l.bom.refs.len() as u32;
        let costs: Vec<Option<BomBuy>> = boards
            .iter()
            .map(|&b| {
                let need = per_board * b;
                let best = |same: bool| {
                    l.offers
                        .iter()
                        .filter(|o| (o.currency == currency) == same)
                        .filter_map(|o| o.at(need).map(|c| (o, c)))
                        .min_by(|a, b| a.1.total.total_cmp(&b.1.total))
                };
                best(true).or_else(|| best(false)).map(|(o, c)| BomBuy {
                    distributor: o.distributor.clone(),
                    sku: o.sku.clone(),
                    buy: c.qty,
                    unit: c.unit,
                    total: c.total,
                    currency: o.currency.clone(),
                })
            })
            .collect();
        let mut notes: Vec<String> =
            l.notes.iter().filter(|n| !n.starts_with("no distributor has")).cloned().collect();
        if l.bom.mpn.is_none() {
            notes.push("no mpn".into());
        }
        if let Some((&b, _)) =
            boards.iter().zip(&costs).find(|(_, c)| c.is_none()).filter(|_| !l.offers.is_empty())
        {
            let n = per_board * b;
            notes.push(format!("no distributor has {n} in stock"));
        }
        rows.push(BomRow {
            refs: l.bom.refs.clone(),
            value: l.bom.value.clone(),
            mpn: l.bom.mpn.clone(),
            per_board,
            costs,
            notes,
        });
    }
    let mut totals = Vec::new();
    for (k, &b) in boards.iter().enumerate() {
        let mut by: std::collections::BTreeMap<String, (f64, usize)> = Default::default();
        let mut unpriced = 0;
        for row in &rows {
            match &row.costs[k] {
                Some(c) => {
                    let e = by.entry(c.currency.clone()).or_default();
                    e.0 += c.total;
                    e.1 += 1;
                }
                None => unpriced += 1,
            }
        }
        if by.is_empty() {
            by.insert(currency.clone(), (0.0, 0));
        }
        for (cur, (total, priced)) in by {
            totals.push(BomTotal {
                boards: b,
                per_board: total / b as f64,
                currency: cur,
                total,
                priced,
                unpriced,
            });
        }
    }
    BomCost {
        name: name.to_string(),
        boards,
        currency,
        rows,
        totals,
        calls: r.calls.clone(),
        errors: r.errors.clone(),
    }
}

pub fn text(c: &BomCost) -> String {
    use std::fmt::Write as _;
    let refs = |v: &[String]| {
        if v.len() > 3 {
            format!("{} .. {} ({})", v[0], v[v.len() - 1], v.len())
        } else {
            v.join(" ")
        }
    };
    let money = |v: f64| format!("{v:.2}");
    let heads: Vec<String> = c
        .boards
        .iter()
        .map(|b| if *b == 1 { "1 board".to_string() } else { format!("{b} boards") })
        .collect();
    let mut table: Vec<Vec<String>> = vec![
        ["refs", "qty", "value", "mpn", "from", "sku"]
            .iter()
            .map(|s| s.to_string())
            .chain(heads.iter().cloned())
            .chain(["note".to_string()])
            .collect(),
    ];
    let mut switched = false;
    for r in &c.rows {
        let first = r.costs.first().cloned().flatten();
        let mut row = vec![
            refs(&r.refs),
            r.per_board.to_string(),
            r.value.clone(),
            r.mpn.clone().unwrap_or_else(|| "-".into()),
            first.as_ref().map(|b| b.distributor.clone()).unwrap_or_else(|| "-".into()),
            first.as_ref().map(|b| b.sku.clone()).unwrap_or_else(|| "-".into()),
        ];
        for b in &r.costs {
            row.push(match b {
                Some(b) => {
                    let other = first
                        .as_ref()
                        .is_some_and(|f| f.distributor != b.distributor || f.sku != b.sku);
                    let foreign = b.currency != c.currency;
                    switched |= other;
                    format!(
                        "{}{}{}",
                        money(b.total),
                        if foreign { format!(" {}", b.currency) } else { String::new() },
                        if other { "*" } else { "" }
                    )
                }
                None => "-".into(),
            });
        }
        row.push(r.notes.join("; "));
        table.push(row);
    }
    let blank = |label: &str| {
        let mut v = vec![label.to_string()];
        v.extend(std::iter::repeat_n(String::new(), 5));
        v
    };
    let mut currencies: Vec<&String> = c.totals.iter().map(|t| &t.currency).collect();
    currencies.sort();
    currencies.dedup();
    let mut foot: Vec<Vec<String>> = Vec::new();
    for cur in currencies {
        let at = |b: u32| c.totals.iter().find(|t| t.boards == b && &t.currency == cur);
        let mut total = blank(&format!("total {cur}"));
        let mut each = blank(&format!("per board {cur}"));
        for &b in &c.boards {
            total.push(at(b).map(|t| money(t.total)).unwrap_or_else(|| "-".into()));
            each.push(at(b).map(|t| money(t.per_board)).unwrap_or_else(|| "-".into()));
        }
        foot.push(total);
        foot.push(each);
    }
    let unpriced = c.totals.first().map(|t| t.unpriced).unwrap_or(0);
    if unpriced > 0 {
        let mut row = blank("unpriced lines");
        for &b in &c.boards {
            let n = c.totals.iter().find(|t| t.boards == b).map(|t| t.unpriced).unwrap_or(0);
            row.push(n.to_string());
        }
        foot.push(row);
    }
    let cols = table[0].len();
    let width: Vec<usize> = (0..cols - 1)
        .map(|i| {
            table
                .iter()
                .chain(&foot)
                .filter_map(|r| r.get(i))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let numeric = |i: usize| i == 1 || (6..cols - 1).contains(&i);
    let line = |r: &[String], out: &mut String| {
        let mut s = String::new();
        for (i, cell) in r.iter().enumerate() {
            if i == cols - 1 {
                if !cell.is_empty() {
                    s += "  ";
                    s += cell;
                }
                continue;
            }
            let w = width[i];
            if i > 0 {
                s += "  ";
            }
            if numeric(i) {
                let _ = write!(s, "{cell:>w$}");
            } else {
                let _ = write!(s, "{cell:<w$}");
            }
        }
        out.push_str(s.trim_end());
        out.push('\n');
    };
    let mut out = String::new();
    let calls: Vec<String> = c.calls.iter().map(|(k, v)| format!("{k} {v}")).collect();
    let _ = writeln!(
        out,
        "BOM cost of {}, line totals in {} at each board count, API calls: {}\n",
        c.name,
        if c.currency.is_empty() { "-" } else { &c.currency },
        calls.join(", ")
    );
    for r in &table {
        line(r, &mut out);
    }
    let rule: usize = width.iter().sum::<usize>() + 2 * (cols - 2);
    out += &"-".repeat(rule);
    out.push('\n');
    for r in &foot {
        let mut r = r.clone();
        r.push(String::new());
        line(&r, &mut out);
    }
    if switched {
        out += "\n* bought from another distributor or SKU at that quantity\n";
    }
    for e in &c.errors {
        let _ = writeln!(out, "error: {e}");
    }
    out
}

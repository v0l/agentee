use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Asset {
    Symbol,
    Footprint,
    Model,
    Datasheet,
}

impl Asset {
    pub const ALL: [Asset; 4] = [Asset::Symbol, Asset::Footprint, Asset::Model, Asset::Datasheet];

    pub fn name(self) -> &'static str {
        match self {
            Asset::Symbol => "symbol",
            Asset::Footprint => "footprint",
            Asset::Model => "model",
            Asset::Datasheet => "datasheet",
        }
    }

    pub fn parse(s: &str) -> Option<Asset> {
        Asset::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub source: String,
    pub asset: Asset,
    pub id: String,
    pub title: String,
    pub detail: String,
    pub score: u32,
}

pub trait AssetSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn assets(&self) -> &'static [Asset];
    fn search(&self, asset: Asset, query: &str, limit: usize) -> Result<Vec<Hit>, String>;
}

pub fn terms(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(str::to_string).collect()
}

pub fn score(terms: &[String], fields: &[(&str, u32)]) -> Option<u32> {
    let lowered: Vec<(String, u32)> = fields.iter().map(|(f, w)| (f.to_lowercase(), *w)).collect();
    let mut total = 0;
    for t in terms {
        let best = lowered.iter().filter(|(f, _)| f.contains(t.as_str())).map(|(_, w)| *w).max()?;
        total += best;
    }
    Some(total)
}

pub fn rank(mut hits: Vec<Hit>, limit: usize) -> Vec<Hit> {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.len().cmp(&b.title.len())));
    hits.truncate(limit);
    hits
}

#[derive(Default, Serialize)]
pub struct Found {
    pub hits: Vec<Hit>,
    pub errors: Vec<String>,
}

pub fn search_all(
    sources: &[Box<dyn AssetSource>],
    asset: Asset,
    query: &str,
    limit: usize,
) -> Found {
    let mut found = Found::default();
    if query.trim().is_empty() {
        return found;
    }
    let results: Vec<(&'static str, Result<Vec<Hit>, String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = sources
            .iter()
            .filter(|src| src.assets().contains(&asset))
            .map(|src| (src.name(), s.spawn(move || src.search(asset, query, limit))))
            .collect();
        handles
            .into_iter()
            .map(|(n, h)| (n, h.join().unwrap_or_else(|_| Err("search panicked".into()))))
            .collect()
    });
    for (name, r) in results {
        match r {
            Ok(h) => found.hits.extend(h),
            Err(e) => found.errors.push(format!("{name}: {e}")),
        }
    }
    found.hits = rank(found.hits, limit);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, Asset, u32);

    impl AssetSource for Fixed {
        fn name(&self) -> &'static str {
            self.0
        }
        fn assets(&self) -> &'static [Asset] {
            match self.1 {
                Asset::Symbol => &[Asset::Symbol],
                _ => &[Asset::Model],
            }
        }
        fn search(&self, asset: Asset, q: &str, _: usize) -> Result<Vec<Hit>, String> {
            Ok(vec![Hit {
                source: self.0.into(),
                asset,
                id: q.into(),
                title: q.into(),
                detail: String::new(),
                score: self.2,
            }])
        }
    }

    #[test]
    fn every_source_for_the_asset_is_asked_and_merged_by_score() {
        let all: Vec<Box<dyn AssetSource>> = vec![
            Box::new(Fixed("a", Asset::Symbol, 1)),
            Box::new(Fixed("b", Asset::Symbol, 9)),
            Box::new(Fixed("c", Asset::Model, 5)),
        ];
        let f = search_all(&all, Asset::Symbol, "zener", 10);
        let order: Vec<&str> = f.hits.iter().map(|h| h.source.as_str()).collect();
        assert_eq!(order, ["b", "a"]);
        assert!(search_all(&all, Asset::Symbol, "  ", 10).hits.is_empty());
    }

    #[test]
    fn every_term_must_hit_some_field() {
        let t = terms("zener 5v1");
        assert_eq!(score(&t, &[("Zener diode 5V1", 2)]), Some(4));
        assert_eq!(score(&t, &[("Zener diode", 2)]), None);
    }
}

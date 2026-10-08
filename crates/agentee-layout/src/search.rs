use crate::{Run, RunReport, load};
use agentee_core::engine::{EngineFile, SearchFile, knob_path, stage_of};
use agentee_core::project::LayoutInputs;
use serde::Serialize;
use std::collections::BTreeMap;

pub const DEFAULT_TRIES: usize = 16;
pub const DEFAULT_KEEP: usize = 4;
const DEFAULT_SCREEN: &str = "global";

#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub tries: Option<usize>,
    pub keep: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Tried {
    pub stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    pub knobs: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unrouted: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copper_overlap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courtyard: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub kept: bool,
    pub ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchReport {
    pub tried: Vec<Tried>,
    pub best: usize,
    pub width: usize,
    pub ms: f64,
    #[serde(skip)]
    pub run: RunReport,
}

impl SearchReport {
    pub fn table(&self) -> String {
        let mut keys: Vec<&String> = self.tried.iter().flat_map(|t| t.knobs.keys()).collect();
        keys.sort();
        keys.dedup();
        let widths: Vec<usize> = keys
            .iter()
            .map(|k| {
                let longest = self.tried.iter().filter_map(|c| c.knobs.get(*k)).map(String::len);
                longest.chain([k.len()]).max().unwrap_or(0) + 2
            })
            .collect();
        let mut t = format!("{:<5}{:<12}{:>7}  ", "node", "stage", "from");
        for (k, w) in keys.iter().zip(&widths) {
            t += &format!("{k:<w$}");
        }
        t += &format!("{:>10}{:>9}{:>12}{:>8}\n", "unrouted", "overlap", "total", "s");
        let n = |v: Option<f64>, w: usize, p: usize| match v {
            Some(v) => format!("{v:>w$.p$}"),
            None => format!("{:>w$}", "-"),
        };
        let mut stages: Vec<&str> = Vec::new();
        for c in &self.tried {
            if !stages.contains(&c.stage.as_str()) {
                stages.push(&c.stage);
            }
        }
        for stage in stages {
            let mut order: Vec<usize> =
                (0..self.tried.len()).filter(|&i| self.tried[i].stage == stage).collect();
            order.sort_by(|&a, &b| rank(&self.tried[a]).total_cmp_key(&rank(&self.tried[b])));
            for i in order {
                let c = &self.tried[i];
                t += &format!(
                    "{:<5}{:<12}{:>7}  ",
                    i,
                    c.stage,
                    c.parent.map(|p| p.to_string()).unwrap_or_else(|| "-".into())
                );
                for (k, w) in keys.iter().zip(&widths) {
                    t += &format!("{:<w$}", c.knobs.get(*k).map(String::as_str).unwrap_or("-"));
                }
                t += &n(c.unrouted, 10, 0);
                t += &n(c.copper_overlap, 9, 0);
                t += &n(c.total.or(c.screen), 12, 1);
                t += &format!("{:>8.1}", c.ms / 1000.0);
                if i == self.best {
                    t += "  best";
                } else if c.kept {
                    t += "  kept";
                }
                if let Some(e) = &c.error {
                    t += &format!("  {e}");
                }
                t.push('\n');
            }
        }
        t
    }
}

struct Key(Vec<f64>);

impl Key {
    fn total_cmp_key(&self, o: &Key) -> std::cmp::Ordering {
        for (a, b) in self.0.iter().zip(&o.0) {
            match a.total_cmp(b) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }
}

fn rank(t: &Tried) -> Key {
    let or_max = |v: Option<f64>| v.unwrap_or(f64::MAX);
    match t.total {
        Some(total) => Key(vec![
            0.0,
            t.courtyard.unwrap_or(0.0),
            or_max(t.copper_overlap),
            or_max(t.unrouted),
            total,
        ]),
        None => Key(vec![1.0, t.courtyard.unwrap_or(0.0), 0.0, 0.0, or_max(t.screen)]),
    }
}

fn set_knob(text: &str, key: &str, value: &toml::Value) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut path = vec!["engine".to_string()];
    path.extend(knob_path(key));
    let (last, parents) = path.split_last().ok_or("an empty knob")?;
    let mut node = doc.as_table_mut();
    for k in parents {
        if !node.get(k).is_some_and(|v| v.is_table_like()) {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            node.insert(k, toml_edit::Item::Table(t));
        }
        node = match node.get_mut(k).and_then(|v| v.as_table_mut()) {
            Some(t) => t,
            None => return Err(format!("`{key}` runs through an inline table")),
        };
    }
    node.insert(last, toml_edit::value(edit_value(value)?));
    Ok(doc.to_string())
}

fn tuned(
    engine: &EngineFile,
    knobs: &[(String, Vec<toml::Value>)],
    c: &[toml::Value],
) -> Result<EngineFile, String> {
    let mut v = toml::Value::try_from(engine).map_err(|e| e.to_string())?;
    for ((key, _), value) in knobs.iter().zip(c) {
        let path = knob_path(key);
        let (last, parents) = path.split_last().ok_or("an empty knob")?;
        let mut node = &mut v;
        for k in parents {
            let table =
                node.as_table_mut().ok_or_else(|| format!("`{key}` runs through a value"))?;
            node = table.entry(k.clone()).or_insert_with(|| toml::Value::Table(Default::default()));
        }
        node.as_table_mut()
            .ok_or_else(|| format!("`{key}` runs through a value"))?
            .insert(last.clone(), value.clone());
    }
    v.try_into().map_err(|e: toml::de::Error| format!("engine settings: {}", e.message()))
}

fn edit_value(v: &toml::Value) -> Result<toml_edit::Value, String> {
    Ok(match v {
        toml::Value::String(s) => s.as_str().into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(f) => (*f).into(),
        toml::Value::Boolean(b) => (*b).into(),
        other => {
            let doc: toml_edit::DocumentMut =
                format!("v = {other}").parse().map_err(|e| format!("{e}"))?;
            doc.get("v").and_then(|i| i.as_value()).cloned().ok_or("not a value")?
        }
    })
}

fn current(engine: &EngineFile, key: &str) -> Option<toml::Value> {
    let mut v = toml::Value::try_from(engine).ok()?;
    for k in knob_path(key) {
        v = v.get(&k)?.clone();
    }
    Some(v)
}

fn jitter(seed: u64, i: u64) -> f64 {
    let mut z = seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(i.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn candidates(
    knobs: &[(String, Vec<toml::Value>)],
    base: Vec<toml::Value>,
    tries: usize,
    seed: u64,
) -> Vec<Vec<toml::Value>> {
    let mut out = vec![base];
    let push = |c: Vec<toml::Value>, out: &mut Vec<Vec<toml::Value>>| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let combos: usize = knobs.iter().map(|(_, v)| v.len()).product();
    if combos <= tries {
        for mut i in 0..combos {
            let mut c = Vec::with_capacity(knobs.len());
            for (_, pool) in knobs {
                c.push(pool[i % pool.len()].clone());
                i /= pool.len();
            }
            push(c, &mut out);
        }
        return out;
    }
    let mut i = 0u64;
    while out.len() < tries && i < tries as u64 * 50 {
        let c = knobs
            .iter()
            .enumerate()
            .map(|(ki, (_, pool))| {
                let r = jitter(seed, i.wrapping_mul(31).wrapping_add(ki as u64));
                pool[((r * pool.len() as f64) as usize).min(pool.len() - 1)].clone()
            })
            .collect();
        push(c, &mut out);
        i += 1;
    }
    out
}

fn shown(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

struct Node<'l> {
    branch: crate::Branch<'l>,
    engine: EngineFile,
    values: Vec<toml::Value>,
    tried: usize,
    own: bool,
}

fn measure(stage: &str, b: &crate::Branch, tried: &mut Tried) {
    let score = b.score();
    let raw = |k: &str| score.terms.get(k).filter(|t| t.measured).map(|t| t.raw);
    tried.courtyard = raw("overlap");
    if matches!(stage, "detail" | "finish") {
        tried.unrouted = raw("unrouted");
        tried.copper_overlap = raw("copper_overlap");
        tried.total = Some(score.total);
    } else {
        tried.screen = Some(score.total);
    }
}

pub fn search(
    inputs: &LayoutInputs,
    text: &str,
    a: &Run,
    ask: &Ask,
) -> Result<SearchReport, String> {
    if a.only.is_some() {
        return Err("search runs a range of stages, not --only".into());
    }
    let started = std::time::Instant::now();
    let loaded = load(inputs, text)?;
    let engine = loaded.file.engine.clone().unwrap_or_default();
    let sf: SearchFile = engine.search.clone().unwrap_or_default();
    let wanted = engine.phases();
    let pos = |n: &str| -> Result<usize, String> {
        let s = stage_of(n).ok_or_else(|| format!("no stage `{n}`"))?;
        wanted
            .iter()
            .position(|w| w == s)
            .ok_or_else(|| format!("stage `{n}` is not in the configured stages"))
    };
    let first = a.from.as_deref().map(pos).transpose()?.unwrap_or(0);
    let last = a.to.as_deref().map(pos).transpose()?.unwrap_or(wanted.len().saturating_sub(1));
    let stages: Vec<String> = wanted.get(first..=last).map(<[String]>::to_vec).unwrap_or_default();
    let tries = ask.tries.or(sf.tries).unwrap_or(DEFAULT_TRIES).max(1);
    let width = ask.keep.or(sf.keep).unwrap_or(DEFAULT_KEEP).max(1);
    let mut knobs: Vec<(String, Vec<toml::Value>)> =
        sf.knobs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    if knobs.is_empty() {
        if !stages.iter().any(|s| s == "place") {
            return Err(
                "with no [engine.search] knobs the search varies the placement seed, so the range has to include place; name knobs to search the later stages"
                    .into(),
            );
        }
        let seeds = (1..=tries as i64).map(toml::Value::Integer).collect();
        knobs.push(("place.seed".into(), seeds));
    }
    for (k, values) in &knobs {
        if values.is_empty() {
            return Err(format!("knob `{k}` lists no values"));
        }
        for v in values {
            agentee_core::engine::knob_fits(k, v)?;
        }
    }
    let mut level: Vec<usize> = Vec::new();
    for (k, _) in &knobs {
        let head = knob_path(k).first().cloned().unwrap_or_default();
        level.push(match stages.iter().position(|s| *s == head) {
            Some(i) => i,
            None if stage_of(&head).is_some() => {
                return Err(format!("knob `{k}` belongs to `{head}`, which is outside the range"));
            }
            None => 0,
        });
    }
    let base: Vec<toml::Value> = knobs
        .iter()
        .map(|(k, pool)| current(&engine, k).unwrap_or_else(|| pool[0].clone()))
        .collect();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let stop = a.stop;
    let stopped = || stop.is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed));
    let screen = sf.screen.as_deref().unwrap_or(DEFAULT_SCREEN);
    let cut_from = stage_of(screen).and_then(|s| stages.iter().position(|x| x == s)).unwrap_or(0);
    let mut tried: Vec<Tried> = Vec::new();
    let root = loaded.root(&stages, &engine)?;
    let mut frontier: Vec<Node> = vec![Node {
        branch: root,
        engine: engine.clone(),
        values: base,
        tried: usize::MAX,
        own: true,
    }];
    for (li, stage) in stages.iter().enumerate() {
        if stopped() {
            return Err("stopped".into());
        }
        let here: Vec<usize> = (0..knobs.len()).filter(|&k| level[k] == li).collect();
        let per = tries.div_ceil(frontier.len()).max(1);
        let mut specs: Vec<(usize, Vec<toml::Value>, EngineFile, bool)> = Vec::new();
        for (pi, parent) in frontier.iter().enumerate() {
            if here.is_empty() {
                specs.push((pi, parent.values.clone(), parent.engine.clone(), parent.own));
                continue;
            }
            let pool: Vec<(String, Vec<toml::Value>)> =
                here.iter().map(|&k| knobs[k].clone()).collect();
            let start: Vec<toml::Value> = here.iter().map(|&k| parent.values[k].clone()).collect();
            let seed = sf.seed.unwrap_or(1).wrapping_add(pi as u64 * 7919 + li as u64);
            for (ci, c) in candidates(&pool, start, per, seed).into_iter().enumerate() {
                let mut values = parent.values.clone();
                for (x, &k) in here.iter().enumerate() {
                    values[k] = c[x].clone();
                }
                let e = tuned(&parent.engine, &pool, &c)?;
                specs.push((pi, values, e, parent.own && ci == 0));
            }
        }
        let heavy = !matches!(stage.as_str(), "constraints" | "place");
        let lanes = if heavy { (cores / 4).max(1) } else { (cores / 2).max(1) };
        let mut children: Vec<(Tried, Option<Node>)> = Vec::new();
        for chunk in specs.chunks(lanes) {
            let done: Vec<(Tried, Option<Node>)> = std::thread::scope(|scope| {
                let hs: Vec<_> = chunk
                    .iter()
                    .map(|(pi, values, e, own)| {
                        let parent = &frontier[*pi];
                        let loaded = &loaded;
                        let knobs = &knobs;
                        scope.spawn(move || {
                            let t0 = std::time::Instant::now();
                            let mut t = Tried {
                                stage: stage.clone(),
                                parent: (parent.tried != usize::MAX).then_some(parent.tried),
                                knobs: knobs
                                    .iter()
                                    .zip(values)
                                    .map(|((k, _), v)| (k.clone(), shown(v)))
                                    .collect(),
                                ..Default::default()
                            };
                            let node = match loaded.advance(&parent.branch, e, stage, stop) {
                                Ok(b) => {
                                    measure(stage, &b, &mut t);
                                    Some(Node {
                                        branch: b,
                                        engine: e.clone(),
                                        values: values.clone(),
                                        tried: 0,
                                        own: *own,
                                    })
                                }
                                Err(err) => {
                                    t.error = Some(err);
                                    None
                                }
                            };
                            t.ms = t0.elapsed().as_secs_f64() * 1e3;
                            (t, node)
                        })
                    })
                    .collect();
                hs.into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            (Tried { error: Some("panicked".into()), ..Default::default() }, None)
                        })
                    })
                    .collect()
            });
            children.extend(done);
        }
        let mut next: Vec<Node> = Vec::new();
        for (t, node) in children {
            tried.push(t);
            if let Some(mut n) = node {
                n.tried = tried.len() - 1;
                next.push(n);
            }
        }
        next.sort_by(|x, y| rank(&tried[x.tried]).total_cmp_key(&rank(&tried[y.tried])));
        if li >= cut_from || li + 1 == stages.len() {
            let own = next.iter().position(|n| n.own).filter(|&k| k >= width);
            let keep_own = own.map(|k| next.swap_remove(k));
            next.truncate(width);
            next.extend(keep_own);
        }
        for n in &next {
            tried[n.tried].kept = true;
        }
        if next.is_empty() {
            let why: Vec<String> = tried.iter().filter_map(|t| t.error.clone()).take(3).collect();
            return Err(format!("every candidate at `{stage}` failed: {}", why.join("; ")));
        }
        frontier = next;
    }
    let best = frontier.swap_remove(0);
    let at = best.tried;
    let mut run = loaded.conclude(best.branch, &best.engine, started)?;
    run.text =
        knobs.iter().zip(&best.values).try_fold(run.text, |t, ((k, _), v)| set_knob(&t, k, v))?;
    Ok(SearchReport { tried, best: at, width, ms: started.elapsed().as_secs_f64() * 1e3, run })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_combination_when_they_fit() {
        let knobs = vec![
            ("place.seed".to_string(), vec![toml::Value::Integer(1), toml::Value::Integer(2)]),
            ("detail.via_cost".to_string(), vec!["1mm".into(), "3mm".into()]),
        ];
        let base = vec![toml::Value::Integer(1), "1mm".into()];
        let c = candidates(&knobs, base.clone(), 16, 1);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0], base);
    }

    #[test]
    fn a_sample_when_they_do_not() {
        let seeds: Vec<toml::Value> = (1..=100).map(toml::Value::Integer).collect();
        let knobs = vec![("place.seed".to_string(), seeds)];
        let c = candidates(&knobs, vec![toml::Value::Integer(1)], 10, 7);
        assert_eq!(c.len(), 10);
        assert_eq!(c, candidates(&knobs, vec![toml::Value::Integer(1)], 10, 7));
    }

    #[test]
    fn knobs_land_under_engine() {
        let t = set_knob(
            "name = \"t\"\n[engine]\nrounds = 2\n",
            "place.seed",
            &toml::Value::Integer(5),
        )
        .unwrap();
        let f: toml::Value = toml::from_str(&t).unwrap();
        assert_eq!(f["engine"]["place"]["seed"].as_integer(), Some(5));
        assert_eq!(f["engine"]["rounds"].as_integer(), Some(2));
        let t = set_knob(&t, "engine.detail.via_cost", &"2mm".into()).unwrap();
        let f: toml::Value = toml::from_str(&t).unwrap();
        assert_eq!(f["engine"]["detail"]["via_cost"].as_str(), Some("2mm"));
    }
}

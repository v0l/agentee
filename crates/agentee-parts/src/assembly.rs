use crate::library::{Entry, Library};
use crate::{BomLine, bom, ref_key};
use agentee_core::footprint::{Footprint, Mount, PadKind};
use agentee_core::height::body_height;
use agentee_core::layout::{Layout, Placed};
use agentee_core::schematic::Schematic;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Group {
    pub step: usize,
    pub side: Side,
    pub value: String,
    pub package: String,
    pub footprint: String,
    pub mpn: Option<String>,
    pub refs: Vec<String>,
    pub tht: bool,
    pub hot: bool,
    pub height: Option<f64>,
    pub library: Option<String>,
    pub bin: Option<String>,
    pub own: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Bottom,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }
}

impl Group {
    pub fn line(&self) -> String {
        let mut bits = vec![format!("{} {}", self.value, self.package).trim().to_string()];
        if let Some(b) = &self.bin {
            bits.push(format!("bin {b}"));
        }
        bits.push(self.refs.join(" "));
        bits.push(format!("{} pcs", self.refs.len()));
        bits.join(", ")
    }
}

pub fn package(footprint: &str) -> String {
    let tokens: Vec<&str> = footprint.split('_').filter(|t| !t.is_empty()).collect();
    let chip = |t: &str| t.len() == 4 && t.chars().all(|c| c.is_ascii_digit());
    if footprint.contains("Metric")
        && let Some(t) = tokens.iter().find(|t| chip(t))
    {
        return t.to_string();
    }
    if let [head @ ("PinHeader" | "PinSocket"), size, ..] = tokens.as_slice() {
        return format!("{head} {size}");
    }
    let skip = |t: &str| {
        let category = ["R", "C", "L", "D", "LED", "CP", "FB", "Q", "SMD", "THT"].contains(&t);
        let word = t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && t.chars().skip(1).any(|c| c.is_ascii_lowercase())
            && t.chars().all(|c| c.is_ascii_alphabetic());
        category || word
    };
    let mut rest = tokens.as_slice();
    while rest.len() > 1 && skip(rest[0]) {
        rest = &rest[1..];
    }
    rest.first().map(|t| t.to_string()).unwrap_or_default()
}

fn smd_pad_areas(fp: &Footprint) -> Vec<f64> {
    fp.pads
        .iter()
        .filter(|p| p.is_copper() && p.kind == PadKind::Smd)
        .map(|p| {
            let [w, h] = p.size.to_mm();
            let ring: Vec<[f64; 2]> = p.points.iter().map(|q| q.to_mm()).collect();
            let shape: f64 = (0..ring.len())
                .map(|i| {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    a[0] * b[1] - b[0] * a[1]
                })
                .sum::<f64>()
                .abs()
                / 2.0;
            (w * h).max(shape)
        })
        .collect()
}

pub fn needs_heat(fp: &Footprint) -> bool {
    let mut areas = smd_pad_areas(fp);
    if areas.len() < 3 {
        return false;
    }
    areas.sort_by(f64::total_cmp);
    let median = areas[areas.len() / 2];
    areas.last().is_some_and(|&big| big >= 3.0 * median && big >= 1.0)
}

fn entry_for<'a>(
    lib: &'a Library,
    line: &BomLine,
    library_field: Option<&str>,
) -> Option<&'a Entry> {
    library_field
        .and_then(|n| lib.get(n))
        .or_else(|| line.mpn.as_deref().and_then(|m| lib.get(m)))
        .or_else(|| {
            let code = line.lcsc.as_deref()?;
            lib.entries.iter().find(|e| e.part.lcsc.as_deref() == Some(code))
        })
}

fn side_order(groups: &[Group]) -> Vec<Side> {
    let count = |s: Side, smd: bool| -> usize {
        groups.iter().filter(|g| g.side == s && (!smd || !g.tht)).map(|g| g.refs.len()).sum()
    };
    let mut sides: Vec<Side> = [Side::Top, Side::Bottom]
        .into_iter()
        .filter(|s| groups.iter().any(|g| g.side == *s))
        .collect();
    sides.sort_by_key(|s| std::cmp::Reverse((count(*s, true), count(*s, false))));
    sides
}

pub fn groups(layout: &Layout, sch: &Schematic, lib: &Library) -> Vec<Group> {
    let placed = |r: &str| layout.parts.iter().find(|p| p.reference == r);
    let field = |r: &str, k: &str| {
        sch.parts.iter().filter(|p| p.reference == r).find_map(|p| p.fields.get(k).cloned())
    };
    let mut out: Vec<Group> = Vec::new();
    for line in bom(sch) {
        let library = field(&line.refs[0], "library");
        let entry = entry_for(lib, &line, library.as_deref());
        for side in [Side::Top, Side::Bottom] {
            let on: Vec<&Placed> = line
                .refs
                .iter()
                .filter_map(|r| placed(r))
                .filter(|p| p.bottom == (side == Side::Bottom))
                .collect();
            let Some(first) = on.first() else { continue };
            let fp = &first.footprint;
            let mut refs: Vec<String> = on.iter().map(|p| p.reference.clone()).collect();
            refs.sort_by_key(|r| ref_key(r));
            out.push(Group {
                step: 0,
                side,
                value: line.value.clone(),
                package: package(&first.footprint_name),
                footprint: first.footprint_name.clone(),
                mpn: line.mpn.clone(),
                refs,
                tht: fp.mount == Mount::Tht,
                hot: needs_heat(fp),
                height: body_height(fp),
                library: entry.map(|e| e.name.clone()),
                bin: entry.and_then(|e| e.bin().map(str::to_string)),
                own: entry.and_then(Entry::self_stock),
            });
        }
    }
    let sides = side_order(&out);
    let side_rank = |s: Side| sides.iter().position(|x| *x == s).unwrap_or(0);
    out.sort_by(|a, b| {
        side_rank(a.side)
            .cmp(&side_rank(b.side))
            .then(a.tht.cmp(&b.tht))
            .then(b.hot.cmp(&a.hot))
            .then(a.height.unwrap_or(0.0).total_cmp(&b.height.unwrap_or(0.0)))
            .then(a.package.cmp(&b.package))
            .then(ref_key(&a.refs[0]).cmp(&ref_key(&b.refs[0])))
    });
    for (i, g) in out.iter_mut().enumerate() {
        g.step = i + 1;
    }
    out
}

pub fn sheet(groups: &[Group]) -> String {
    let mut s = String::new();
    let mut side = None;
    for g in groups {
        if side != Some(g.side) {
            if side.is_some() {
                s.push('\n');
            }
            s += &format!("{} side\n", g.side.name());
            side = Some(g.side);
        }
        s += &format!("{:>3}. {}\n", g.step, g.line());
    }
    s
}

#[derive(Clone, Debug, Serialize)]
pub struct Used {
    pub library: String,
    pub took: u64,
    pub left: u64,
    pub short: u64,
}

pub fn use_up(groups: &[Group], lib: &Library, boards: u32) -> Result<Vec<Used>, String> {
    let mut need: Vec<(String, u64)> = Vec::new();
    for g in groups {
        let Some(name) = &g.library else { continue };
        let n = g.refs.len() as u64 * u64::from(boards);
        match need.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v += n,
            None => need.push((name.clone(), n)),
        }
    }
    let mut out = Vec::new();
    for (name, want) in need {
        let Some(e) = lib.get(&name) else { continue };
        let mut part = e.part.clone();
        let mut rest = want;
        for s in part.stock.iter_mut().filter(|s| s.own > 0) {
            let take = rest.min(s.own);
            s.own -= take;
            rest -= take;
        }
        part.stock.retain(|s| s.own > 0 || s.distributor != "home");
        let left: u64 = part.stock.iter().map(|s| s.own).sum();
        if want - rest > 0 {
            crate::library::write_entry(&e.dir, &part)?;
        }
        out.push(Used { library: name, took: want - rest, left, short: rest });
    }
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct Take {
    pub group: usize,
    pub library: String,
    pub label: String,
    pub pieces: u64,
    pub before: u64,
    pub after: u64,
    pub short: u64,
    pub stale: bool,
}

impl Take {
    pub fn line(&self) -> String {
        let mut s = format!("{}: {} -> {}", self.label, self.before, self.after);
        if self.short > 0 {
            s += &format!(", {} short", self.short);
        }
        if self.stale {
            s += ", count changed since the sheet loaded";
        }
        s
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Shelf {
    pub takes: Vec<Take>,
    pub skipped: Vec<Skipped>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Skipped {
    pub group: usize,
    pub label: String,
}

impl Take {
    pub fn taken(&self) -> u64 {
        self.pieces - self.short
    }
}

pub fn shelf_restore(library: &str, count: u64, lib: &Library) -> Result<u64, String> {
    let e = lib.get(library).ok_or_else(|| format!("{library} left the library"))?;
    let now = e.self_stock().unwrap_or(0) + count;
    crate::library::set_own(e, now, None)?;
    Ok(now)
}

fn label(g: &Group) -> String {
    let first = &g.refs[0];
    let refs = match g.refs.len() {
        1 => first.clone(),
        n => format!("{first} +{}", n - 1),
    };
    format!("{refs} {} {}", g.value, g.package).trim().to_string()
}

pub fn shelf_plan(groups: &[Group], picked: &[usize], lib: &Library) -> Shelf {
    let mut out = Shelf::default();
    let mut now: Vec<(String, u64)> = Vec::new();
    for &i in picked {
        let Some(g) = groups.get(i) else { continue };
        let own = g.library.as_deref().and_then(|n| lib.get(n)).and_then(Entry::self_stock);
        let (Some(name), Some(current)) = (g.library.clone(), own) else {
            out.skipped.push(Skipped { group: i, label: label(g) });
            continue;
        };
        let at = match now.iter().position(|(k, _)| *k == name) {
            Some(k) => k,
            None => {
                now.push((name.clone(), current));
                now.len() - 1
            }
        };
        let before = now[at].1;
        let pieces = g.refs.len() as u64;
        let after = before.saturating_sub(pieces);
        now[at].1 = after;
        out.takes.push(Take {
            group: i,
            library: name,
            label: label(g),
            pieces,
            before,
            after,
            short: pieces - (before - after),
            stale: g.own != Some(current),
        });
    }
    out
}

pub fn shelf_save(groups: &[Group], picked: &[usize], lib: &Library) -> Result<Shelf, String> {
    let plan = shelf_plan(groups, picked, lib);
    let mut done: Vec<&str> = Vec::new();
    for t in &plan.takes {
        if done.contains(&t.library.as_str()) {
            continue;
        }
        done.push(&t.library);
        let last = plan.takes.iter().rfind(|o| o.library == t.library).unwrap();
        let e = lib.get(&t.library).ok_or_else(|| format!("{} left the library", t.library))?;
        crate::library::set_own(e, last.after, None)?;
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_read_like_a_bag_label() {
        for (fp, want) in [
            ("C_0402_1005Metric", "0402"),
            ("LED_0603_1608Metric", "0603"),
            ("D_SOD-323", "SOD-323"),
            ("SOT-89-3", "SOT-89-3"),
            ("SOIC-14_3.9x8.7mm_P1.27mm", "SOIC-14"),
            ("Infineon_SG-WLL-2-3_0.58x0.28_P0.36mm", "SG-WLL-2-3"),
            ("Coilcraft_BCR-162", "BCR-162"),
            ("Crystal_SMD_3225-4Pin_3.2x2.5mm", "3225-4Pin"),
            ("PinHeader_1x02_P2.54mm_Vertical", "PinHeader 1x02"),
            ("SMA_Amphenol_132289_EdgeMount", "SMA"),
        ] {
            assert_eq!(package(fp), want, "{fp}");
        }
    }
}

#[cfg(test)]
mod shelf_tests {
    use super::*;

    fn lib(tag: &str, own: &[(&str, Option<u64>)]) -> (std::path::PathBuf, Library) {
        let root = std::env::temp_dir().join(format!("agentee-shelf-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, own) in own {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let stock = own.map_or(String::new(), |n| {
                format!("\n[[stock]]\ndistributor = \"home\"\nsku = \"{name}\"\nstock = 0\nchecked = \"2026-01-01\"\nown = {n}\nbin = \"A1\"\n")
            });
            std::fs::write(
                dir.join("part.toml"),
                format!("role = \"resistor\"\nmpn = \"{name}\"\nsymbol = \"R.sym.toml\"\n{stock}"),
            )
            .unwrap();
        }
        let l = Library::open(None, Some(&root));
        (root, l)
    }

    fn group(step: usize, side: Side, library: &str, refs: &[&str], own: Option<u64>) -> Group {
        Group {
            step,
            side,
            value: "100k".into(),
            package: "0603".into(),
            footprint: "R_0603_1608Metric".into(),
            mpn: None,
            refs: refs.iter().map(|r| r.to_string()).collect(),
            tht: false,
            hot: false,
            height: None,
            library: Some(library.into()),
            bin: None,
            own,
        }
    }

    fn own(root: &std::path::Path, name: &str) -> Option<u64> {
        Library::open(None, Some(root)).get(name).and_then(Entry::self_stock)
    }

    #[test]
    fn placed_groups_lower_the_stock_by_their_size() {
        let (root, lib) = lib("two", &[("RA", Some(40)), ("RB", Some(10))]);
        let groups = vec![
            group(1, Side::Top, "RA", &["R1", "R2", "R3"], Some(40)),
            group(2, Side::Top, "RB", &["R4"], Some(10)),
        ];
        let plan = shelf_save(&groups, &[0, 1], &lib).unwrap();
        assert_eq!(plan.takes[0].line(), "R1 +2 100k 0603: 40 -> 37");
        assert_eq!((own(&root, "RA"), own(&root, "RB")), (Some(37), Some(9)));
        let text = std::fs::read_to_string(root.join("RA/part.toml")).unwrap();
        assert!(text.contains("bin = \"A1\""), "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_short_shelf_saves_zero_and_flags_it_without_stopping_the_rest() {
        let (root, lib) = lib("short", &[("RA", Some(2)), ("RB", Some(10))]);
        let groups = vec![
            group(1, Side::Top, "RA", &["R1", "R2", "R3"], Some(2)),
            group(2, Side::Top, "RB", &["R4"], Some(10)),
        ];
        let plan = shelf_save(&groups, &[0, 1], &lib).unwrap();
        assert_eq!((plan.takes[0].after, plan.takes[0].short), (0, 1));
        assert_eq!((own(&root, "RA"), own(&root, "RB")), (None, Some(9)));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_part_without_own_stock_is_skipped_and_listed() {
        let (root, lib) = lib("none", &[("RA", None)]);
        let groups = vec![group(1, Side::Top, "RA", &["R1"], None)];
        let plan = shelf_save(&groups, &[0], &lib).unwrap();
        assert!(plan.takes.is_empty());
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(own(&root, "RA"), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_part_split_by_side_subtracts_the_total_once() {
        let (root, lib) = lib("sides", &[("RA", Some(10))]);
        let groups = vec![
            group(1, Side::Top, "RA", &["R1", "R2"], Some(10)),
            group(2, Side::Bottom, "RA", &["R3"], Some(10)),
        ];
        let plan = shelf_save(&groups, &[0, 1], &lib).unwrap();
        assert_eq!((plan.takes[1].before, plan.takes[1].after), (8, 7));
        assert_eq!(own(&root, "RA"), Some(7));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_current_count_wins_over_the_one_the_sheet_loaded_with() {
        let (root, lib) = lib("stale", &[("RA", Some(30))]);
        let groups = vec![group(1, Side::Top, "RA", &["R1"], Some(40))];
        let plan = shelf_save(&groups, &[0], &lib).unwrap();
        assert!(plan.takes[0].stale);
        assert_eq!(own(&root, "RA"), Some(29));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn putting_back_restores_what_was_taken() {
        let (root, lib) = lib("back", &[("RA", Some(5))]);
        assert_eq!(shelf_restore("RA", 3, &lib).unwrap(), 8);
        assert_eq!(own(&root, "RA"), Some(8));
        let _ = std::fs::remove_dir_all(root);
    }
}

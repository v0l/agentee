use super::*;
use serde_json::json;

pub const COMMANDS: &[&str] = &[
    "place",
    "unplace",
    "track",
    "untrack",
    "via",
    "unvia",
    "zone",
    "unzone",
    "pair",
    "text",
    "fanout",
    "stitch",
    "watermark",
    "title",
    "test",
    "board",
    "schematic",
];

pub fn command(name: &str) -> Option<Cmd> {
    Some(match name {
        "place" => place,
        "unplace" => unplace,
        "track" => track,
        "untrack" => untrack,
        "via" => via,
        "unvia" => unvia,
        "zone" => zone,
        "unzone" => unzone,
        "pair" => pair,
        "text" => text_cmd,
        "fanout" => fanout,
        "stitch" => stitch,
        "watermark" => watermark,
        "title" => title,
        "test" => test,
        "board" => board_ref,
        "schematic" => schematic_ref,
        _ => return None,
    })
}

pub fn bool_flags(name: &str) -> &'static [&'static str] {
    match name {
        "place" => &["bottom", "locked"],
        "track" => &[],
        "zone" => &["relief", "no-relief", "relief-tht-only"],
        "watermark" => &["hide"],
        "title" => &["clear"],
        "test" => &["through-holes", "no-vias"],
        _ => &[],
    }
}

pub fn usage(name: &str) -> &'static str {
    match name {
        "place" => "place REF [X,Y] [--rotation 90] [--bottom] [--locked]",
        "unplace" => "unplace REF ...",
        "track" => "track NET LAYER X,Y X,Y ... [--width 0.2mm]",
        "untrack" => "untrack NET [X,Y X,Y] | X,Y X,Y",
        "via" => "via NET X,Y [--via std] [--count 1] [--pitch 1.2,0]",
        "unvia" => "unvia NET X,Y | NET ...",
        "zone" => {
            "zone NET [--layers F.Cu,In1.Cu] [--outline X,Y ...] [--priority 1] [--clearance 0.25] [--min-width 0.15mm] [--min-island-area 2.0] [--pad-connection solid|relief|none] [--relief-gap 0.3mm] [--spoke-width 0.3mm] [--relief-tht-only]"
        }
        "unzone" => "unzone NET ...",
        "pair" => "pair NET+ NET- [--max-skew 1mm]",
        "text" => {
            "text STRING [--layer F.SilkS] [--at X,Y] [--size 1.0] [--rotation 90] [--locked]"
        }
        "fanout" => {
            "fanout REF [--via bga] [--skip-rings 2] [--always GND] [--skip A1] [--nets GND,3V3] [--exclude C2?]"
        }
        "stitch" => {
            "stitch NET [--via std] [--pitch 2.5mm] [--fence RF_*] [--offset 0.5mm] [--margin 0.6mm] [--outline X,Y ...]"
        }
        "watermark" => "watermark [--at X,Y] [--layer B.SilkS] [--rotation 90] [--hide]",
        "title" => {
            "title TEXT [--at X,Y] [--layer F.SilkS] [--rotation 90] [--size 1.5mm] | title --clear"
        }
        "test" => {
            "test [--nets 3V3,*RST*] [--exclude LED_*] [--side B] [--through-holes] [--no-vias] [--min-test-pad 1.0mm]"
        }
        "board" => "board NAME",
        "schematic" => "schematic NAME",
        _ => "",
    }
}

fn board_name(s: &Session, path: &Path) -> Result<String, String> {
    Ok(s.read(path)?.get("board").and_then(|v| v.as_str()).unwrap_or_default().to_string())
}

impl Session {
    fn sheet_tree(&self, root: &str) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        let mut queue: Vec<String> = vec![root.to_string()];
        let mut seen: Vec<String> = Vec::new();
        while !queue.is_empty() {
            let name = queue.remove(0);
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            let named = self.docs.keys().find(|p| {
                Kind::of(p) == Some(Kind::Schematic)
                    && self.read(p).ok().and_then(|d| d.get("name")).and_then(|v| v.as_str())
                        == Some(name.as_str())
            });
            let Some(path) = named.cloned() else { continue };
            let children = self.read(&path).map(|d| texts(d, "sheets")).unwrap_or_default();
            out.push((name, path));
            queue.extend(children);
        }
        out
    }

    pub fn sheet_of(&self, root: &str, reference: &str) -> Option<(String, PathBuf)> {
        self.sheet_tree(root)
            .into_iter()
            .find(|(_, path)| self.read(path).is_ok_and(|d| part_index_of(d, reference)))
    }
}

fn part_index_of(doc: &DocumentMut, reference: &str) -> bool {
    doc.get("parts")
        .and_then(|v| v.as_array_of_tables())
        .is_some_and(|a| a.iter().any(|t| text(t, "ref").as_deref() == Some(reference)))
}

fn net_known(s: &Session, path: &Path, net: &str) -> Result<(), String> {
    let schematic =
        s.read(path)?.get("schematic").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if schematic.is_empty() {
        return Err("this layout names no schematic, so its netlist is unknown".into());
    }
    let tree = s.sheet_tree(&schematic);
    if tree.is_empty() {
        return Err(format!("no schematic named `{schematic}` in the project"));
    }
    let mut nets: Vec<String> = Vec::new();
    for (_, sheet) in &tree {
        let Ok(doc) = s.read(sheet) else { continue };
        let here = doc
            .get("nets")
            .and_then(|v| v.as_array_of_tables())
            .map(|a| a.iter().filter_map(|t| text(t, "name")).collect::<Vec<_>>())
            .unwrap_or_default();
        nets.extend(here);
    }
    nets.sort();
    nets.dedup();
    if nets.iter().any(|n| n == net) {
        return Ok(());
    }
    if nets.is_empty() {
        return Err(format!("`{schematic}` and its sheets have no nets yet"));
    }
    let show = |v: &[String]| {
        if v.len() > 12 {
            format!("{} and {} more", v[..12].join(", "), v.len() - 12)
        } else {
            v.join(", ")
        }
    };
    let near: Vec<String> = nets
        .iter()
        .filter(|n| {
            n.to_lowercase().contains(&net.to_lowercase())
                || net.to_lowercase().contains(&n.to_lowercase())
        })
        .cloned()
        .collect();
    Err(if near.is_empty() {
        format!("`{net}` is not a net of {schematic} or its sheets; it has {}", show(&nets))
    } else {
        format!(
            "`{net}` is not a net of {schematic} or its sheets; did you mean: {}",
            near.join(", ")
        )
    })
}

fn place(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let reference = o.word("a reference")?;
    let given = match o.pos.is_empty() {
        true => None,
        false => Some(o.word("X,Y")?),
    };
    let typed = given.as_deref().map(|v| point(v, "place")).transpose()?;
    let at = o.at("at")?.or(typed);
    let rotation = o.typed("rotation")?;
    let bottom = o.flag("bottom");
    let locked = o.flag("locked");
    o.done("place", 0)?;
    let schematic =
        s.read(path)?.get("schematic").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let Some((sheet, _)) = s.sheet_of(&schematic, &reference) else {
        return Err(format!(
            "`{reference}` is not a part of schematic `{schematic}` or of any sheet it lists"
        ));
    };
    let existing =
        s.read(path)?.get("footprints").and_then(|v| v.as_array_of_tables()).and_then(|a| {
            a.iter().position(|t| text(t, "ref").as_deref() == Some(reference.as_str()))
        });
    let outline = s
        .board(Some(&board_name(s, path).unwrap_or_default()))
        .ok()
        .and_then(|b| b.outline.clone());
    let mut bounds = agentee_core::graphic::Bounds::EMPTY;
    if let Some(b) = outline {
        for p in b.points() {
            bounds.add(p);
        }
    }
    let spot = at.unwrap_or_else(|| {
        let mid = if bounds.is_empty() { 0.0 } else { (bounds.min[0] + bounds.max[0]) / 2.0 };
        tv(point_value([round(mid), 0.0]))
    });
    let mut t = match existing {
        Some(i) => s
            .read(path)?
            .get("footprints")
            .and_then(|v| v.as_array_of_tables())
            .and_then(|a| a.get(i))
            .cloned()
            .ok_or("no such placement")?,
        None => {
            let mut t = Table::new();
            set(&mut t, "ref", reference.clone());
            t
        }
    };
    set(&mut t, "at", spot);
    if let Some(r) = rotation {
        set(&mut t, "rotation", r);
    }
    if bottom {
        set(&mut t, "side", "bottom");
    }
    if locked {
        set(&mut t, "locked", true);
    }
    let (x, y) = spot_at(&t);
    let mut log = vec![format!("{reference} placed at {x},{y}")];
    if schematic != sheet {
        log.push(format!("{reference} is a part of sheet {sheet}"));
    }
    let doc = s.doc(path)?;
    match existing {
        Some(i) => {
            if let Some(a) = doc.get_mut("footprints").and_then(|v| v.as_array_of_tables_mut())
                && let Some(slot) = a.get_mut(i)
            {
                *slot = t;
            }
        }
        None => insert(doc, "footprints", t),
    }
    Ok(Report { log, facts: vec![json!({ "ref": reference, "at": [x, y], "sheet": sheet })] })
}

fn spot_at(t: &Table) -> (f64, f64) {
    let [x, y] = table_point(t, "at").unwrap_or([0.0, 0.0]);
    (x, y)
}

fn round(v: f64) -> f64 {
    (v / 0.05).round() * 0.05
}

fn point_value(at: [f64; 2]) -> TValue {
    let mut a = Array::new();
    a.push(at[0]);
    a.push(at[1]);
    TValue::Array(a)
}

fn unplace(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let refs = o.rest();
    o.done("unplace", 0)?;
    if refs.is_empty() {
        return Err("unplace: name the parts to take off the board".into());
    }
    let mut log = Vec::new();
    for reference in refs {
        let doc = s.doc(path)?;
        let a = doc
            .get_mut("footprints")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("this layout has no footprints")?;
        let before = a.len();
        a.retain(|t| text(t, "ref").as_deref() != Some(reference.as_str()));
        if a.len() == before {
            return Err(format!("`{reference}` has no placement in this layout"));
        }
        log.push(format!("{reference} taken off the board"));
    }
    Ok(Report { log, facts: Vec::new() })
}

fn track(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let net = o.word("a net name")?;
    let layer = o.word("a copper layer")?;
    let mut pts = o.rest();
    pts.extend(o.words("point")?);
    let width = o.typed("width")?;
    o.done("track", 0)?;
    net_known(s, path, &net)?;
    let board = board_name(s, path)?;
    s.check_layer(Some(&board), &layer)?;
    if pts.len() < 2 {
        return Err("track: a track needs at least two --point X,Y".into());
    }
    let mut t = Table::new();
    set(&mut t, "net", net.clone());
    set(&mut t, "layer", layer.clone());
    if let Some(w) = width {
        set(&mut t, "width", w);
    }
    set(&mut t, "points", points(&pts, "--point")?);
    insert(s.doc(path)?, "tracks", t);
    Ok(Report {
        log: vec![format!("{net} track on {layer} through {} points", pts.len())],
        facts: vec![json!({ "net": net, "layer": layer, "points": pts })],
    })
}

fn untrack(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let mut words = o.rest();
    words.extend(o.words("point")?);
    o.done("untrack", 0)?;
    let mut net: Option<String> = None;
    let mut ends: Vec<[f64; 2]> = Vec::new();
    for w in &words {
        if w.contains(',') {
            ends.push(parse_xy(w)?);
        } else if net.is_none() {
            net = Some(w.clone());
        } else {
            return Err(format!("untrack: `{w}` is neither X,Y nor the only net name"));
        }
    }
    if net.is_none() && ends.is_empty() {
        return Err("untrack: give a net name or the two points of a track".into());
    }
    if !ends.is_empty() && ends.len() != 2 {
        return Err("untrack: give the two points the removed span runs between".into());
    }
    let doc = s.doc(path)?;
    let a = doc
        .get_mut("tracks")
        .and_then(|v| v.as_array_of_tables_mut())
        .ok_or("this layout has no tracks")?;
    let of_net =
        |t: &Table| net.as_deref().is_none_or(|n| text(t, "net").as_deref() == Some(n));
    let before = a.len();
    let mut kept: Vec<Table> = Vec::new();
    let mut cut = 0usize;
    for t in a.iter() {
        if !of_net(t) {
            kept.push(t.clone());
            continue;
        }
        if ends.is_empty() {
            cut += 1;
            continue;
        }
        match split_track(t, ends[0], ends[1]) {
            Some(parts) => {
                cut += 1;
                kept.extend(parts);
            }
            None => kept.push(t.clone()),
        }
    }
    if cut == 0 {
        return Err(match (&net, ends.is_empty()) {
            (Some(n), true) => format!("no track of net `{n}`"),
            (Some(n), false) => format!("no track of net `{n}` runs between those points"),
            (None, _) => "no track runs between those points".into(),
        });
    }
    a.clear();
    for t in kept {
        a.push(t);
    }
    Ok(Report::log(match ends.is_empty() {
        true => format!("removed {} tracks", before - a.len()),
        false => format!("cut the span from {} tracks", cut),
    }))
}

fn split_track(t: &Table, from: [f64; 2], to: [f64; 2]) -> Option<Vec<Table>> {
    let pts = t.get("points").and_then(|v| v.as_array())?;
    let same = |v: &TValue, b: [f64; 2]| {
        point_of(v).is_some_and(|a| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
    };
    let i = pts.iter().position(|v| same(v, from))?;
    let j = pts.iter().position(|v| same(v, to))?;
    if i == j {
        return None;
    }
    let (lo, hi) = (i.min(j), i.max(j));
    let all: Vec<TValue> = pts.iter().cloned().collect();
    let piece = |span: &[TValue]| {
        (span.len() >= 2).then(|| {
            let mut p = t.clone();
            let mut a = Array::new();
            for v in span {
                a.push(v.clone());
            }
            a.fmt();
            set(&mut p, "points", TValue::Array(a));
            p
        })
    };
    Some([piece(&all[..=lo]), piece(&all[hi..])].into_iter().flatten().collect())
}

fn point_of(v: &TValue) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    let n =
        |i: usize| a.get(i).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
    Some([n(0)?, n(1)?])
}

fn parse_xy(tok: &str) -> Result<[f64; 2], String> {
    let (x, y) = tok.split_once(',').ok_or(format!("`{tok}` is not X,Y"))?;
    let num = |v: &str| -> Result<f64, String> {
        v.trim().parse::<f64>().map_err(|_| format!("`{v}` is not a number")).or_else(|e| {
            agentee_core::units::Length::parse(v).map(|l| l.to_mm()).map_err(|_| e.to_string())
        })
    };
    Ok([num(x)?, num(y)?])
}

fn via(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let net = o.word("a net name")?;
    let spot = o.word("X,Y")?;
    let at = point(&spot, "via")?;
    let kind = o.take("via");
    let count = o.take("count");
    let pitch = o.at("pitch")?;
    o.done("via", 0)?;
    net_known(s, path, &net)?;
    let board = board_name(s, path)?;
    if let Some(k) = &kind {
        let names = s.vias(Some(&board))?;
        if !names.iter().any(|v| v == k) {
            return Err(format!("no via `{k}` on the board; it has {}", names.join(", ")));
        }
    }
    let mut t = Table::new();
    set(&mut t, "net", net.clone());
    set(&mut t, "at", at);
    if let Some(k) = kind {
        set(&mut t, "via", k);
    }
    if let Some(c) = count {
        set(
            &mut t,
            "count",
            c.parse::<i64>().map_err(|_| format!("--count `{c}` is not a number"))?,
        );
    }
    if let Some(p) = pitch {
        set(&mut t, "pitch", p);
    }
    insert(s.doc(path)?, "vias", t);
    Ok(Report {
        log: vec![format!("{net} via placed at {spot}")],
        facts: vec![json!({ "net": net, "at": spot })],
    })
}

fn unvia(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let words = o.rest();
    let spot = words.get(1).map(|v| parse_xy(v)).transpose()?;
    o.done("unvia", 0)?;
    if words.is_empty() {
        return Err("unvia: name the net, or the net and X,Y".into());
    }
    let net = words[0].clone();
    let doc = s.doc(path)?;
    let a = doc
        .get_mut("vias")
        .and_then(|v| v.as_array_of_tables_mut())
        .ok_or("this layout has no vias")?;
    let before = a.len();
    a.retain(|t| {
        if text(t, "net").as_deref() != Some(net.as_str()) {
            return true;
        }
        match (spot, table_point(t, "at")) {
            (None, _) => false,
            (Some(want), Some(have)) => {
                (want[0] - have[0]).abs() > 1e-6 || (want[1] - have[1]).abs() > 1e-6
            }
            _ => true,
        }
    });
    if a.len() == before {
        return Err(format!("no via of `{net}` at that spot"));
    }
    Ok(Report::log(format!("removed {} vias of {net}", before - a.len())))
}

fn zone(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let net = o.word("a net name")?;
    let mut layers = o.words("layers")?;
    layers.extend(o.rest());
    let outline = o.words("outline")?;
    let priority = o.take("priority");
    let clearance = o.typed("clearance")?;
    let min_width = o.typed("min-width")?;
    let min_island = o.take("min-island-area");
    let connection = o.take("pad-connection");
    let relief = o.flag("relief");
    let no_relief = o.flag("no-relief");
    let relief_tht = o.flag("relief-tht-only");
    let relief_gap = o.typed("relief-gap")?;
    let spoke = o.typed("spoke-width")?;
    o.done("zone", 0)?;
    net_known(s, path, &net)?;
    let board = board_name(s, path)?;
    let layers = if layers.is_empty() { s.layers(Some(&board))? } else { layers };
    for l in &layers {
        s.check_layer(Some(&board), l)?;
    }
    let mut t = Table::new();
    set(&mut t, "net", net.clone());
    let mut a = Array::new();
    for l in &layers {
        a.push(l.as_str());
    }
    set(&mut t, "layers", TValue::Array(a));
    if !outline.is_empty() {
        set(&mut t, "outline", points(&outline, "--outline")?);
    }
    if let Some(p) = priority {
        set(&mut t, "priority", value("priority", &p)?);
    }
    for (key, v) in [
        ("clearance", clearance),
        ("min_width", min_width),
        ("relief_gap", relief_gap),
        ("spoke_width", spoke),
    ] {
        if let Some(v) = v {
            set(&mut t, key, v);
        }
    }
    if let Some(m) = min_island {
        set(&mut t, "min_island_area", value("min_island_area", &m)?);
    }
    let pad = connection.or(if relief {
        Some("relief".into())
    } else if no_relief {
        Some("none".into())
    } else {
        None
    });
    if let Some(p) = pad {
        set(&mut t, "pad_connection", p);
    }
    if relief_tht {
        set(&mut t, "relief_tht_only", true);
    }
    let doc = s.doc(path)?;
    match doc
        .get("zones")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|z| text(z, "net").as_deref() == Some(net.as_str())))
    {
        Some(i) => {
            if let Some(a) = doc.get_mut("zones").and_then(|v| v.as_array_of_tables_mut())
                && let Some(slot) = a.get_mut(i)
            {
                *slot = t;
            }
        }
        None => insert(doc, "zones", t),
    }
    Ok(Report {
        log: vec![format!("{net} poured on {}", layers.join(", "))],
        facts: vec![json!({ "net": net, "layers": layers })],
    })
}

fn unzone(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let nets = o.rest();
    o.done("unzone", 0)?;
    if nets.is_empty() {
        return Err("unzone: name the nets to unpour".into());
    }
    let mut log = Vec::new();
    for net in nets {
        let doc = s.doc(path)?;
        let a = doc
            .get_mut("zones")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("this layout has no zones")?;
        let before = a.len();
        a.retain(|z| text(z, "net").as_deref() != Some(net.as_str()));
        if a.len() == before {
            return Err(format!("no zone for `{net}`"));
        }
        log.push(format!("{net} unpoured"));
    }
    Ok(Report { log, facts: Vec::new() })
}

fn pair(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let p = o.word("the + net")?;
    let n = o.word("the - net")?;
    let skew = o.typed("max-skew")?;
    o.done("pair", 0)?;
    net_known(s, path, &p)?;
    net_known(s, path, &n)?;
    let mut t = Table::new();
    set(&mut t, "p", p.clone());
    set(&mut t, "n", n.clone());
    if let Some(k) = skew {
        set(&mut t, "max_skew", k);
    }
    let doc = s.doc(path)?;
    match doc
        .get("pairs")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|q| text(q, "p").as_deref() == Some(p.as_str())))
    {
        Some(i) => {
            if let Some(a) = doc.get_mut("pairs").and_then(|v| v.as_array_of_tables_mut())
                && let Some(slot) = a.get_mut(i)
            {
                *slot = t;
            }
        }
        None => insert(doc, "pairs", t),
    }
    Ok(Report {
        log: vec![format!("{p} and {n} are a pair")],
        facts: vec![json!({ "p": p, "n": n })],
    })
}

fn text_cmd(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let body = o.word("the text")?;
    let layer = o.take("layer").unwrap_or_else(|| "F.SilkS".into());
    let at = o.at("at")?;
    let size = o.typed("size")?;
    let rotation = o.typed("rotation")?;
    let locked = o.flag("locked");
    o.done("text", 0)?;
    if !matches!(layer.as_str(), "F.SilkS" | "B.SilkS" | "F.Fab" | "B.Fab") {
        return Err(format!(
            "`{layer}` is not a silk or fab layer: F.SilkS, B.SilkS, F.Fab, B.Fab"
        ));
    }
    let mut t = Table::new();
    set(&mut t, "kind", "text");
    set(&mut t, "layer", layer.clone());
    set(&mut t, "text", body.clone());
    if let Some(v) = at {
        set(&mut t, "at", v);
    }
    if let Some(v) = size {
        set(&mut t, "size", v);
    }
    if let Some(v) = rotation {
        set(&mut t, "rotation", v);
    }
    if locked {
        set(&mut t, "locked", true);
    }
    insert(s.doc(path)?, "graphics", t);
    Ok(Report {
        log: vec![format!("silk text `{body}` on {layer}")],
        facts: vec![json!({ "text": body, "layer": layer })],
    })
}

fn fanout(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let reference = o.word("a reference")?;
    let via = o.words("via")?;
    let skip_rings = o.take("skip-rings");
    let always = o.words("always")?;
    let skip = o.words("skip")?;
    let nets = o.words("nets")?;
    let exclude = o.words("exclude")?;
    o.done("fanout", 0)?;
    let board = board_name(s, path)?;
    if !via.is_empty() {
        let names = s.vias(Some(&board))?;
        for v in &via {
            if !names.iter().any(|k| k == v) {
                return Err(format!("no via `{v}` on the board; it has {}", names.join(", ")));
            }
        }
    }
    let mut t = Table::new();
    set(&mut t, "ref", reference.clone());
    if via.len() == 1 {
        set(&mut t, "via", via[0].clone());
    } else if via.len() > 1 {
        let mut a = Array::new();
        for v in &via {
            a.push(v.as_str());
        }
        set(&mut t, "via", TValue::Array(a));
    }
    for (key, words) in [("always", always), ("skip", skip), ("nets", nets), ("exclude", exclude)] {
        if !words.is_empty() {
            let mut a = Array::new();
            for w in words {
                a.push(w.as_str());
            }
            set(&mut t, key, TValue::Array(a));
        }
    }
    if let Some(r) = skip_rings {
        set(
            &mut t,
            "skip_rings",
            r.parse::<i64>().map_err(|_| format!("--skip-rings `{r}` is not a number"))?,
        );
    }
    let doc = s.doc(path)?;
    match doc
        .get("fanouts")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|f| text(f, "ref").as_deref() == Some(reference.as_str())))
    {
        Some(i) => {
            if let Some(a) = doc.get_mut("fanouts").and_then(|v| v.as_array_of_tables_mut())
                && let Some(slot) = a.get_mut(i)
            {
                *slot = t;
            }
        }
        None => insert(doc, "fanouts", t),
    }
    Ok(Report::log(format!("{reference} fans out to vias")))
}

fn stitch(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let net = o.word("a net name")?;
    let via = o.take("via");
    let pitch = o.typed("pitch")?;
    let outline = o.words("outline")?;
    let margin = o.typed("margin")?;
    let fence = o.words("fence")?;
    let offset = o.typed("offset")?;
    o.done("stitch", 0)?;
    net_known(s, path, &net)?;
    let board = board_name(s, path)?;
    if let Some(v) = &via {
        let names = s.vias(Some(&board))?;
        if !names.iter().any(|k| k == v) {
            return Err(format!("no via `{v}` on the board; it has {}", names.join(", ")));
        }
    }
    let mut t = Table::new();
    set(&mut t, "net", net.clone());
    if let Some(v) = via {
        set(&mut t, "via", v);
    }
    for (key, v) in [("pitch", pitch), ("margin", margin), ("offset", offset)] {
        if let Some(v) = v {
            set(&mut t, key, v);
        }
    }
    if !outline.is_empty() {
        set(&mut t, "outline", points(&outline, "--outline")?);
    }
    if !fence.is_empty() {
        let mut a = Array::new();
        for f in &fence {
            a.push(f.as_str());
        }
        set(&mut t, "fence", TValue::Array(a));
    }
    insert(s.doc(path)?, "stitching", t);
    Ok(Report::log(format!("{net} stitched with vias")))
}

fn watermark(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let at = o.at("at")?;
    let layer = o.take("layer");
    let rotation = o.typed("rotation")?;
    let hide = o.flag("hide");
    o.done("watermark", 0)?;
    if hide {
        s.doc(path)?.remove("watermark");
        return Ok(Report::log("watermark back to automatic"));
    }
    if at.is_none() && layer.is_none() && rotation.is_none() {
        s.doc(path)?.remove("watermark");
        return Ok(Report::log("watermark back to automatic"));
    }
    let mut t = Table::new();
    if let Some(v) = at {
        set(&mut t, "at", v);
    }
    if let Some(l) = layer {
        if !matches!(l.as_str(), "F.SilkS" | "B.SilkS") {
            return Err(format!("the watermark goes on silk, not `{l}`"));
        }
        set(&mut t, "layer", l);
    }
    if let Some(r) = rotation {
        set(&mut t, "rotation", r);
    }
    s.doc(path)?["watermark"] = Item::Table(t);
    Ok(Report::log("watermark spot set"))
}

fn title(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    if o.flag("clear") {
        o.done("title", 0)?;
        s.doc(path)?.remove("title");
        return Ok(Report::log("title removed"));
    }
    let body = o.word("the title text")?;
    let at = o.at("at")?;
    let layer = o.take("layer");
    let rotation = o.typed("rotation")?;
    let size = match o.take("size") {
        Some(v) => Some(length(&v, "--size")?),
        None => None,
    };
    o.done("title", 0)?;
    if let Some(l) = &layer
        && !matches!(l.as_str(), "F.SilkS" | "B.SilkS")
    {
        return Err(format!("the title goes on silk, F.SilkS or B.SilkS, not `{l}`"));
    }
    let plain = at.is_none() && layer.is_none() && rotation.is_none() && size.is_none();
    let doc = s.doc(path)?;
    if plain {
        doc["title"] = toml_edit::value(body.clone());
    } else {
        let mut t = toml_edit::InlineTable::new();
        t.insert("text", body.clone().into());
        for (k, v) in
            [("at", at), ("layer", layer.map(Into::into)), ("rotation", rotation), ("size", size)]
        {
            if let Some(v) = v {
                t.insert(k, v);
            }
        }
        doc["title"] = Item::Value(TValue::InlineTable(t));
    }
    Ok(Report { log: vec![format!("title `{body}`")], facts: vec![json!({ "title": body })] })
}

fn test(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let nets = o.words("nets")?;
    let exclude = o.words("exclude")?;
    let side = o.take("side");
    let through_holes = o.flag("through-holes");
    let no_vias = o.flag("no-vias");
    let min_pad = o.typed("min-test-pad")?;
    o.done("test", 0)?;
    if let Some(side) = &side
        && !matches!(side.to_ascii_uppercase().as_str(), "F" | "B" | "TOP" | "BOTTOM")
    {
        return Err(format!("side `{side}` is not F or B"));
    }
    let mut t = table_of(s.doc(path)?.get("test"))?;
    for (key, words) in [("nets", nets), ("exclude", exclude)] {
        if !words.is_empty() {
            let mut a = Array::new();
            for w in words {
                a.push(w.as_str());
            }
            set(&mut t, key, TValue::Array(a));
        }
    }
    if let Some(side) = side {
        set(&mut t, "side", side);
    }
    if through_holes {
        set(&mut t, "through_holes", true);
    }
    if no_vias {
        set(&mut t, "vias", false);
    }
    if let Some(p) = min_pad {
        set(&mut t, "min_test_pad", p);
    }
    s.doc(path)?["test"] = Item::Table(t);
    Ok(Report::log("test access set"))
}

fn board_ref(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let name = o.word("a board name")?;
    o.done("board", 0)?;
    if s.item(Kind::Board, &name).is_none() {
        return Err(format!("no board named `{name}` in this project"));
    }
    set(s.doc(path)?, "board", name.clone());
    Ok(Report::log(format!("layout uses board {name}")))
}

fn schematic_ref(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let name = o.word("a schematic name")?;
    o.done("schematic", 0)?;
    if s.item(Kind::Schematic, &name).is_none() {
        return Err(format!("no schematic named `{name}` in this project"));
    }
    set(s.doc(path)?, "schematic", name.clone());
    Ok(Report::log(format!("layout places schematic {name}")))
}

pub fn show(s: &Session, path: &Path) -> Value {
    let doc = &s.docs[path];
    let at = |key: &str| doc.get(key).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    json!({
        "name": doc.get("name").and_then(|v| v.as_str()),
        "board": doc.get("board").and_then(|v| v.as_str()),
        "schematic": doc.get("schematic").and_then(|v| v.as_str()),
        "footprints": at("footprints"),
        "tracks": at("tracks"),
        "vias": at("vias"),
        "zones": at("zones"),
        "cutouts": at("cutouts"),
        "pairs": at("pairs"),
        "graphics": at("graphics"),
        "fanouts": at("fanouts"),
        "stitching": at("stitching"),
        "file": path.display().to_string(),
    })
}

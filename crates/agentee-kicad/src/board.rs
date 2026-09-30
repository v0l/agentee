use crate::footprint;
use crate::sexpr::{self, Node};
use agentee_core::board::BoardFile;
use agentee_core::footprint::FootprintFile;
use agentee_core::layout::LayoutFile;
use agentee_core::schematic::SchematicFile;
use agentee_core::symbol::SymbolFile;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub struct BoardImport {
    pub board: BoardFile,
    pub layout: LayoutFile,
    pub schematic: SchematicFile,
    pub footprints: Vec<FootprintFile>,
    pub symbols: Vec<SymbolFile>,
    pub notes: Vec<String>,
}

type P = [f64; 2];

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn pt(p: P) -> Value {
    json!([round(p[0]), round(p[1])])
}

fn mm(v: f64) -> String {
    format!("{}mm", round(v))
}

fn rotate_kicad(p: P, deg: f64) -> P {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c + p[1] * s, -p[0] * s + p[1] * c]
}

fn arc_points(start: P, mid: P, end: P) -> Vec<P> {
    let (ax, ay, bx, by, cx, cy) = (start[0], start[1], mid[0], mid[1], end[0], end[1]);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-12 {
        return vec![start, end];
    }
    let ux = ((ax * ax + ay * ay) * (by - cy)
        + (bx * bx + by * by) * (cy - ay)
        + (cx * cx + cy * cy) * (ay - by))
        / d;
    let uy = ((ax * ax + ay * ay) * (cx - bx)
        + (bx * bx + by * by) * (ax - cx)
        + (cx * cx + cy * cy) * (bx - ax))
        / d;
    let r = ((ax - ux).powi(2) + (ay - uy).powi(2)).sqrt();
    let a0 = (ay - uy).atan2(ax - ux);
    let am = (by - uy).atan2(bx - ux);
    let a1 = (cy - uy).atan2(cx - ux);
    let norm = |a: f64| a.rem_euclid(std::f64::consts::TAU);
    let ccw = norm(am - a0) < norm(a1 - a0);
    let sweep = if ccw { norm(a1 - a0) } else { -norm(a0 - a1) };
    let step = 2.0 * (1.0 - 5e-4 / r.max(1e-3)).clamp(-1.0, 1.0).acos();
    let n = ((sweep.abs() / step.max(1f64.to_radians())).ceil() as usize).clamp(2, 720);
    (0..=n)
        .map(|k| {
            let a = a0 + sweep * k as f64 / n as f64;
            [ux + r * a.cos(), uy + r * a.sin()]
        })
        .collect()
}

fn flip_layer(l: &str) -> String {
    if let Some(r) = l.strip_prefix("F.") {
        format!("B.{r}")
    } else if let Some(r) = l.strip_prefix("B.") {
        format!("F.{r}")
    } else {
        l.to_string()
    }
}

fn net_of(n: &Node, table: &HashMap<i64, String>) -> Option<String> {
    let net = n.find("net")?;
    if let Some(name) = net.arg(1) {
        return Some(name.to_string()).filter(|s| !s.is_empty());
    }
    let a = net.arg(0)?;
    match a.parse::<i64>() {
        Ok(id) => table.get(&id).cloned().filter(|s| !s.is_empty()),
        Err(_) => Some(a.to_string()).filter(|s| !s.is_empty()),
    }
}

fn normalize_footprint(fp: &Node, bottom: bool, rot: f64) -> Node {
    fn walk(n: &Node, bottom: bool, rot: f64, parent: Option<&str>) -> Node {
        let Node::List(items) = n else { return n.clone() };
        let head = n.head().unwrap_or("");
        let mut out: Vec<Node> = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let mut item = walk(item, bottom, rot, Some(head));
            if i == 0 {
                out.push(item);
                continue;
            }
            if let Node::List(inner) = &mut item {
                let ih = inner.first().and_then(Node::text).unwrap_or("").to_string();
                let in_pad = head == "pad";
                let geometric =
                    matches!(ih.as_str(), "at" | "start" | "end" | "mid" | "center" | "xy");
                if geometric && head != "footprint" && parent != Some("model") && head != "offset" {
                    if bottom
                        && inner.len() >= 3
                        && let Some(x) = inner[1].text().and_then(|t| t.parse::<f64>().ok())
                    {
                        inner[1] = Node::Atom(format!("{}", round(-x)));
                    }
                    if ih == "at" && in_pad {
                        let a = inner
                            .get(3)
                            .and_then(|t| t.text())
                            .and_then(|t| t.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        let rel = if bottom { -(a - rot) } else { a - rot };
                        let rel = (rel + 180.0).rem_euclid(360.0) - 180.0;
                        inner.truncate(3);
                        if rel.abs() > 1e-9 {
                            inner.push(Node::Atom(format!("{}", round(rel))));
                        }
                    }
                }
                if bottom && matches!(ih.as_str(), "layer" | "layers") {
                    for t in inner.iter_mut().skip(1) {
                        if let Some(s) = t.text() {
                            *t = Node::Atom(flip_layer(s));
                        }
                    }
                }
            }
            out.push(item);
        }
        Node::List(out)
    }
    walk(fp, bottom, rot, None)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-+".contains(c) { c } else { '_' })
        .collect()
}

fn chain_loops(segs: &[Vec<P>]) -> Vec<Vec<P>> {
    let close = |a: P, b: P| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
    let mut left: Vec<Vec<P>> = segs.iter().filter(|s| s.len() >= 2).cloned().collect();
    let mut loops = Vec::new();
    while let Some(mut cur) = left.pop() {
        loop {
            if cur.len() > 2 && close(cur[0], *cur.last().unwrap()) {
                cur.pop();
                break;
            }
            let end = *cur.last().unwrap();
            let Some(k) =
                left.iter().position(|s| close(s[0], end) || close(*s.last().unwrap(), end))
            else {
                break;
            };
            let mut s = left.swap_remove(k);
            if !close(s[0], end) {
                s.reverse();
            }
            cur.extend(s.into_iter().skip(1));
        }
        if cur.len() >= 3 {
            loops.push(cur);
        }
    }
    loops
}

fn area(r: &[P]) -> f64 {
    let mut a = 0.0;
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    (a / 2.0).abs()
}

fn shape_points(n: &Node) -> Option<Vec<P>> {
    let head = n.head()?;
    Some(match head.trim_start_matches("gr_").trim_start_matches("fp_") {
        "line" => vec![n.xy("start")?, n.xy("end")?],
        "arc" => match n.xy("mid") {
            Some(mid) => arc_points(n.xy("start")?, mid, n.xy("end")?),
            None => return None,
        },
        "rect" => {
            let (a, b) = (n.xy("start")?, n.xy("end")?);
            vec![a, [b[0], a[1]], b, [a[0], b[1]], a]
        }
        "circle" => {
            let (c, e) = (n.xy("center")?, n.xy("end")?);
            let r = ((e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2)).sqrt();
            let mut v: Vec<P> = (0..48)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / 48.0;
                    [c[0] + r * a.cos(), c[1] + r * a.sin()]
                })
                .collect();
            v.push(v[0]);
            v
        }
        "poly" => {
            let mut v = n.pts();
            if let Some(f) = v.first().copied() {
                v.push(f);
            }
            v
        }
        _ => return None,
    })
}

fn kind_of(t: &str) -> Option<&'static str> {
    let t = t.to_lowercase();
    Some(if t == "copper" {
        "copper"
    } else if t.contains("core") {
        "core"
    } else if t.contains("prepreg") {
        "prepreg"
    } else if t.contains("mask") {
        "mask"
    } else if t.contains("silk") {
        "silk"
    } else if t.contains("paste") {
        "paste"
    } else {
        return None;
    })
}

fn glob(p: &str, s: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => go(&p[1..], s) || (!s.is_empty() && go(p, &s[1..])),
            (Some(b'?'), Some(_)) => go(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &s[1..]),
            _ => false,
        }
    }
    go(p.as_bytes(), s.as_bytes())
}

fn text_variables(root: &Node, pro: &Value) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    if let Some(tb) = root.find("title_block") {
        for (key, var) in
            [("title", "TITLE"), ("date", "DATE"), ("rev", "REVISION"), ("company", "COMPANY")]
        {
            if let Some(v) = tb.find(key).and_then(|n| n.arg(0)) {
                vars.insert(var.to_string(), v.to_string());
            }
        }
        for c in tb.all("comment") {
            if let (Some(i), Some(v)) = (c.arg(0), c.arg(1)) {
                vars.insert(format!("COMMENT{i}"), v.to_string());
            }
        }
    }
    if let Some(m) = pro["text_variables"].as_object() {
        for (k, v) in m {
            if let Some(v) = v.as_str() {
                vars.insert(k.clone(), v.to_string());
            }
        }
    }
    vars
}

fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

fn text_lines(n: &Node, content: &str) -> Vec<agentee_core::graphic::GraphicFile> {
    let lines: Vec<&str> = content.trim_end_matches('\n').split('\n').collect();
    let Some(first) = footprint::text(n, lines[0]) else { return Vec::new() };
    let size = first.size.map(|s| s.to_mm()).unwrap_or(1.0);
    let rot = first.rotation.unwrap_or(0.0);
    let at = first.at.map(|a| a.to_mm()).unwrap_or([0.0, 0.0]);
    let pitch = size * 1.62;
    let middle = (lines.len() as f64 - 1.0) / 2.0;
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let off = rotate_kicad([0.0, (i as f64 - middle) * pitch], rot);
            agentee_core::graphic::GraphicFile {
                text: Some(line.to_string()),
                at: Some(agentee_core::units::Point::mm(at[0] + off[0], at[1] + off[1])),
                ..first.clone()
            }
        })
        .collect()
}

fn shift_graphic(g: &mut agentee_core::graphic::GraphicFile, shift: &dyn Fn(P) -> P) {
    use agentee_core::units::Point;
    let mv = |p: &mut Option<Point>| {
        if let Some(q) = p {
            let [x, y] = shift(q.to_mm());
            *q = Point::mm(x, y);
        }
    };
    mv(&mut g.start);
    mv(&mut g.mid);
    mv(&mut g.end);
    mv(&mut g.center);
    mv(&mut g.at);
    if let Some(pts) = &mut g.points {
        for q in pts.iter_mut() {
            let [x, y] = shift(q.to_mm());
            *q = Point::mm(x, y);
        }
    }
}

pub fn import_board(text: &str, project: Option<&str>, name: &str) -> Result<BoardImport, String> {
    let root = sexpr::parse(text).map_err(|e| e.to_string())?;
    if root.head() != Some("kicad_pcb") {
        return Err("not a KiCad board".into());
    }
    let mut notes = Vec::new();
    let mut nets: HashMap<i64, String> = HashMap::new();
    for n in root.all("net") {
        if let (Some(id), Some(nm)) = (n.num(0), n.arg(1)) {
            nets.insert(id as i64, nm.to_string());
        }
    }
    let copper: Vec<String> = root
        .find("layers")
        .map(|l| {
            l.items()
                .iter()
                .skip(1)
                .filter_map(|x| x.arg(0).map(str::to_string))
                .filter(|n| n.ends_with(".Cu"))
                .collect()
        })
        .unwrap_or_default();

    let mut edge: Vec<Vec<P>> = Vec::new();
    for item in root.items().iter().skip(1) {
        if item.head().is_some_and(|h| h.starts_with("gr_"))
            && item.find("layer").and_then(|l| l.arg(0)) == Some("Edge.Cuts")
        {
            edge.extend(shape_points(item));
        }
    }
    let footprints_nodes: Vec<&Node> = root.all("footprint").collect();
    for fp in &footprints_nodes {
        let at = fp.find("at");
        let origin =
            [at.and_then(|a| a.num(0)).unwrap_or(0.0), at.and_then(|a| a.num(1)).unwrap_or(0.0)];
        let rot = at.and_then(|a| a.num(2)).unwrap_or(0.0);
        for g in fp.items() {
            if g.head().is_some_and(|h| h.starts_with("fp_"))
                && g.find("layer").and_then(|l| l.arg(0)) == Some("Edge.Cuts")
                && let Some(pts) = shape_points(g)
            {
                edge.push(
                    pts.into_iter()
                        .map(|p| {
                            let r = rotate_kicad(p, rot);
                            [r[0] + origin[0], r[1] + origin[1]]
                        })
                        .collect(),
                );
            }
        }
    }
    let mut loops = chain_loops(&edge);
    loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
    let outline = loops.first().cloned().ok_or("no closed Edge.Cuts outline")?;
    let mut lo = [f64::MAX; 2];
    for p in &outline {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
    }
    let shift = |p: P| [p[0] - lo[0], p[1] - lo[1]];

    let mut stack = Vec::new();
    let setup = root.find("setup");
    let st = setup.and_then(|s| s.find("stackup"));
    let mut finish = None;
    let mut mask_color = None;
    if let Some(st) = st {
        for l in st.all("layer") {
            let lname = l.arg(0).unwrap_or("");
            let Some(kind) = l.find("type").and_then(|t| t.arg(0)).and_then(kind_of) else {
                continue;
            };
            let mut e = serde_json::Map::new();
            e.insert("kind".into(), json!(kind));
            if matches!(kind, "copper" | "mask" | "silk" | "paste") {
                e.insert("name".into(), json!(lname));
            }
            if let Some(t) = l.find("thickness").and_then(|t| t.num(0)) {
                e.insert("thickness".into(), json!(mm(t)));
            }
            if let Some(m) = l.find("material").and_then(|m| m.arg(0)) {
                e.insert("material".into(), json!(m));
            }
            if let Some(er) = l.find("epsilon_r").and_then(|m| m.num(0)) {
                e.insert("er".into(), json!(er));
            }
            if let Some(tan) = l.find("loss_tangent").and_then(|m| m.num(0)) {
                e.insert("loss_tangent".into(), json!(tan));
            }
            if kind == "mask" && mask_color.is_none() {
                mask_color = l.find("color").and_then(|c| c.arg(0)).map(|c| c.to_lowercase());
            }
            if matches!(kind, "silk" | "paste") {
                continue;
            }
            stack.push(Value::Object(e));
        }
        finish = st
            .find("copper_finish")
            .and_then(|f| f.arg(0))
            .filter(|f| *f != "None")
            .map(str::to_string);
    }
    let mut stackup = serde_json::Map::new();
    if stack.is_empty() {
        let n = copper.len().max(2);
        stackup.insert(
            "preset".into(),
            json!(if n >= 4 { "jlcpcb-4l-1.6mm-7628" } else { "jlcpcb-2l-1.6mm" }),
        );
        notes.push("the board has no stackup, a 1.6 mm preset stands in".into());
    } else {
        stackup.insert("layers".into(), json!(stack));
    }
    if let Some(f) = finish {
        stackup.insert("finish".into(), json!(f));
    }
    if let Some(c) = mask_color {
        stackup.insert("mask_color".into(), json!(c));
    }

    let pro: Value = project.and_then(|p| serde_json::from_str(p).ok()).unwrap_or(Value::Null);
    let settings = &pro["net_settings"];
    let mut via_types: BTreeMap<(i64, i64), String> = BTreeMap::new();
    let mut via_name = |drill: f64, dia: f64| -> String {
        let key = ((drill * 1e4).round() as i64, (dia * 1e4).round() as i64);
        via_types.entry(key).or_insert_with(|| format!("v{}-{}", round(drill), round(dia))).clone()
    };
    let mut classes = Vec::new();
    for c in settings["classes"].as_array().into_iter().flatten() {
        let Some(cname) = c["name"].as_str() else { continue };
        let mut e = serde_json::Map::new();
        e.insert("name".into(), json!(cname));
        if let Some(v) = c["track_width"].as_f64() {
            e.insert("track_width".into(), json!(mm(v)));
        }
        if let Some(v) = c["clearance"].as_f64() {
            e.insert("clearance".into(), json!(mm(v)));
        }
        if let (Some(d), Some(s)) = (c["via_drill"].as_f64(), c["via_diameter"].as_f64()) {
            e.insert("via".into(), json!(via_name(d, s)));
        }
        if let Some(v) = c["diff_pair_gap"].as_f64() {
            e.insert("diff_gap".into(), json!(mm(v)));
        }
        classes.push(Value::Object(e));
    }
    let mut assign: Vec<(String, String)> = Vec::new();
    for p in settings["netclass_patterns"].as_array().into_iter().flatten() {
        if let (Some(c), Some(pat)) = (p["netclass"].as_str(), p["pattern"].as_str()) {
            assign.push((pat.to_string(), c.to_string()));
        }
    }
    if let Some(m) = settings["netclass_assignments"].as_object() {
        for (net, c) in m {
            if let Some(c) =
                c.as_str().or_else(|| c.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()))
            {
                assign.push((net.clone(), c.to_string()));
            }
        }
    }
    if !classes.iter().any(|c| c["name"] == "Default") {
        classes.insert(0, json!({ "name": "Default" }));
    }

    let mut fp_defs: Vec<(String, FootprintFile)> = Vec::new();
    let mut placements = Vec::new();
    let mut parts = Vec::new();
    let mut pins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut symbols: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (idx, fp) in footprints_nodes.iter().enumerate() {
        let at = fp.find("at");
        let origin =
            [at.and_then(|a| a.num(0)).unwrap_or(0.0), at.and_then(|a| a.num(1)).unwrap_or(0.0)];
        let rot = at.and_then(|a| a.num(2)).unwrap_or(0.0);
        let bottom = fp.find("layer").and_then(|l| l.arg(0)) == Some("B.Cu");
        let reference = fp
            .property_text("Reference")
            .or_else(|| {
                fp.all("fp_text")
                    .find(|t| t.arg(0) == Some("reference"))
                    .and_then(|t| t.arg(1))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("X{}", idx + 1));
        let reference = {
            let mut r = reference;
            if r.is_empty() || r.contains('*') {
                r = format!("X{}", idx + 1);
            }
            let mut unique = r.clone();
            let mut k = 2;
            while used.contains(&unique) {
                unique = format!("{r}_{k}");
                k += 1;
            }
            used.insert(unique.clone());
            unique
        };
        let value = fp
            .property_text("Value")
            .or_else(|| {
                fp.all("fp_text")
                    .find(|t| t.arg(0) == Some("value"))
                    .and_then(|t| t.arg(1))
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let lib_id = fp.arg(0).unwrap_or("footprint");
        let base = sanitize(lib_id.rsplit(':').next().unwrap_or(lib_id));
        let local = normalize_footprint(fp, bottom, rot);
        let mut def = footprint::convert(&local)?;
        def.name = base.clone();
        let shapes: Vec<_> = def.graphics.iter().filter(|g| g.text.is_none()).collect();
        let text = format!("{:?}{:?}", def.pads, shapes);
        let fp_name = match fp_defs.iter().find(|(t, f)| *t == text && f.name.starts_with(&base)) {
            Some((_, f)) => f.name.clone(),
            None => {
                let taken = fp_defs
                    .iter()
                    .filter(|(_, f)| f.name == base || f.name.starts_with(&format!("{base}_v")))
                    .count();
                if taken > 0 {
                    def.name = format!("{base}_v{}", taken + 1);
                }
                fp_defs.push((text, def.clone()));
                def.name.clone()
            }
        };
        let mut numbers = BTreeSet::new();
        for pad in fp.all("pad") {
            let number = pad.arg(0).unwrap_or("").to_string();
            if number.is_empty() {
                continue;
            }
            numbers.insert(number.clone());
            if let Some(net) = net_of(pad, &nets)
                && !net.starts_with("unconnected-")
            {
                pins.entry(net).or_default().insert(format!("{reference}.{number}"));
            }
        }
        let symbol = format!("KiCad_{fp_name}");
        symbols.entry(symbol.clone()).or_default().extend(numbers);
        let mut place = serde_json::Map::new();
        place.insert("ref".into(), json!(reference));
        place.insert("at".into(), pt(shift(origin)));
        let r = (rot + 180.0).rem_euclid(360.0) - 180.0;
        if r.abs() > 1e-9 {
            place.insert("rotation".into(), json!(round(r)));
        }
        if bottom {
            place.insert("side".into(), json!("bottom"));
        }
        placements.push(Value::Object(place));
        let col = parts.len() % 12;
        let row = parts.len() / 12;
        let mut part = serde_json::Map::new();
        part.insert("ref".into(), json!(reference));
        part.insert("symbol".into(), json!(symbol));
        if !value.is_empty() {
            part.insert("value".into(), json!(value));
        }
        part.insert("footprint".into(), json!(fp_name));
        part.insert("at".into(), json!([25.4 + col as f64 * 25.4, 25.4 + row as f64 * 25.4]));
        if fp.find("attr").is_some_and(|a| a.has_atom("dnp")) {
            part.insert("dnp".into(), json!(true));
        }
        parts.push(Value::Object(part));
    }

    let mut segs: Vec<(String, String, f64, Vec<P>)> = Vec::new();
    for item in root.items().iter().skip(1) {
        match item.head() {
            Some("segment") => {
                let (Some(a), Some(b)) = (item.xy("start"), item.xy("end")) else { continue };
                let Some(net) = net_of(item, &nets) else { continue };
                let layer = item.find("layer").and_then(|l| l.arg(0)).unwrap_or("F.Cu").to_string();
                let w = item.find("width").and_then(|w| w.num(0)).unwrap_or(0.2);
                segs.push((net, layer, w, vec![a, b]));
            }
            Some("arc") => {
                let (Some(a), Some(m), Some(b)) =
                    (item.xy("start"), item.xy("mid"), item.xy("end"))
                else {
                    continue;
                };
                let Some(net) = net_of(item, &nets) else { continue };
                let layer = item.find("layer").and_then(|l| l.arg(0)).unwrap_or("F.Cu").to_string();
                let w = item.find("width").and_then(|w| w.num(0)).unwrap_or(0.2);
                segs.push((net, layer, w, arc_points(a, m, b)));
            }
            _ => {}
        }
    }
    let mut groups: BTreeMap<(String, String, i64), Vec<Vec<P>>> = BTreeMap::new();
    for (net, layer, w, pts) in segs {
        groups.entry((net, layer, (w * 1e6).round() as i64)).or_default().push(pts);
    }
    let mut tracks = Vec::new();
    let close = |a: P, b: P| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    for ((net, layer, w), mut list) in groups {
        let mut degree: HashMap<(i64, i64), usize> = HashMap::new();
        let key = |p: P| ((p[0] * 1e5).round() as i64, (p[1] * 1e5).round() as i64);
        for s in &list {
            *degree.entry(key(s[0])).or_default() += 1;
            *degree.entry(key(*s.last().unwrap())).or_default() += 1;
        }
        while let Some(mut cur) = list.pop() {
            loop {
                let end = *cur.last().unwrap();
                if degree.get(&key(end)).copied().unwrap_or(0) != 2 {
                    break;
                }
                let Some(k) =
                    list.iter().position(|s| close(s[0], end) || close(*s.last().unwrap(), end))
                else {
                    break;
                };
                let mut s = list.swap_remove(k);
                if !close(s[0], end) {
                    s.reverse();
                }
                cur.extend(s.into_iter().skip(1));
            }
            tracks.push(json!({
                "net": net,
                "layer": layer,
                "width": mm(w as f64 / 1e6),
                "points": cur.iter().map(|p| pt(shift(*p))).collect::<Vec<_>>(),
            }));
        }
    }

    let mut vias = Vec::new();
    let mut spans: HashMap<String, (String, String)> = HashMap::new();
    for v in root.all("via") {
        let Some(at) = v.xy("at") else { continue };
        let Some(net) = net_of(v, &nets) else { continue };
        let size = v.find("size").and_then(|s| s.num(0)).unwrap_or(0.6);
        let drill = v.find("drill").and_then(|s| s.num(0)).unwrap_or(0.3);
        let layers: Vec<String> = v
            .find("layers")
            .map(|l| l.items().iter().skip(1).filter_map(Node::text).map(str::to_string).collect())
            .unwrap_or_default();
        let mut vname = via_name(drill, size);
        if layers.len() == 2 && copper.first() != Some(&layers[0])
            || layers.len() == 2 && copper.last() != Some(&layers[1])
        {
            vname = format!(
                "{vname}-{}-{}",
                layers[0].trim_end_matches(".Cu"),
                layers[1].trim_end_matches(".Cu")
            );
            spans.insert(vname.clone(), (layers[0].clone(), layers[1].clone()));
        }
        vias.push(json!({ "net": net, "at": pt(shift(at)), "via": vname }));
    }
    let mut board_vias: Vec<Value> = via_types
        .iter()
        .map(|((d, s), n)| json!({ "name": n, "drill": mm(*d as f64 / 1e4), "diameter": mm(*s as f64 / 1e4) }))
        .collect();
    for (n, (from, to)) in &spans {
        let base =
            board_vias.iter().find(|v| n.starts_with(v["name"].as_str().unwrap_or("?"))).cloned();
        if let Some(mut b) = base {
            b["name"] = json!(n);
            b["from"] = json!(from);
            b["to"] = json!(to);
            board_vias.push(b);
        }
    }

    let mut zones = Vec::new();
    let mut keepouts = 0;
    let mut teardrops = 0;
    for z in root.all("zone") {
        if z.find("keepout").is_some() {
            keepouts += 1;
            continue;
        }
        if z.find("attr").is_some_and(|a| a.find("teardrop").is_some()) {
            teardrops += 1;
            continue;
        }
        let Some(net) = net_of(z, &nets).or_else(|| {
            z.find("net_name").and_then(|n| n.arg(0)).map(str::to_string).filter(|s| !s.is_empty())
        }) else {
            continue;
        };
        let layers: Vec<String> = match z.find("layers") {
            Some(l) => {
                l.items().iter().skip(1).filter_map(Node::text).map(str::to_string).collect()
            }
            None => z
                .find("layer")
                .and_then(|l| l.arg(0))
                .map(|l| vec![l.to_string()])
                .unwrap_or_default(),
        };
        let layers: Vec<String> = layers
            .into_iter()
            .flat_map(|l| {
                if l == "F&B.Cu" {
                    vec!["F.Cu".to_string(), "B.Cu".to_string()]
                } else if l == "*.Cu" {
                    copper.clone()
                } else {
                    vec![l]
                }
            })
            .collect();
        let Some(poly) = z.find("polygon").map(Node::pts) else { continue };
        let mut e = serde_json::Map::new();
        e.insert("net".into(), json!(net));
        e.insert("layers".into(), json!(layers));
        e.insert("outline".into(), json!(poly.iter().map(|p| pt(shift(*p))).collect::<Vec<_>>()));
        if let Some(c) =
            z.find("connect_pads").and_then(|c| c.find("clearance")).and_then(|c| c.num(0))
        {
            e.insert("clearance".into(), json!(mm(c)));
        }
        if let Some(m) = z.find("min_thickness").and_then(|m| m.num(0)) {
            e.insert("min_width".into(), json!(mm(m)));
        }
        if let Some(p) = z.find("priority").and_then(|p| p.num(0)) {
            e.insert("priority".into(), json!(p as i64));
        }
        zones.push(Value::Object(e));
    }
    if teardrops > 0 {
        notes.push(format!("{teardrops} teardrops were left out"));
    }
    if keepouts > 0 {
        notes.push(format!("{keepouts} keepout areas were left out"));
    }
    let cutouts: Vec<Value> = loops
        .iter()
        .skip(1)
        .map(|l| json!({ "layers": copper, "points": l.iter().map(|p| pt(shift(*p))).collect::<Vec<_>>() }))
        .collect();

    let class_of = |net: &str| -> Option<String> {
        assign
            .iter()
            .find(|(pat, _)| glob(pat, net))
            .map(|(_, c)| c.clone())
            .filter(|c| c != "Default")
    };
    let sch_nets: Vec<Value> = pins
        .iter()
        .filter(|(_, p)| p.len() >= 2)
        .map(|(n, p)| {
            let mut e = serde_json::Map::new();
            e.insert("name".into(), json!(n));
            if let Some(c) = class_of(n) {
                e.insert("class".into(), json!(c));
            }
            e.insert("style".into(), json!("label"));
            e.insert("pins".into(), json!(p.iter().collect::<Vec<_>>()));
            Value::Object(e)
        })
        .collect();

    let mut rules = serde_json::Map::new();
    let r = &pro["board"]["design_settings"]["rules"];
    for (ours, theirs) in [
        ("min_track_width", "min_track_width"),
        ("min_clearance", "min_clearance"),
        ("min_copper_to_edge", "min_copper_edge_clearance"),
        ("min_hole_to_hole", "min_hole_to_hole"),
        ("min_drill", "min_through_hole_diameter"),
        ("min_via_diameter", "min_via_diameter"),
        ("min_annular_ring", "min_via_annular_width"),
        ("min_silk_text_height", "min_text_height"),
        ("min_silk_width", "min_text_thickness"),
    ] {
        if let Some(v) = r[theirs].as_f64() {
            rules.insert(ours.into(), json!(mm(v)));
        }
    }
    let mut narrowest: HashMap<String, f64> = HashMap::new();
    for t in &tracks {
        let (Some(net), Some(w)) = (t["net"].as_str(), t["width"].as_str()) else { continue };
        let w: f64 = w.trim_end_matches("mm").parse().unwrap_or(f64::MAX);
        let class = class_of(net).unwrap_or_else(|| "Default".into());
        let e = narrowest.entry(class).or_insert(f64::MAX);
        *e = e.min(w);
    }
    for c in classes.iter_mut() {
        let cname = c["name"].as_str().unwrap_or("").to_string();
        let (Some(n), Some(w)) = (
            narrowest.get(&cname),
            c.get("track_width").and_then(|w| w.as_str()).map(str::to_string),
        ) else {
            continue;
        };
        let w: f64 = w.trim_end_matches("mm").parse().unwrap_or(0.0);
        if *n < w - 1e-9 {
            c["track_width"] = json!(mm(*n));
            notes.push(format!("class {cname}: tracks down to {n} mm, under its {w} mm width, so the class takes {n} mm"));
        }
    }
    let board: BoardFile = serde_json::from_value(json!({
        "name": name,
        "description": format!("imported from KiCad"),
        "outline": { "points": outline.iter().map(|p| pt(shift(*p))).collect::<Vec<_>>() },
        "stackup": Value::Object(stackup),
        "vias": board_vias,
        "rules": Value::Object(rules),
        "netclasses": classes,
    }))
    .map_err(|e| format!("board: {e}"))?;
    let mut layout: LayoutFile = serde_json::from_value(json!({
        "name": name,
        "board": name,
        "schematic": name,
        "footprints": placements,
        "tracks": tracks,
        "vias": vias,
        "zones": zones,
        "cutouts": cutouts,
    }))
    .map_err(|e| format!("layout: {e}"))?;
    let vars = text_variables(&root, &pro);
    let mut skipped = 0;
    for item in root.items().iter().skip(1) {
        let Some(head) = item.head().filter(|h| h.starts_with("gr_")) else { continue };
        let Some(l) = item.find("layer").and_then(|l| l.arg(0)) else { continue };
        if l == "Edge.Cuts" {
            continue;
        }
        if !agentee_core::layout::ART_LAYERS.contains(&l) {
            skipped += 1;
            continue;
        }
        let found = if head == "gr_text" {
            let content = substitute(item.arg(0).unwrap_or(""), &vars);
            text_lines(item, &content)
        } else {
            footprint::graphic(item).into_iter().collect()
        };
        for mut g in found {
            shift_graphic(&mut g, &shift);
            layout.graphics.push(g);
        }
    }
    if skipped > 0 {
        notes.push(format!(
            "{skipped} board graphics on copper, mask and user layers were left out"
        ));
    }
    let schematic: SchematicFile = serde_json::from_value(json!({
        "name": name,
        "board": name,
        "parts": parts,
        "nets": sch_nets,
    }))
    .map_err(|e| format!("schematic: {e}"))?;
    let mut syms = Vec::new();
    for (sname, numbers) in symbols {
        let list: Vec<String> = numbers.into_iter().collect();
        let half = list.len().div_ceil(2);
        let left: Vec<Value> = list[..half].iter().map(|n| json!({ "number": n })).collect();
        let right: Vec<Value> = list[half..].iter().map(|n| json!({ "number": n })).collect();
        let mut body = serde_json::Map::new();
        if !left.is_empty() {
            body.insert("left".into(), json!(left));
        }
        if !right.is_empty() {
            body.insert("right".into(), json!(right));
        }
        let sym: SymbolFile = serde_json::from_value(json!({
            "name": sname,
            "reference": "U",
            "description": "pins of an imported KiCad footprint",
            "bodies": if body.is_empty() { json!([]) } else { json!([Value::Object(body)]) },
        }))
        .map_err(|e| format!("symbol {sname}: {e}"))?;
        syms.push(sym);
    }
    Ok(BoardImport {
        board,
        layout,
        schematic,
        footprints: fp_defs.into_iter().map(|(_, f)| f).collect(),
        symbols: syms,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
  (net 0 "") (net 1 "GND") (net 2 "SIG")
  (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
  (footprint "Lib:R_1206" (layer "B.Cu") (at 10 10 90)
    (property "Reference" "R1" (at 0 0 0) (layer "B.SilkS"))
    (pad "1" smd rect (at -1.55 0 90) (size 1.3 1.75) (layers "B.Cu" "B.Mask") (net 2 "SIG"))
    (pad "2" smd rect (at 1.55 0 90) (size 1.3 1.75) (layers "B.Cu" "B.Mask") (net 1 "GND")))
  (footprint "Lib:TO-92" (layer "F.Cu") (at 5 5 180)
    (property "Reference" "Q1" (at 0 0 0) (layer "F.SilkS"))
    (pad "1" thru_hole rect (at 0 0 180) (size 1.1 1.8) (drill 0.75 (offset 0 0.4)) (layers "*.Cu") (net 2 "SIG"))
    (pad "2" thru_hole oval (at 1.27 0 180) (size 1.1 1.8) (drill 0.75) (layers "*.Cu") (net 1 "GND")))
  (segment (start 5 5) (end 10 8.45) (width 0.25) (layer "F.Cu") (net 2)))"#;

    #[test]
    fn imported_pads_land_where_kicad_puts_them() {
        let b = import_board(BOARD, None, "t").unwrap();
        let place =
            |r: &str| b.layout.footprints.iter().find(|f| f.reference == r).unwrap().clone();
        let fp = |n: &str| b.footprints.iter().find(|f| f.name == n).unwrap().clone();
        let r1 = place("R1");
        assert_eq!(r1.side, Some(agentee_core::layout::BoardSide::Bottom));
        let t = agentee_core::geom::Transform {
            at: r1.at.to_mm(),
            rotation: r1.rotation.unwrap_or(0.0),
            mirror: true,
        };
        let pad1 = fp("R_1206").pads.iter().find(|p| p.number == "1").unwrap().at.to_mm();
        let got = t.apply(pad1);
        assert!((got[0] - 10.0).abs() < 1e-9 && (got[1] - 11.55).abs() < 1e-9, "{got:?}");
        assert!(fp("R_1206").pads.iter().all(|p| p.layers.as_ref().unwrap()[0] == "F.Cu"));
        let q1 = fp("TO-92").pads.iter().find(|p| p.number == "1").unwrap().clone();
        let off = q1.drill_offset.unwrap().to_mm();
        assert!((q1.at.to_mm()[1] - 0.4).abs() < 1e-9 && (off[1] + 0.4).abs() < 1e-9, "{q1:?}");
        let sig = b.schematic.nets.iter().find(|n| n.name == "SIG").unwrap();
        assert_eq!(sig.pins, vec!["Q1.1".to_string(), "R1.1".to_string()]);
        assert_eq!(b.layout.tracks.len(), 1);
    }

    #[test]
    fn arcs_turn_the_short_way_through_their_mid_point() {
        let pts = arc_points([1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]);
        assert!(pts.iter().all(|p| p[1] >= -1e-9));
        assert!(glob("SPI_*", "SPI_CLK") && !glob("SPI_*", "I2C"));
        assert_eq!(rotate_kicad([1.0, 0.0], 90.0).map(|v| v.round()), [0.0, -1.0]);
    }
}

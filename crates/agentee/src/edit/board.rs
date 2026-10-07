use super::*;
use serde_json::json;

pub const COMMANDS: &[&str] = &["class", "unclass", "via", "unvia", "outline", "cutout", "stackup"];

pub fn command(name: &str) -> Option<Cmd> {
    Some(match name {
        "class" => class,
        "unclass" => unclass,
        "via" => via,
        "unvia" => unvia,
        "outline" => outline,
        "cutout" => cutout,
        "stackup" => stackup,
        _ => return None,
    })
}

pub fn bool_flags(name: &str) -> &'static [&'static str] {
    match name {
        "class" => &["solver-field"],
        _ => &[],
    }
}

pub fn usage(name: &str) -> &'static str {
    match name {
        "class" => {
            "class NAME [--track-width 0.2mm] [--clearance 0.2mm] [--voltage 48VDC|230VAC] [--via std] [--current 1A] [--max-temp-rise 10C] [--impedance 50ohm] [--impedance-tolerance 10%] [--solver field] [--diff-gap 0.15mm] [--coplanar-gap 0.2mm] [--layers F.Cu] [--max-skew 1mm] [--max-uncoupled 1mm] [--neckdown 1mm] [--width LAYER=WIDTH ...] [--description TEXT]"
        }
        "unclass" => "unclass NAME ...",
        "via" => {
            "via NAME [--drill 0.3mm] [--diameter 0.6mm] [--type through|blind|buried|microvia] [--from F.Cu] [--to B.Cu] [--fill filled_capped] [--drill-kind laser] [--stacked] [--skip] [--cost 2.0] [--backdrill-from L] [--backdrill-to L] [--backdrill-diameter 0.4mm]"
        }
        "unvia" => "unvia NAME ...",
        "outline" => "outline [--size W,H] [--corner-radius 1mm] [--origin X,Y] [--point X,Y ...]",
        "cutout" => "cutout [--size W,H] [--origin X,Y] [--point X,Y ...]",
        "stackup" => {
            "stackup [--preset NAME] [--finish ENIG] [--mask-color green] [--silk-color white] [--roughness 1um] [--layer kind:name:thickness ...]"
        }
        _ => "",
    }
}

fn netclass_index(doc: &DocumentMut, name: &str) -> Result<usize, String> {
    doc.get("netclasses")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|t| text(t, "name").as_deref() == Some(name)))
        .ok_or_else(|| {
            let mut names: Vec<String> = doc
                .get("netclasses")
                .and_then(|v| v.as_array_of_tables())
                .map(|a| a.iter().filter_map(|t| text(t, "name")).collect())
                .unwrap_or_default();
            names.sort();
            format!("no netclass `{name}`; the board has {}", names.join(", "))
        })
}

fn width_list(word: &str) -> Result<(String, TValue), String> {
    let (layer, width) = word
        .split_once('=')
        .ok_or_else(|| format!("--width `{word}` is LAYER=WIDTH, like B.Cu=0.16mm"))?;
    Ok((layer.to_string(), length(width, "--width")?))
}

fn class(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let name = o.word("a netclass name")?;
    let mut found = Vec::new();
    for (flag, key) in [
        ("track-width", "track_width"),
        ("clearance", "clearance"),
        ("voltage", "voltage"),
        ("current", "current"),
        ("max-temp-rise", "max_temp_rise"),
        ("impedance", "impedance"),
        ("impedance-tolerance", "impedance_tolerance"),
        ("solver", "solver"),
        ("diff-gap", "diff_gap"),
        ("coplanar-gap", "coplanar_gap"),
        ("max-skew", "max_skew"),
        ("max-uncoupled", "max_uncoupled"),
        ("neckdown", "neckdown"),
    ] {
        if let Some(v) = o.take(flag) {
            if key == "voltage" {
                agentee_core::insulation::Voltage::parse(&v)?;
                found.push((key, toml_edit::Value::from(v).into()));
                continue;
            }
            found.push((key, value(key, &v)?));
        }
    }
    let via = o.words("via")?;
    let layers = o.words("layers")?;
    let width_words = o.words("width")?;
    let description = o.words("description")?.join(" ");
    let solver_field = o.flag("solver-field");
    o.done("class", 0)?;
    let doc = s.read(path)?;
    let board_name = doc.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    for l in &layers {
        s.check_layer(Some(&board_name), l)?;
    }
    let vias = if via.is_empty() { Vec::new() } else { s.vias(Some(&board_name))? };
    for v in &via {
        if !vias.iter().any(|k| k == v) {
            return Err(format!("no via `{v}` on the board; it has {}", vias.join(", ")));
        }
    }
    let existing = netclass_index(doc, &name);
    let mut table = match existing {
        Ok(i) => doc
            .get("netclasses")
            .and_then(|v| v.as_array_of_tables())
            .and_then(|a| a.get(i))
            .cloned()
            .ok_or("no such netclass")?,
        Err(_) => {
            let mut t = Table::new();
            set(&mut t, "name", name.clone());
            t
        }
    };
    if !description.is_empty() {
        set(&mut table, "description", description);
    }
    for (key, v) in found {
        set(&mut table, key, v);
    }
    if !via.is_empty() {
        if via.len() == 1 {
            set(&mut table, "via", via[0].clone());
        } else {
            let mut a = Array::new();
            for v in &via {
                a.push(v.as_str());
            }
            set(&mut table, "via", TValue::Array(a));
        }
    }
    if !layers.is_empty() {
        let mut a = Array::new();
        for l in &layers {
            a.push(l.as_str());
        }
        set(&mut table, "layers", TValue::Array(a));
    }
    if solver_field {
        set(&mut table, "solver", "field");
    }
    if !width_words.is_empty() {
        let mut inline = toml_edit::InlineTable::new();
        for w in &width_words {
            let (layer, v) = width_list(w)?;
            inline.insert(&layer, v);
        }
        set(&mut table, "widths", TValue::InlineTable(inline));
    }
    let doc = s.doc(path)?;
    match existing {
        Ok(i) => {
            let a = doc.get_mut("netclasses").and_then(|v| v.as_array_of_tables_mut());
            if let Some(a) = a {
                a.replace(i, table);
            }
        }
        Err(_) => insert(doc, "netclasses", table),
    }
    Ok(Report { log: vec![format!("netclass {name}")], facts: vec![json!({ "netclass": name })] })
}

fn unclass(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let names = o.rest();
    o.done("unclass", 0)?;
    let used: Vec<String> = s
        .docs
        .iter()
        .filter(|(p, _)| Kind::of(p) == Some(Kind::Schematic))
        .flat_map(|(_, d)| {
            d.get("nets")
                .and_then(|v| v.as_array_of_tables())
                .map(|a| a.iter().filter_map(|t| text(t, "class")).collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .collect();
    let mut log = Vec::new();
    for name in names {
        let doc = s.doc(path)?;
        netclass_index(doc, &name)?;
        if name == "Default" {
            return Err("the Default netclass is the fallback, it cannot go".into());
        }
        if let Some(a) = doc.get_mut("netclasses").and_then(|v| v.as_array_of_tables_mut()) {
            a.retain(|t| text(t, "name").as_deref() != Some(name.as_str()));
        }
        let sheets: Vec<PathBuf> =
            s.docs.keys().filter(|p| Kind::of(p) == Some(Kind::Schematic)).cloned().collect();
        for sheet in sheets {
            if let Some(a) = s.doc(&sheet)?.get_mut("nets").and_then(|v| v.as_array_of_tables_mut())
            {
                for t in a.iter_mut() {
                    if text(t, "class").as_deref() == Some(name.as_str()) {
                        t.remove("class");
                    }
                }
            }
        }
        log.push(format!("removed netclass {name}"));
    }
    let _ = used;
    Ok(Report { log, facts: Vec::new() })
}

fn via(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let name = o.word("a via name")?;
    let drill = o.typed("drill")?;
    let diameter = o.typed("diameter")?;
    let kind = o.take("type");
    let from = o.take("from");
    let to = o.take("to");
    let fill = o.take("fill");
    let drill_kind = o.take("drill-kind");
    let stacked = o.flag("stacked");
    let skip = o.flag("skip");
    let cost = o.take("cost");
    let bd_from = o.take("backdrill-from");
    let bd_to = o.take("backdrill-to");
    let bd_dia = o.typed("backdrill-diameter")?;
    o.done("via", 0)?;
    let board_name = {
        let doc = s.read(path)?;
        doc.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string()
    };
    let layers = s.layers(Some(&board_name))?;
    for l in [&from, &to].into_iter().flatten() {
        if !layers.iter().any(|k| k == l) {
            return Err(format!("`{l}` is not a copper layer; it has {}", layers.join(", ")));
        }
    }
    let mut t = Table::new();
    set(&mut t, "name", name.clone());
    match drill {
        Some(v) => set(&mut t, "drill", v),
        None => return Err("via: --drill is required".into()),
    }
    match diameter {
        Some(v) => set(&mut t, "diameter", v),
        None => return Err("via: --diameter is required".into()),
    }
    if let Some(k) = kind {
        set(&mut t, "type", k);
    }
    if let Some(f) = from {
        set(&mut t, "from", f);
    }
    if let Some(layer) = to {
        set(&mut t, "to", layer);
    }
    if let Some(f) = fill {
        set(&mut t, "fill", f);
    }
    if let Some(k) = drill_kind {
        set(&mut t, "drill_kind", k);
    }
    if stacked {
        set(&mut t, "stacked", true);
    }
    if skip {
        set(&mut t, "skip", true);
    }
    if let Some(c) = cost {
        set(&mut t, "cost", value("cost", &c)?);
    }
    if bd_from.is_some() || bd_to.is_some() || bd_dia.is_some() {
        let mut b = Table::new();
        if let Some(f) = bd_from {
            set(&mut b, "from", f);
        }
        if let Some(t) = bd_to {
            set(&mut b, "to", t);
        }
        if let Some(d) = bd_dia {
            set(&mut b, "diameter", d);
        }
        t["backdrill"] = Item::Table(b);
    }
    let doc = s.doc(path)?;
    let existing = doc
        .get("vias")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|v| text(v, "name").as_deref() == Some(name.as_str())));
    match existing {
        Some(i) => {
            if let Some(a) = doc.get_mut("vias").and_then(|v| v.as_array_of_tables_mut())
                && let Some(slot) = a.get_mut(i)
            {
                *slot = t;
            }
        }
        None => insert(doc, "vias", t),
    }
    Ok(Report { log: vec![format!("via {name}")], facts: vec![json!({ "via": name })] })
}

fn unvia(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let names = o.rest();
    o.done("unvia", 0)?;
    let mut log = Vec::new();
    for name in names {
        let doc = s.doc(path)?;
        let a = doc
            .get_mut("vias")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("this board has no vias")?;
        let before = a.len();
        a.retain(|t| text(t, "name").as_deref() != Some(name.as_str()));
        if a.len() == before {
            let have: Vec<String> = s
                .read(path)?
                .get("vias")
                .and_then(|v| v.as_array_of_tables())
                .map(|a| a.iter().filter_map(|t| text(t, "name")).collect())
                .unwrap_or_default();
            return Err(format!("no via `{name}`; it has {}", have.join(", ")));
        }
        log.push(format!("removed via {name}"));
    }
    Ok(Report { log, facts: Vec::new() })
}

fn shape(o: &mut Opts, cmd: &str) -> Result<(Table, Vec<String>), String> {
    let size = o.at("size")?;
    let origin = o.at("origin")?;
    let radius = o.typed("corner-radius")?;
    let pts = o.words("point")?;
    o.done(cmd, 0)?;
    if size.is_none() && pts.is_empty() && radius.is_none() && origin.is_none() {
        return Err(format!("{cmd}: give --size W,H or --point X,Y ..."));
    }
    let mut t = Table::new();
    if let Some(v) = size {
        set(&mut t, "size", v);
    }
    if let Some(v) = origin {
        set(&mut t, "origin", v);
    }
    if let Some(v) = radius {
        set(&mut t, "corner_radius", v);
    }
    if !pts.is_empty() {
        set(&mut t, "points", points(&pts, "--point")?);
    }
    let log = vec![format!("{cmd} set")];
    Ok((t, log))
}

fn outline(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let (t, log) = shape(&mut o, "outline")?;
    let mut table = table_of(s.read(path)?.get("outline"))?;
    for (k, v) in t.iter() {
        table[k] = v.clone();
    }
    s.doc(path)?["outline"] = Item::Table(table);
    Ok(Report { log, facts: Vec::new() })
}

fn cutout(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let layers = o.words("layers")?;
    let (t, mut log) = shape(&mut o, "cutout")?;
    if layers.is_empty() {
        return Err("cutout: --layers F.Cu,In1.Cu".into());
    }
    let doc = s.doc(path)?;
    let mut outline = table_of(doc.get("outline"))?;
    let mut table = t;
    let mut a = Array::new();
    for l in &layers {
        a.push(l.as_str());
    }
    set(&mut table, "layers", TValue::Array(a));
    match outline.get_mut("cutouts").and_then(|v: &mut Item| v.as_array_of_tables_mut()) {
        Some(list) => list.push(table),
        None => {
            let mut a = ArrayOfTables::new();
            a.push(table);
            outline["cutouts"] = Item::ArrayOfTables(a);
        }
    }
    s.doc(path)?["outline"] = Item::Table(outline);
    log.push(format!("cutout kept off {}", layers.join(", ")));
    Ok(Report { log, facts: Vec::new() })
}

fn stackup(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let preset = o.take("preset");
    let finish = o.take("finish");
    let mask_color = o.take("mask-color");
    let silk_color = o.take("silk-color");
    let roughness = o.typed("roughness")?;
    let layers = o.words("layer")?;
    o.done("stackup", 0)?;
    if preset.is_none() && layers.is_empty() && finish.is_none() && mask_color.is_none() {
        return Err(
            "stackup: give --preset NAME (see `agentee stackups`) or --layer kind:name:thickness"
                .into(),
        );
    }
    if let Some(p) = &preset {
        if agentee_core::board::stackup_preset(p).is_none() {
            let near = agentee_core::stackups::suggest_stackup_presets(p, 5);
            return Err(format!("no stackup preset `{p}`; close: {}", near.join(", ")));
        }
    }
    let mut t = table_of(s.doc(path)?.get("stackup"))?;
    if let Some(p) = preset {
        set(&mut t, "preset", p);
        t.remove("layers");
    }
    if let Some(f) = finish {
        set(&mut t, "finish", f);
    }
    if let Some(c) = mask_color {
        set(&mut t, "mask_color", c);
    }
    if let Some(c) = silk_color {
        set(&mut t, "silk_color", c);
    }
    if let Some(r) = roughness {
        set(&mut t, "roughness", r);
    }
    if !layers.is_empty() {
        t.remove("preset");
        for spec in &layers {
            let mut it = spec.split(':');
            let kind = it.next().unwrap_or_default().to_string();
            let name = it.next().unwrap_or_default().to_string();
            let thickness = it.next().unwrap_or_default().to_string();
            if kind.is_empty() || name.is_empty() || thickness.is_empty() {
                return Err(format!(
                    "--layer `{spec}` is kind:name:thickness, like copper:F.Cu:0.035mm"
                ));
            }
            if !matches!(kind.as_str(), "silk" | "paste" | "mask" | "copper" | "core" | "prepreg") {
                return Err(format!(
                    "`{kind}` is not a layer kind: silk, paste, mask, copper, core, prepreg"
                ));
            }
            let mut l = Table::new();
            set(&mut l, "kind", kind);
            set(&mut l, "name", name);
            set(&mut l, "thickness", length(&thickness, "--layer")?);
            insert(s.doc(path)?, "layers", l);
        }
    }
    s.doc(path)?["stackup"] = Item::Table(t);
    Ok(Report { log: vec!["stackup set".into()], facts: Vec::new() })
}

use super::*;
use agentee_core::schematic::GRID_MM;
use serde_json::json;

pub const COMMANDS: &[&str] =
    &["add", "remove", "move", "set", "net", "connect", "disconnect", "nc", "unnc", "note"];

pub fn command(name: &str) -> Option<Cmd> {
    Some(match name {
        "add" => add,
        "remove" => remove,
        "move" => move_part,
        "set" => set_part,
        "net" | "wire" => net,
        "connect" => connect,
        "disconnect" => disconnect,
        "nc" => no_connect,
        "unnc" => un_no_connect,
        "note" => note,
        _ => return None,
    })
}

pub fn bool_flags(name: &str) -> &'static [&'static str] {
    match name {
        "add" => &["mirror", "dnp"],
        "set" => &["mirror", "dnp", "clear"],
        "net" | "wire" => &["label", "power"],
        _ => &[],
    }
}

pub fn usage(name: &str) -> &'static str {
    match name {
        "add" => {
            "add REF SYMBOL [VALUE] [--footprint FP] [--at X,Y] [--rotation 90] [--mirror] [--unit 2] [--dnp] [--field key=value]"
        }
        "remove" => "remove REF ...  or  remove net NAME ...",
        "move" => "move REF X,Y [--rotation 90] [--mirror] [--unit 2]",
        "set" => {
            "set REF [VALUE] [--value V] [--footprint FP] [--unit 2] [--dnp] [--clear dnp,footprint] [--field k=v]"
        }
        "net" | "wire" => {
            "net NAME [REF.PIN ...] [--class Power] [--style power|label] [--sheet NAME]"
        }
        "connect" => "connect REF.PIN ... NET",
        "disconnect" => "disconnect REF.PIN ...",
        "nc" => "nc REF.PIN ...",
        "unnc" => "unnc REF.PIN ...",
        "note" => "note TEXT ...",
        _ => "",
    }
}

fn part_index(doc: &DocumentMut, reference: &str) -> Result<usize, String> {
    doc.get("parts")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|t| text(t, "ref").as_deref() == Some(reference)))
        .ok_or(format!("no part `{reference}` in this schematic"))
}

fn refs(doc: &DocumentMut) -> Vec<String> {
    doc.get("parts")
        .and_then(|v| v.as_array_of_tables())
        .map(|a| a.iter().filter_map(|t| text(t, "ref")).collect())
        .unwrap_or_default()
}

fn next_ref(doc: &DocumentMut, symbol: &str) -> String {
    let prefix = doc
        .get("parts")
        .and_then(|v| v.as_array_of_tables())
        .map(|a| {
            a.iter()
                .filter_map(|t| text(t, "symbol"))
                .find(|s| s == symbol)
                .map(|s| s.trim_end_matches(|c: char| !c.is_ascii_digit()).to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let prefix = if prefix.is_empty() { "U".to_string() } else { prefix };
    let used: Vec<u32> = refs(doc)
        .iter()
        .filter_map(|r| r.strip_prefix(&prefix).map(|n| n.parse::<u32>().ok()).flatten())
        .collect();
    format!("{prefix}{}", (1..).find(|n| !used.contains(n)).unwrap_or(1))
}

fn place(doc: &DocumentMut) -> [f64; 2] {
    let mut bounds = agentee_core::graphic::Bounds::EMPTY;
    for t in doc.get("parts").and_then(|v| v.as_array_of_tables()).into_iter().flatten() {
        if let Some(at) = table_point(t, "at") {
            bounds.add(at);
        }
    }
    if bounds.is_empty() {
        return [round(GRID_MM * 4.0), 0.0];
    }
    [round(bounds.max[0] + GRID_MM * 4.0), round(bounds.min[1])]
}

fn round(v: f64) -> f64 {
    (v / GRID_MM).round() * GRID_MM
}

fn grid_point(v: &TValue) -> (TValue, bool) {
    let raw: Vec<f64> = v
        .as_array()
        .map(|a| a.iter().map(|v| v.as_float().unwrap_or(0.0)).collect())
        .unwrap_or_default();
    let [x, y] = raw[..] else { return (v.clone(), false) };
    let (sx, sy) = (round(x), round(y));
    (tv(point_value([sx, sy])), (sx - x).abs() > 1e-9 || (sy - y).abs() > 1e-9)
}

fn point_value(at: [f64; 2]) -> TValue {
    let mut a = Array::new();
    a.push(at[0]);
    a.push(at[1]);
    TValue::Array(a)
}

fn split_field(f: &str) -> Result<(String, String), String> {
    let (k, v) = f.split_once('=').ok_or_else(|| format!("--field `{f}` wants key=value"))?;
    Ok((k.to_string(), v.to_string()))
}

fn add(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let first = o.word("REF SYMBOL [VALUE], like R1 R 10k")?;
    let second = o.pos.first().cloned();
    let (reference, symbol, value) = match second {
        None => {
            let doc = s.read(path)?;
            let reference = next_ref(doc, &first);
            (reference, first, String::new())
        }
        Some(second) => {
            o.pos.remove(0);
            if first.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "`{first}` looks like a value; `add` wants REF SYMBOL [VALUE]"
                ));
            }
            let value = o.word("a value, or --value")?;
            (first, second, value)
        }
    };
    let footprint = o.take("footprint");
    let sym = resolve_symbol(s, &symbol, footprint.as_deref())?;
    let doc = s.read(path)?;
    if refs(doc).contains(&reference) {
        return Err(format!("`{reference}` is already a part here"));
    }
    let at = o.at("at")?.unwrap_or_else(|| tv(point_value(place(doc))));
    let (at, snapped) = grid_point(&at);
    let rotation = o.typed("rotation")?;
    let mirror = o.flag("mirror");
    let unit = o.take("unit");
    let dnp = o.flag("dnp");
    let mut fields = Vec::new();
    while let Some(f) = o.take("field") {
        fields.push(split_field(&f)?);
    }
    o.done("add", 0)?;
    let mut t = Table::new();
    set(&mut t, "ref", reference.clone());
    set(&mut t, "symbol", sym.name.clone());
    set(&mut t, "at", at);
    if !value.is_empty() {
        set(&mut t, "value", value);
    }
    if let Some(f) = &footprint {
        set(&mut t, "footprint", f.clone());
    }
    if let Some(r) = rotation {
        set(&mut t, "rotation", r);
    }
    if mirror {
        set(&mut t, "mirror", true);
    }
    if let Some(u) = unit {
        set(&mut t, "unit", u.parse::<i64>().map_err(|_| format!("--unit `{u}` is not a number"))?);
    }
    if dnp {
        set(&mut t, "dnp", true);
    }
    if !fields.is_empty() {
        let mut inline = toml_edit::InlineTable::new();
        for (k, v) in &fields {
            inline.insert(k, TValue::from(v.clone()));
        }
        set(&mut t, "fields", TValue::InlineTable(inline));
    }
    let doc = s.doc(path)?;
    insert(doc, "parts", t);
    let pins: Vec<String> = sym.pins.iter().map(|p| format!("{reference}.{}", p.number)).collect();
    let at = show_point(doc, &reference);
    Ok(Report {
        log: vec![format!(
            "added {reference} ({}){snap_note}",
            sym.name,
            snap_note = if snapped { " on the 1.27mm grid" } else { "" }
        )],
        facts: vec![json!({
            "part": reference,
            "symbol": sym.name,
            "footprint": footprint.or(sym.footprint.clone()).unwrap_or_default(),
            "at": at,
            "pins": pins,
        })],
    })
}

fn short(s: &str) -> &str {
    s.rsplit(':').next().unwrap_or(s)
}

fn resolve_symbol(
    s: &Session,
    symbol: &str,
    footprint: Option<&str>,
) -> Result<agentee_core::symbol::Symbol, String> {
    if let Ok(sym) = s.symbol(symbol) {
        return Ok(sym.clone());
    }
    let Some(fp) = footprint else {
        return Err(format!("no symbol `{symbol}` in the project"));
    };
    let mut hits =
        s.project.symbols.iter().filter(|e| e.item.footprint.as_deref() == Some(short(fp)));
    let hit = hits.next().ok_or(format!(
        "no symbol named `{symbol}` and no symbol in the project uses footprint `{fp}`"
    ))?;
    if hits.next().is_some() {
        return Err(format!("several symbols use footprint `{fp}`; name one of them"));
    }
    Ok(hit.item.clone())
}

fn show_point(doc: &DocumentMut, reference: &str) -> Value {
    doc.get("parts")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().find(|t| text(t, "ref").as_deref() == Some(reference)))
        .and_then(|t| table_point(t, "at"))
        .map(|p| json!([p[0], p[1]]))
        .unwrap_or(Value::Null)
}

fn remove(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let words = o.rest();
    o.done("remove", 0)?;
    if words.is_empty() {
        return Err("remove: name a part or a net".into());
    }
    let mut log = Vec::new();
    let mut facts = Vec::new();
    for w in words {
        let doc = s.doc(path)?;
        if doc
            .get("nets")
            .and_then(|v| v.as_array_of_tables())
            .is_some_and(|a| a.iter().any(|t| text(t, "name").as_deref() == Some(w.as_str())))
        {
            if let Some(a) = doc.get_mut("nets").and_then(|v| v.as_array_of_tables_mut()) {
                a.retain(|t| text(t, "name").as_deref() != Some(w.as_str()));
            }
            log.push(format!("removed net {w}"));
            facts.push(json!({ "net": w, "removed": true }));
            continue;
        }
        let doc = s.doc(path)?;
        if doc
            .get("parts")
            .and_then(|v| v.as_array_of_tables())
            .is_some_and(|a| a.iter().any(|t| text(t, "ref").as_deref() == Some(w.as_str())))
        {
            if let Some(a) = doc.get_mut("parts").and_then(|v| v.as_array_of_tables_mut()) {
                a.retain(|t| text(t, "ref").as_deref() != Some(w.as_str()));
            }
            if let Some(a) = doc.get_mut("no_connect").and_then(|v| v.as_array_mut()) {
                a.retain(|p| !p.as_str().is_some_and(|p| p.starts_with(&format!("{w}."))));
            }
            let doc = s.doc(path)?;
            let names: Vec<String> = doc
                .get("nets")
                .and_then(|v| v.as_array_of_tables())
                .map(|a| a.iter().filter_map(|t| text(t, "name")).collect())
                .unwrap_or_default();
            for n in names {
                if let Some(a) = s
                    .doc(path)?
                    .get_mut("nets")
                    .and_then(|v| v.as_array_of_tables_mut())
                    .and_then(|a| {
                        a.iter_mut().find(|t| text(t, "name").as_deref() == Some(n.as_str()))
                    })
                    .and_then(|t| t.get_mut("pins"))
                    .and_then(|v| v.as_array_mut())
                {
                    a.retain(|p| !p.as_str().is_some_and(|p| p.starts_with(&format!("{w}."))));
                }
            }
            log.push(format!("removed {w} and its pins"));
            facts.push(json!({ "part": w, "removed": true }));
            continue;
        }
        if doc
            .get("no_connect")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.iter().any(|p| p.as_str() == Some(w.as_str())))
        {
            if let Some(a) = s.doc(path)?.get_mut("no_connect").and_then(|v| v.as_array_mut()) {
                a.retain(|p| p.as_str() != Some(w.as_str()));
            }
            log.push(format!("{w} is no longer marked no-connect"));
            facts.push(json!({ "pin": w, "removed": true }));
            continue;
        }
        return Err(format!("remove: nothing named `{w}` in this schematic"));
    }
    Ok(Report { log, facts })
}

fn move_part(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let reference = o.word("a reference")?;
    let given = match o.pos.is_empty() {
        true => None,
        false => Some(o.word("X,Y")?),
    };
    let typed = given.as_deref().map(|v| point(v, "move")).transpose()?;
    let at = o.at("at")?.or(typed);
    let at = at.map(|v| {
        let (v, _) = grid_point(&v);
        v
    });
    let rotation = o.typed("rotation")?;
    let mirror = o.flag("mirror");
    let unit = o.take("unit");
    o.done("move", 0)?;
    let doc = s.doc(path)?;
    let i = part_index(doc, &reference)?;
    let t = doc
        .get_mut("parts")
        .and_then(|v| v.as_array_of_tables_mut())
        .and_then(|a| a.get_mut(i))
        .ok_or("no such part")?;
    if let Some(at) = at {
        set(t, "at", at);
    }
    if let Some(r) = rotation {
        set(t, "rotation", r);
    }
    if mirror {
        set(t, "mirror", true);
    }
    if let Some(u) = unit {
        set(t, "unit", u.parse::<i64>().map_err(|_| format!("--unit `{u}` is not a number"))?);
    }
    Ok(Report::log(format!("moved {reference}")))
}

fn set_part(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let reference = o.word("a reference")?;
    let positional = o.rest();
    let value = o.take("value");
    let footprint = o.take("footprint");
    let unit = o.take("unit");
    let dnp = o.flag("dnp");
    let mirror = o.flag("mirror");
    let clear = o.take("clear").unwrap_or_default();
    let mut fields = Vec::new();
    while let Some(f) = o.take("field") {
        fields.push(split_field(&f)?);
    }
    o.done("set", 0)?;
    let doc = s.doc(path)?;
    let i = part_index(doc, &reference)?;
    let t = doc
        .get_mut("parts")
        .and_then(|v| v.as_array_of_tables_mut())
        .and_then(|a| a.get_mut(i))
        .ok_or("no such part")?;
    let mut log = Vec::new();
    if let Some(v) = value.or(positional.into_iter().next()) {
        set(t, "value", v);
        log.push(format!("{reference} value set"));
    }
    if let Some(f) = footprint {
        set(t, "footprint", f);
        log.push(format!("{reference} footprint set"));
    }
    if let Some(u) = unit {
        set(t, "unit", u.parse::<i64>().map_err(|_| format!("--unit `{u}` is not a number"))?);
    }
    if dnp {
        set(t, "dnp", true);
    }
    if mirror {
        set(t, "mirror", true);
    }
    for k in clear.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        if !matches!(k, "dnp" | "mirror" | "footprint" | "unit" | "rotation" | "fields") {
            return Err(format!("--clear `{k}` is not a part field"));
        }
        t.remove(k);
        log.push(format!("{reference} cleared {k}"));
    }
    for (k, v) in fields {
        let Some(f) = t.get_mut("fields").and_then(|v: &mut Item| v.as_inline_table_mut()) else {
            let mut inline = toml_edit::InlineTable::new();
            inline.insert(&k, TValue::from(v.clone()));
            set(t, "fields", TValue::InlineTable(inline));
            continue;
        };
        f.insert(&k, TValue::from(v.clone()));
    }
    Ok(Report { log, facts: Vec::new() })
}

struct Joined {
    added: Vec<String>,
    moved: Vec<String>,
}

fn net(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let name = o.word("a net name")?;
    let pins = o.rest();
    let class = o.take("class");
    let mut style =
        o.take("style").or_else(|| if o.flag("power") { Some("power".into()) } else { None });
    if o.flag("label") {
        style = Some("label".into());
    }
    let sheet = o.take("sheet");
    o.done("net", 0)?;
    let board = s.schematic_board(path);
    if let Some(c) = &class {
        s.check_class(board.as_deref(), c)?;
    }
    let sheets: Vec<PathBuf> = s
        .docs
        .keys()
        .filter(|p| Kind::of(p) == Some(Kind::Schematic))
        .filter(|p| {
            s.read(p)
                .ok()
                .and_then(|d| d.get("nets"))
                .and_then(|v| v.as_array_of_tables())
                .is_some_and(|a| {
                    a.iter().any(|t| text(t, "name").as_deref() == Some(name.as_str()))
                })
        })
        .cloned()
        .collect();
    let here = match &sheet {
        Some(sheet) => s.target(Kind::Schematic, sheet)?,
        None if !sheets.is_empty() => sheets[0].clone(),
        None => path.to_path_buf(),
    };
    let existing = s
        .read(path)?
        .get("nets")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| a.iter().position(|t| text(t, "name").as_deref() == Some(name.as_str())));
    let index = if existing.is_some() {
        existing
    } else {
        s.read(&here)?
            .get("nets")
            .and_then(|v| v.as_array_of_tables())
            .and_then(|a| a.iter().position(|t| text(t, "name").as_deref() == Some(name.as_str())))
    };
    let mut joined = Joined { added: Vec::new(), moved: Vec::new() };
    let mut here_pins: Vec<String> = index
        .and_then(|i| {
            let doc = s.read(&here).ok()?;
            let a = doc.get("nets")?.as_array_of_tables()?;
            a.get(i).map(texts_pin)
        })
        .unwrap_or_default();
    for pin in pins {
        let num = s.resolve_pin(&here, &pin)?;
        let label = label_of(&pin, &num);
        if here_pins.contains(&label) {
            continue;
        }
        if let Some((old, _)) = owner(s.read(path)?, &label) {
            let _ = old;
            s.detach(&here, &old, &label)?;
            joined.moved.push(format!("{label} left net {old}"));
        }
        here_pins.push(label.clone());
        joined.added.push(label);
    }
    let log_line = format!("{} now joins {}", name, joined.added.join(", "));
    match index {
        Some(i) => {
            let doc = s.doc(&here)?;
            let t = doc
                .get_mut("nets")
                .and_then(|v| v.as_array_of_tables_mut())
                .and_then(|a| a.get_mut(i))
                .ok_or("no such net")?;
            if let Some(c) = &class {
                set(t, "class", c.clone());
            }
            if let Some(st) = &style {
                set(t, "style", st.clone());
            }
            let a = t
                .get_mut("pins")
                .and_then(|v: &mut Item| v.as_array_mut())
                .ok_or("this net has no pins")?;
            push_new(a, "pin", &joined.added)?;
        }
        None => {
            let doc = s.doc(&here)?;
            let mut t = Table::new();
            set(&mut t, "name", name.clone());
            if let Some(c) = &class {
                set(&mut t, "class", c.clone());
            }
            if let Some(st) = &style {
                set(&mut t, "style", st.clone());
            }
            let mut a = Array::new();
            for p in &joined.added {
                a.push(p.as_str());
            }
            set(&mut t, "pins", TValue::Array(a));
            insert(doc, "nets", t);
        }
    }
    let mut log = joined.moved.clone();
    if !joined.added.is_empty() {
        log.push(log_line);
    }
    Ok(Report {
        log,
        facts: vec![json!({ "net": name, "pins": joined.added, "moved_from": joined.moved })],
    })
}

fn label_of(given: &str, number: &str) -> String {
    let reference = given.split('.').next().unwrap_or(given);
    format!("{reference}.{number}")
}

fn owner(doc: &DocumentMut, pin: &str) -> Option<(String, String)> {
    doc.get("nets")
        .and_then(|v| v.as_array_of_tables())
        .and_then(|a| {
            a.iter()
                .find(|t| texts(t, "pins").iter().any(|p| p == pin))
                .and_then(|t| text(t, "name"))
        })
        .map(|n| (n, pin.to_string()))
}

fn connect(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let mut words = o.rest();
    o.done("connect", 0)?;
    let name = words.pop().ok_or("connect: give the net name last, `connect U1.3 VCC`")?;
    let mut log = Vec::new();
    for pin in words {
        let one = net(s, path, Opts { pos: vec![name.clone(), pin], flags: BTreeMap::new() })?;
        log.extend(one.log);
    }
    Ok(Report { log, facts: Vec::new() })
}

fn disconnect(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let pins = o.rest();
    o.done("disconnect", 0)?;
    if pins.is_empty() {
        return Err("disconnect: name the pins to unwire".into());
    }
    let mut log = Vec::new();
    for pin in pins {
        let num = s.resolve_pin(path, &pin)?;
        let label = label_of(&pin, &num);
        for source in [path.to_path_buf()] {
            let names = nets_of(s.read(&source)?);
            for n in names {
                if let Some(a) = s
                    .doc(&source)?
                    .get_mut("nets")
                    .and_then(|v| v.as_array_of_tables_mut())
                    .and_then(|a| {
                        a.iter_mut().find(|t| text(t, "name").as_deref() == Some(n.as_str()))
                    })
                    .and_then(|t| t.get_mut("pins"))
                    .and_then(|v| v.as_array_mut())
                {
                    a.retain(|v| v.as_str() != Some(label.as_str()));
                }
            }
        }
        let doc = s.doc(path)?;
        if let Some(a) = doc.get_mut("no_connect").and_then(|v| v.as_array_mut()) {
            a.retain(|v| v.as_str() != Some(label.as_str()));
        }
        log.push(format!("unwired {label}"));
    }
    Ok(Report { log, facts: Vec::new() })
}

fn texts_pin(t: &Table) -> Vec<String> {
    texts(t, "pins")
}

fn nets_of(doc: &DocumentMut) -> Vec<String> {
    doc.get("nets")
        .and_then(|v| v.as_array_of_tables())
        .map(|a| a.iter().filter_map(|t| text(t, "name")).collect())
        .unwrap_or_default()
}

fn no_connect(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let pins = o.rest();
    o.done("nc", 0)?;
    if pins.is_empty() {
        return Err("nc: name the pins to leave open".into());
    }
    let mut log = Vec::new();
    for pin in pins {
        let num = s.resolve_pin(path, &pin)?;
        let label = label_of(&pin, &num);
        let doc = s.doc(path)?;
        if let Some((name, _)) = owner(doc, &label) {
            s.detach(path, &name, &label)?;
            log.push(format!("{label} left net {name}"));
        }
        let doc = s.doc(path)?;
        let a = arr(doc, "no_connect")?;
        if !a.iter().any(|v| v.as_str() == Some(label.as_str())) {
            a.push(label.as_str());
        }
    }
    Ok(Report { log, facts: Vec::new() })
}

fn un_no_connect(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let pins = o.rest();
    o.done("unnc", 0)?;
    let mut log = Vec::new();
    for pin in pins {
        let num = s.resolve_pin(path, &pin)?;
        let label = label_of(&pin, &num);
        let doc = s.doc(path)?;
        if let Some(a) = doc.get_mut("no_connect").and_then(|v| v.as_array_mut()) {
            let before = a.len();
            a.retain(|v| v.as_str() != Some(label.as_str()));
            if a.len() != before {
                log.push(format!("{label} may be left open"));
            }
        }
    }
    Ok(Report { log, facts: Vec::new() })
}

fn note(s: &mut Session, path: &Path, mut o: Opts) -> Result<Report, String> {
    let words = o.rest();
    o.done("note", 0)?;
    if words.is_empty() {
        return Err("note: say something".into());
    }
    let doc = s.doc(path)?;
    let existing = doc.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let line = words.join(" ");
    let joined = if existing.is_empty() { line.clone() } else { format!("{existing}\n{line}") };
    set(doc, "description", joined);
    Ok(Report::log(format!("noted: {line}")))
}

impl Session {
    pub fn resolve_pin(&mut self, path: &Path, spec: &str) -> Result<String, String> {
        let (reference, wanted) = spec
            .rsplit_once('.')
            .ok_or_else(|| format!("`{spec}` is REF.PIN, like U1.3 or U1.VCC"))?;
        let doc =
            self.docs.get(path).ok_or_else(|| format!("{} is not a schematic", path.display()))?;
        let parts = doc
            .get("parts")
            .and_then(|v| v.as_array_of_tables())
            .ok_or(format!("`{spec}`: this schematic has no parts yet"))?;
        let part = parts
            .iter()
            .find(|t| text(t, "ref").as_deref() == Some(reference))
            .ok_or(format!("no part `{reference}` in this schematic"))?;
        let symbol = text(part, "symbol").unwrap_or_default();
        let unit = part.get("unit").and_then(|v| v.as_integer()).unwrap_or(1).max(1) as u32;
        let sym = self.symbol(&symbol)?;
        let pins: Vec<(&str, &str)> = sym
            .pins
            .iter()
            .filter(|p| p.in_unit(unit))
            .map(|p| (p.number.as_str(), p.name.as_str()))
            .collect();
        if let Some((n, _)) = pins.iter().find(|(n, _)| *n == wanted) {
            return Ok(n.to_string());
        }
        let by_name: Vec<&str> =
            pins.iter().filter(|(_, name)| *name == wanted).map(|(n, _)| *n).collect();
        match by_name.as_slice() {
            [] => Err(format!(
                "`{reference}` ({symbol}) has no pin `{wanted}`; it has {}",
                pins.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>().join(", ")
            )),
            [n] => Ok(n.to_string()),
            many => Err(format!(
                "`{reference}` has {} pins named `{wanted}`, use the number",
                many.len()
            )),
        }
    }

    pub fn detach(&mut self, path: &Path, net: &str, pin: &str) -> Result<(), String> {
        let a = self
            .doc(path)?
            .get_mut("nets")
            .and_then(|v| v.as_array_of_tables_mut())
            .and_then(|a| a.iter_mut().find(|t| text(t, "name").as_deref() == Some(net)))
            .and_then(|t| t.get_mut("pins"))
            .and_then(|v| v.as_array_mut())
            .ok_or(format!("no net `{net}` in {}", path.display()))?;
        a.retain(|v| v.as_str() != Some(pin));
        Ok(())
    }
}

pub fn show(s: &Session, path: &Path) -> Value {
    let doc = &s.docs[path];
    json!({
        "name": doc.get("name").and_then(|v| v.as_str()),
        "board": doc.get("board").and_then(|v| v.as_str()),
        "description": doc.get("description").and_then(|v| v.as_str()),
        "parts": doc.get("parts").and_then(|v| v.as_array_of_tables()).map(|a| a.iter().map(|t| json!({
            "ref": text(t, "ref"),
            "symbol": text(t, "symbol"),
            "value": text(t, "value"),
            "footprint": text(t, "footprint"),
            "at": table_point(t, "at"),
            "rotation": t.get("rotation").and_then(|v| v.as_float()),
            "mirror": t.get("mirror").and_then(|v| v.as_bool()).unwrap_or(false),
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "nets": doc.get("nets").and_then(|v| v.as_array_of_tables()).map(|a| a.iter().map(|t| json!({
            "name": text(t, "name"),
            "class": text(t, "class"),
            "style": text(t, "style"),
            "pins": texts(t, "pins"),
            "wires_drawn": t.get("wires").and_then(|v| v.as_array()).map(|w| w.len()).unwrap_or(0),
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "no_connect": texts(doc, "no_connect"),
        "file": path.display().to_string(),
    })
}

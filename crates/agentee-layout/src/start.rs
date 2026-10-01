use agentee_core::engine::PHASES;
use agentee_core::layout::{LabelFix, LayoutFile};
use agentee_core::place::{self as pl, PlaceOptions, PlaceResult, Placement, TextMove};
use agentee_core::project::LayoutInputs;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reset {
    pub routing: bool,
    pub via_rules: bool,
    pub zones: bool,
    pub labels: bool,
}

pub fn reset(text: &str, r: &Reset) -> Result<String, String> {
    let mut text = text.to_string();
    if r.routing {
        for p in PHASES {
            text = crate::strip_plan(&text, p);
        }
    }
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut drop = Vec::new();
    if r.routing {
        drop.extend(["tracks", "vias"]);
    }
    if r.via_rules {
        drop.extend(["fanouts", "stitching"]);
    }
    if r.zones {
        drop.extend(["zones", "fills"]);
    }
    for key in drop {
        doc.remove(key);
    }
    if r.labels
        && let Some(parts) = doc.get_mut("footprints").and_then(Item::as_array_of_tables_mut)
    {
        for t in parts.iter_mut() {
            let hidden =
                t.get("label").and_then(|l| l.get("hide")).and_then(Item::as_bool).unwrap_or(false);
            t.remove("label");
            if hidden {
                let mut l = toml_edit::InlineTable::new();
                l.insert("hide", true.into());
                t["label"] = value(l);
            }
        }
    }
    Ok(doc.to_string())
}

pub fn starter(
    name: &str,
    board: &agentee_core::board::Board,
    schematic: &agentee_core::schematic::Schematic,
) -> String {
    let mut doc = DocumentMut::new();
    doc["name"] = value(name);
    doc["board"] = value(board.name.as_str());
    doc["schematic"] = value(schematic.name.as_str());
    let copper = board.stackup.copper_names();
    let mut grounds: Vec<&str> =
        schematic.nets.iter().filter(|n| pl::is_ground(&n.name)).map(|n| n.name.as_str()).collect();
    grounds.sort_by_key(|n| {
        std::cmp::Reverse(schematic.nets.iter().find(|x| x.name == *n).map(|x| x.pins.len()))
    });
    let rf: Vec<&str> = schematic
        .nets
        .iter()
        .filter(|n| pl::is_rf_class(board, &n.class))
        .map(|n| n.name.as_str())
        .collect();
    let coplanar = schematic.nets.iter().any(|n| {
        pl::is_rf_class(board, &n.class)
            && board.netclasses.iter().any(|c| c.name == n.class && c.coplanar_gap.is_some())
    });
    let mut zones = ArrayOfTables::new();
    for g in &grounds {
        let mut t = Table::new();
        t["net"] = value(*g);
        let mut layers = Array::new();
        copper.iter().for_each(|l| layers.push(l.as_str()));
        t["layers"] = value(layers);
        zones.push(t);
    }
    if !zones.is_empty() {
        doc["zones"] = Item::ArrayOfTables(zones);
    }
    if let Some(g) = grounds.first().filter(|_| !rf.is_empty()) {
        let mut stitching = ArrayOfTables::new();
        if coplanar {
            let mut t = Table::new();
            t["net"] = value(*g);
            let mut fence = Array::new();
            rf.iter().for_each(|n| fence.push(*n));
            t["fence"] = value(fence);
            stitching.push(t);
        }
        let mut t = Table::new();
        t["net"] = value(*g);
        t["pitch"] = value("2.5mm");
        stitching.push(t);
        doc["stitching"] = Item::ArrayOfTables(stitching);
    }
    doc.to_string()
}

pub fn place_text(
    inputs: &LayoutInputs,
    text: &str,
    opts: &PlaceOptions,
) -> Result<(String, PlaceResult), String> {
    let file: LayoutFile =
        agentee_core::project::parse(text).map_err(|(at, m)| format!("{at}: {m}"))?;
    let board = &inputs.board;
    let mut fast: Vec<String> = file.interfaces.iter().flat_map(|f| f.nets.clone()).collect();
    for pr in &file.pairs {
        fast.push(pr.p.clone());
        fast.push(pr.n.clone());
    }
    let footprints: std::collections::HashMap<&str, &agentee_core::footprint::Footprint> =
        inputs.footprints.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let spec = file.place.clone().unwrap_or_default();
    let outline = board.outline.as_ref().map(|o| o.points()).unwrap_or_default();
    let cutouts: Vec<Vec<[f64; 2]>> = board.cutouts.iter().map(|c| c.points()).collect();
    let dir = inputs.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let mut d = agentee_core::diag::Diags::new(&file.name);
    let (graphics, artwork) = file.artwork_of(&dir, &mut d);
    let input = pl::PlaceInput {
        board,
        outline: &outline,
        cutouts: &cutouts,
        schematic: &inputs.schematic,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: fast,
        heat: pl::thermal_heat(&inputs.sims, &file.name),
        silk: pl::board_silk(&graphics, &artwork),
        texts: pl::movable_texts(&graphics),
    };
    let r = pl::place(&input, opts)?;
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    write_placements(&mut doc, &r.placements)?;
    move_board_texts(&mut doc, &r.texts_moved);
    let placed = doc.to_string();
    let entry = inputs.resolve(&placed)?;
    let (moved, _) = entry.item.settle_labels(board);
    write_labels(&mut doc, &moved)?;
    Ok((doc.to_string(), r))
}

fn parts(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    if doc.get("footprints").and_then(Item::as_array_of_tables).is_none() {
        doc["footprints"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    doc.get_mut("footprints")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| "[[footprints]] is not an array of tables".to_string())
}

fn point(p: [f64; 2]) -> Array {
    let mut a = Array::new();
    a.push(p[0]);
    a.push(p[1]);
    a
}

pub fn write_placements(doc: &mut DocumentMut, placements: &[Placement]) -> Result<(), String> {
    let parts = parts(doc)?;
    for pm in placements {
        let found = parts
            .iter_mut()
            .position(|t| t.get("ref").and_then(Item::as_str) == Some(pm.reference.as_str()));
        let k = match found {
            Some(k) => k,
            None => {
                let mut t = Table::new();
                t["ref"] = value(pm.reference.as_str());
                parts.push(t);
                parts.len() - 1
            }
        };
        let t = parts.get_mut(k).ok_or("could not add a footprint")?;
        t["at"] = value(point(pm.at));
        if pm.rotation != 0.0 {
            t["rotation"] = value(pm.rotation);
        } else {
            t.remove("rotation");
        }
        if pm.bottom {
            t["side"] = value("bottom");
        } else {
            t.remove("side");
        }
        let hidden = t
            .get("label")
            .and_then(Item::as_inline_table)
            .and_then(|l| l.get("hide"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if let Some((at, rotation)) = pm.label {
            let mut label =
                t.get("label").and_then(Item::as_inline_table).cloned().unwrap_or_default();
            label.insert("at", point(at).into());
            if rotation != 0.0 {
                label.insert("rotation", rotation.into());
            } else {
                label.remove("rotation");
            }
            t["label"] = value(label);
        } else if let Some(mut label) = t.get("label").and_then(Item::as_inline_table).cloned() {
            label.remove("at");
            label.remove("rotation");
            if label.is_empty() && !hidden {
                t.remove("label");
            } else {
                t["label"] = value(label);
            }
        }
    }
    Ok(())
}

pub fn write_labels(doc: &mut DocumentMut, moved: &[LabelFix]) -> Result<(), String> {
    if moved.is_empty() {
        return Ok(());
    }
    let parts = parts(doc)?;
    for f in moved {
        let Some(at) = f.at else { continue };
        let Some(t) = parts
            .iter_mut()
            .find(|t| t.get("ref").and_then(Item::as_str) == Some(f.reference.as_str()))
        else {
            continue;
        };
        let mut label = t.get("label").and_then(Item::as_inline_table).cloned().unwrap_or_default();
        label.insert("at", point(at).into());
        if f.rotation != 0.0 {
            label.insert("rotation", f.rotation.into());
        } else {
            label.remove("rotation");
        }
        t["label"] = value(label);
    }
    Ok(())
}

pub fn move_board_texts(doc: &mut DocumentMut, moves: &[TextMove]) {
    let Some(graphics) = doc.get_mut("graphics").and_then(Item::as_array_of_tables_mut) else {
        return;
    };
    for m in moves {
        let found = graphics.iter_mut().find(|t| {
            let at: Vec<f64> = t
                .get("at")
                .and_then(Item::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_float().or(x.as_integer().map(|i| i as f64)))
                        .collect()
                })
                .unwrap_or_default();
            t.get("kind").and_then(Item::as_str) == Some("text")
                && t.get("text").and_then(Item::as_str) == Some(m.text.as_str())
                && t.get("layer").and_then(Item::as_str).unwrap_or("F.SilkS") == m.layer
                && at.len() == 2
                && (at[0] - m.from[0]).abs() < 1e-6
                && (at[1] - m.from[1]).abs() < 1e-6
        });
        if let Some(t) = found {
            t["at"] = value(point(m.to));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = "name = \"x\"\n\n[[footprints]]\nref = \"R1\"\nat = [1, 2]\nlabel = { at = [1, 3], hide = true }\n\n[[footprints]]\nref = \"R2\"\nat = [4, 2]\nlabel = { at = [4, 3] }\n\n[[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\npoints = [[1, 2], [4, 2]]\n\n[[zones]]\nnet = \"GND\"\nlayers = [\"F.Cu\"]\n\n# plan detail\n[[vias]]\nnet = \"A\"\nat = [2, 2]\n# end plan detail\n";

    #[test]
    fn reset_routing_drops_copper_and_plans_but_keeps_zones() {
        let t = reset(LAYOUT, &Reset { routing: true, ..Default::default() }).unwrap();
        assert!(
            !t.contains("[[tracks]]") && !t.contains("[[vias]]") && !t.contains("# plan"),
            "{t}"
        );
        assert!(t.contains("[[zones]]") && t.contains("ref = \"R2\""), "{t}");
    }

    #[test]
    fn reset_labels_keeps_hidden_ones_hidden() {
        let t = reset(LAYOUT, &Reset { labels: true, zones: true, ..Default::default() }).unwrap();
        assert!(t.contains("label = { hide = true }"), "{t}");
        assert!(!t.contains("at = [4, 3]") && !t.contains("[[zones]]"), "{t}");
    }
}

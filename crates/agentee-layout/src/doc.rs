use agentee_core::geom::P;
use agentee_core::layout::{LabelFile, LayoutFile, StitchFile, TrackFile, ViaFile, ZoneFile};
use agentee_core::units::{Length, Point};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, value};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Section {
    #[serde(default)]
    pub tracks: Vec<TrackFile>,
    #[serde(default)]
    pub vias: Vec<ViaFile>,
    #[serde(default)]
    pub zones: Vec<ZoneFile>,
    #[serde(default)]
    pub stitching: Vec<StitchFile>,
}

impl Section {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.vias.is_empty()
            && self.zones.is_empty()
            && self.stitching.is_empty()
    }

    pub fn track(&mut self, net: &str, layer: &str, width: Option<f64>, points: &[P]) {
        self.tracks.push(TrackFile {
            net: net.to_string(),
            layer: layer.to_string(),
            width: width.map(Length::mm),
            points: points.iter().map(|q| Point::mm(q[0], q[1])).collect(),
        });
    }

    pub fn via(&mut self, net: &str, at: P, via: &str) {
        self.vias.push(ViaFile {
            net: net.to_string(),
            at: Point::mm(at[0], at[1]),
            via: Some(via.to_string()),
            count: None,
            pitch: None,
        });
    }

    fn toml(&self) -> String {
        let mut t = String::new();
        for s in &self.stitching {
            let fence: Vec<String> = s.fence.iter().map(|n| format!("\"{n}\"")).collect();
            t += &format!("\n[[stitching]]\nnet = \"{}\"\nfence = [{}]\n", s.net, fence.join(", "));
        }
        for z in &self.zones {
            let layers: Vec<String> = z.layers.iter().map(|l| format!("\"{l}\"")).collect();
            t += &format!("\n[[zones]]\nnet = \"{}\"\nlayers = [{}]\n", z.net, layers.join(", "));
            if let Some(p) = z.priority {
                t += &format!("priority = {p}\n");
            }
            if let Some(o) = &z.outline {
                let pts: Vec<String> = o.iter().map(|q| pt(q.to_mm())).collect();
                t += &format!("outline = [{}]\n", pts.join(", "));
            }
        }
        for tr in &self.tracks {
            let pts: Vec<P> = tr.points.iter().map(|q| q.to_mm()).collect();
            t += &crate::track_toml(&tr.net, &tr.layer, tr.width.map(|w| w.to_mm()), &pts);
        }
        for v in &self.vias {
            t += &crate::via_toml(&v.net, v.at.to_mm(), v.via.as_deref().unwrap_or(""));
        }
        t
    }
}

fn pt(q: P) -> String {
    crate::pt(q)
}

#[derive(Clone)]
pub struct Doc {
    pub base: LayoutFile,
    sections: Vec<(String, Section)>,
    changed: bool,
    version: u64,
    base_version: u64,
    section_versions: std::collections::BTreeMap<String, u64>,
}

fn sections_of(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find("# plan ") {
        let line_end = rest[a..].find('\n').map_or(rest.len(), |k| a + k);
        let name = rest[a + 7..line_end].trim().to_string();
        let end = format!("# end plan {name}\n");
        let Some(b) = rest[line_end..].find(&end).map(|k| line_end + k) else { break };
        out.push((name, rest[(line_end + 1).min(b)..b].to_string()));
        rest = &rest[b + end.len()..];
    }
    out
}

fn strip_all(text: &str) -> String {
    let mut bare = text.to_string();
    for (name, _) in sections_of(text) {
        bare = crate::strip_plan(&bare, &name);
    }
    bare
}

impl Doc {
    pub fn new(text: &str) -> Result<Doc, String> {
        let base: LayoutFile = agentee_core::project::parse(&strip_all(text))
            .map_err(|(at, m)| format!("{at}: {m}"))?;
        let mut sections: Vec<(String, Section)> = Vec::new();
        for (name, body) in sections_of(text) {
            let s: Section =
                toml::from_str(&body).map_err(|e| format!("plan {name}: {}", e.message()))?;
            sections.retain(|(n, _)| *n != name);
            sections.push((name, s));
        }
        Ok(Doc {
            base,
            sections,
            changed: false,
            version: 0,
            base_version: 0,
            section_versions: Default::default(),
        })
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn changed(&mut self) {
        self.changed = true;
        self.version += 1;
        self.base_version += 1;
    }

    fn section_changed(&mut self, name: &str) {
        self.changed = true;
        self.version += 1;
        *self.section_versions.entry(name.to_string()).or_default() += 1;
    }

    pub fn version_except(&self, name: &str) -> u64 {
        self.base_version
            + self.section_versions.iter().filter(|(n, _)| *n != name).map(|(_, v)| v).sum::<u64>()
    }

    pub fn file(&self) -> LayoutFile {
        self.file_without(None)
    }

    pub fn file_without(&self, skip: Option<&str>) -> LayoutFile {
        let mut f = self.base.clone();
        for (name, s) in &self.sections {
            if Some(name.as_str()) == skip {
                continue;
            }
            f.tracks.extend(s.tracks.iter().cloned());
            f.vias.extend(s.vias.iter().cloned());
            f.zones.extend(s.zones.iter().cloned());
            f.stitching.extend(s.stitching.iter().cloned());
        }
        f
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    pub fn strip(&mut self, name: &str) {
        let before = self.sections.len();
        self.sections.retain(|(n, _)| n != name);
        if self.sections.len() != before {
            self.section_changed(name);
        }
    }

    pub fn set(&mut self, name: &str, s: Section) {
        self.sections.retain(|(n, _)| n != name);
        self.sections.push((name.to_string(), s));
        self.section_changed(name);
    }

    pub fn moves(&mut self, moves: &[crate::placement::Move]) {
        for m in moves {
            let Some(f) = self.base.footprints.iter_mut().find(|f| f.reference == m.reference)
            else {
                continue;
            };
            let old_at = f.at.to_mm();
            let old_rot = f.rotation.unwrap_or(0.0);
            f.at = Point::mm(m.at[0], m.at[1]);
            let rot = m.rotation.rem_euclid(360.0);
            f.rotation = (rot.abs() >= 1e-6).then_some(rot);
            f.side = m.bottom.then_some(agentee_core::layout::BoardSide::Bottom);
            if m.hide_label {
                f.label = Some(LabelFile { at: None, rotation: None, size: None, hide: true });
                continue;
            }
            if let Some(label) = f.label.as_mut()
                && let Some(la) = label.at
            {
                let la = la.to_mm();
                let off = agentee_core::geom::rotate(
                    [la[0] - old_at[0], la[1] - old_at[1]],
                    m.rotation - old_rot,
                );
                label.at = Some(Point::mm(
                    ((m.at[0] + off[0]) * 1e4).round() / 1e4,
                    ((m.at[1] + off[1]) * 1e4).round() / 1e4,
                ));
            }
        }
        if !moves.is_empty() {
            self.changed();
        }
    }

    pub fn labels(&mut self, fixes: &[agentee_core::layout::LabelFix]) {
        for fix in fixes {
            let Some(at) = fix.at else { continue };
            let Some(f) = self.base.footprints.iter_mut().find(|f| f.reference == fix.reference)
            else {
                continue;
            };
            let label = f.label.get_or_insert(LabelFile {
                at: None,
                rotation: None,
                size: None,
                hide: false,
            });
            label.at = Some(Point::mm(at[0], at[1]));
            label.rotation = (fix.rotation != 0.0).then_some(fix.rotation);
        }
        if !fixes.is_empty() {
            self.changed();
        }
    }

    pub fn board_texts(&mut self, moves: &[agentee_core::place::TextMove]) {
        for m in moves {
            let found = self.base.graphics.iter_mut().find(|g| {
                g.kind == agentee_core::graphic::GraphicKind::Text
                    && g.text.as_deref() == Some(m.text.as_str())
                    && g.layer.as_deref().unwrap_or("F.SilkS") == m.layer
                    && g.at.is_some_and(|a| {
                        let a = a.to_mm();
                        (a[0] - m.from[0]).abs() < 1e-6 && (a[1] - m.from[1]).abs() < 1e-6
                    })
            });
            if let Some(g) = found {
                g.at = Some(Point::mm(m.to[0], m.to[1]));
            }
        }
        if !moves.is_empty() {
            self.changed();
        }
    }

    pub fn write(&self, original: &str) -> Result<String, String> {
        if !self.changed {
            return Ok(original.to_string());
        }
        let mut doc: DocumentMut = strip_all(original).parse().map_err(|e| format!("{e}"))?;
        let was: LayoutFile = agentee_core::project::parse(&strip_all(original))
            .map_err(|(at, m)| format!("{at}: {m}"))?;
        sync_footprints(&mut doc, &was, &self.base)?;
        sync_graphics(&mut doc, &was, &self.base);
        let mut t = doc.to_string();
        for (name, s) in &self.sections {
            t = crate::write_plan(&t, name, s.toml().trim_start_matches('\n').trim_end());
        }
        Ok(t)
    }
}

fn num(v: f64) -> toml_edit::Value {
    if v.fract() == 0.0 && v.abs() < 1e12 { (v as i64).into() } else { v.into() }
}

fn point_value(q: Point) -> Item {
    let [x, y] = q.to_mm();
    let mut a = toml_edit::Array::new();
    a.push(x);
    a.push(y);
    value(a)
}

fn sync_footprints(
    doc: &mut DocumentMut,
    was: &LayoutFile,
    now: &LayoutFile,
) -> Result<(), String> {
    let changed: Vec<_> = now
        .footprints
        .iter()
        .filter(|f| {
            was.footprints
                .iter()
                .find(|w| w.reference == f.reference)
                .is_none_or(|w| serde_json::to_value(w).ok() != serde_json::to_value(f).ok())
        })
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    let parts = doc
        .entry("footprints")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or("[[footprints]] is not an array of tables")?;
    for f in changed {
        let at = parts
            .iter()
            .position(|t| t.get("ref").and_then(Item::as_str) == Some(f.reference.as_str()));
        let t = match at {
            Some(k) => parts.get_mut(k).expect("found above"),
            None => {
                let mut t = toml_edit::Table::new();
                t["ref"] = value(f.reference.clone());
                parts.push(t);
                parts.iter_mut().last().expect("just pushed")
            }
        };
        t["at"] = point_value(f.at);
        match f.rotation {
            Some(r) => t["rotation"] = Item::Value(num(r)),
            None => {
                t.remove("rotation");
            }
        }
        match f.side {
            Some(agentee_core::layout::BoardSide::Bottom) => t["side"] = value("bottom"),
            _ => {
                t.remove("side");
            }
        }
        match &f.label {
            None => {
                t.remove("label");
            }
            Some(l) => {
                let mut label =
                    t.get("label").and_then(Item::as_inline_table).cloned().unwrap_or_default();
                match l.at {
                    Some(q) => {
                        let [x, y] = q.to_mm();
                        let mut a = toml_edit::Array::new();
                        a.push(x);
                        a.push(y);
                        label.insert("at", a.into());
                    }
                    None => {
                        label.remove("at");
                    }
                }
                match l.rotation {
                    Some(r) => {
                        label.insert("rotation", num(r));
                    }
                    None => {
                        label.remove("rotation");
                    }
                }
                if l.hide {
                    label.insert("hide", true.into());
                } else {
                    label.remove("hide");
                }
                t["label"] = value(label);
            }
        }
        if f.locked {
            t["locked"] = value(true);
        } else {
            t.remove("locked");
        }
    }
    Ok(())
}

fn sync_graphics(doc: &mut DocumentMut, was: &LayoutFile, now: &LayoutFile) {
    let Some(graphics) = doc.get_mut("graphics").and_then(Item::as_array_of_tables_mut) else {
        return;
    };
    for (k, (w, n)) in was.graphics.iter().zip(&now.graphics).enumerate() {
        if w.at.map(|q| q.to_mm()) == n.at.map(|q| q.to_mm()) {
            continue;
        }
        if let (Some(t), Some(at)) = (graphics.get_mut(k), n.at) {
            t["at"] = point_value(at);
        }
    }
}

use agentee_core::geom::{self, P, Transform};
use agentee_core::layout::{Layout, Track, Via, ViaSource};
use agentee_core::project::{Entry, LayoutInputs, Project};
use agentee_core::route::RouteResult;
use agentee_core::units::{Length, Point};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

type Resolved = Result<Entry<Layout>, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum Sel {
    Part(String),
    Track(usize),
    Via(ViaSource, P),
    Ratsnest(P, P, usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Route,
    Via,
}

#[derive(Clone, Debug)]
pub enum Drag {
    Pan,
    Part { reference: String, start: P, grab: P, now: P },
    Vertex { track: usize, vertex: usize, points: Vec<P> },
    Segment { track: usize, segment: usize, points: Vec<P>, grab: P, now: Vec<P> },
    Via { source: ViaSource, start: P, grab: P, now: P },
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub net: usize,
    pub layer: String,
    pub runs: Vec<(String, Vec<P>)>,
    pub points: Vec<P>,
    pub vias: Vec<P>,
    pub diagonal_first: bool,
}

pub struct Editor {
    pub path: PathBuf,
    disk: String,
    doc: DocumentMut,
    undo: Vec<String>,
    redo: Vec<String>,
    merge: Option<(String, Instant)>,
    inputs: Option<Arc<LayoutInputs>>,
    shown: Option<Entry<Layout>>,
    seq: u64,
    job: Option<(u64, mpsc::Receiver<Resolved>)>,
    route_job: Option<mpsc::Receiver<Result<RouteResult, String>>>,
    pub dirty: bool,
    pub error: Option<String>,
    pub conflict: Option<String>,
    pub sel: Option<Sel>,
    pub tool: Tool,
    pub drag: Option<Drag>,
    pub draft: Option<Draft>,
    pub layer: String,
    pub grid: f64,
    pub snap: bool,
    pub note: Option<String>,
    pub engine: crate::engine::Panel,
}

impl Editor {
    pub fn open(project: &Project, i: usize) -> Result<Editor, String> {
        let path = project.layouts[i].path.clone();
        let disk =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc = disk.parse().map_err(|e| format!("{e}"))?;
        Ok(Editor {
            path,
            disk,
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            merge: None,
            inputs: None,
            shown: None,
            seq: 0,
            job: None,
            route_job: None,
            dirty: false,
            error: None,
            conflict: None,
            sel: None,
            tool: Tool::Select,
            drag: None,
            draft: None,
            layer: project.layouts[i].item.copper.first().cloned().unwrap_or_default(),
            grid: 0.05,
            snap: true,
            note: None,
            engine: Default::default(),
        })
    }

    pub fn entry<'a>(&'a self, project: &'a Project, i: usize) -> &'a Entry<Layout> {
        self.shown.as_ref().unwrap_or(&project.layouts[i])
    }

    pub fn layout<'a>(&'a self, project: &'a Project, i: usize) -> &'a Layout {
        &self.entry(project, i).item
    }

    pub fn revision(&self) -> u64 {
        if self.shown.is_some() { self.seq } else { 0 }
    }

    pub fn checking(&self) -> bool {
        self.job.is_some()
    }

    pub fn routing(&self) -> bool {
        self.route_job.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn snap(&self, p: P) -> P {
        if !self.snap || self.grid <= 0.0 {
            return p;
        }
        p.map(|v| (v / self.grid).round() * self.grid)
    }

    pub(crate) fn inputs(
        &mut self,
        project: &Project,
        i: usize,
    ) -> Result<Arc<LayoutInputs>, String> {
        if self.inputs.is_none() {
            self.inputs = Some(Arc::new(project.layout_inputs(i)?));
        }
        Ok(self.inputs.clone().unwrap())
    }

    pub fn shown_mut(&mut self, project: &Project, i: usize) -> &mut Layout {
        &mut self.shown.get_or_insert_with(|| project.layouts[i].clone()).item
    }

    pub fn commit(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        key: Option<&str>,
        edit: impl FnOnce(&mut DocumentMut) -> Result<(), String>,
        patch: impl FnOnce(&mut Layout, &LayoutInputs),
    ) {
        let inputs = match self.inputs(project, i) {
            Ok(x) => x,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let merged = match (&self.merge, key) {
            (Some((k, t)), Some(now)) => k == now && t.elapsed() < Duration::from_millis(1500),
            _ => false,
        };
        let before = (!merged).then(|| self.doc.to_string());
        if let Err(e) = edit(&mut self.doc) {
            if let Some(b) = before {
                self.doc = b.parse().expect("the document parsed before");
            }
            self.note = Some(e);
            return;
        }
        if let Some(b) = before {
            self.undo.push(b);
        }
        self.merge = key.map(|k| (k.to_string(), Instant::now()));
        self.redo.clear();
        self.note = None;
        patch(self.shown_mut(project, i), &inputs);
        self.schedule(ctx);
    }

    fn schedule(&mut self, ctx: &egui::Context) {
        self.seq += 1;
        self.dirty = true;
        if self.job.is_none() {
            let text = self.doc.to_string();
            self.spawn(ctx, text);
        }
    }

    fn spawn(&mut self, ctx: &egui::Context, text: String) {
        self.dirty = text != self.disk;
        let Some(inputs) = self.inputs.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(inputs.resolve(&text));
            ctx.request_repaint();
        });
        self.job = Some((self.seq, rx));
    }

    pub fn poll(&mut self, ctx: &egui::Context, project: &Project, i: usize) {
        crate::engine::poll(self, ctx, project, i);
        if let Some(rx) = &self.route_job
            && let Ok(r) = rx.try_recv()
        {
            self.route_job = None;
            match r {
                Ok(r) => self.apply_route(ctx, project, i, r),
                Err(e) => self.note = Some(e),
            }
        }
        if self.drag.is_some() {
            return;
        }
        let Some((seq, rx)) = &self.job else { return };
        let seq = *seq;
        match rx.try_recv() {
            Ok(r) => {
                self.job = None;
                if seq != self.seq {
                    let text = self.doc.to_string();
                    self.spawn(ctx, text);
                    return;
                }
                match r {
                    Ok(e) => {
                        self.shown = Some(e);
                        self.error = None;
                        self.keep_selection();
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.job = None,
        }
    }

    fn keep_selection(&mut self) {
        let Some(l) = self.shown.as_ref().map(|e| &e.item) else { return };
        let alive = match &self.sel {
            Some(Sel::Part(r)) => l.parts.iter().any(|p| &p.reference == r),
            Some(Sel::Track(t)) => l.tracks.iter().any(|x| x.source == *t),
            Some(Sel::Via(s, at)) => l.vias.iter().any(|v| v.source == *s && near(v.at, *at)),
            Some(Sel::Ratsnest(a, b, _)) => {
                l.ratsnest.iter().any(|(x, y, _)| near(*x, *a) && near(*y, *b))
            }
            None => true,
        };
        if !alive {
            self.sel = None;
        }
    }

    pub fn undo(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.undo.pop() {
            self.redo.push(self.doc.to_string());
            self.restore(ctx, &t);
        }
    }

    pub fn redo(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.redo.pop() {
            self.undo.push(self.doc.to_string());
            self.restore(ctx, &t);
        }
    }

    fn restore(&mut self, ctx: &egui::Context, text: &str) {
        self.doc = text.parse().expect("an undo snapshot parses");
        self.merge = None;
        self.draft = None;
        self.drag = None;
        self.note = None;
        if text == self.disk && self.inputs.is_none() {
            self.shown = None;
            self.dirty = false;
            return;
        }
        self.schedule(ctx);
    }

    pub fn save(&mut self) -> Result<(), String> {
        let text = self.doc.to_string();
        std::fs::write(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.disk = text;
        self.dirty = false;
        self.conflict = None;
        Ok(())
    }

    pub fn revert(&mut self, ctx: &egui::Context) {
        if !self.dirty {
            return;
        }
        self.undo.push(self.doc.to_string());
        self.redo.clear();
        let disk = self.disk.clone();
        self.restore(ctx, &disk);
        self.shown = None;
        self.dirty = false;
        self.job = None;
        self.seq += 1;
    }

    pub fn sync(&mut self, ctx: &egui::Context, project: &Project, i: usize) {
        let Ok(now) = std::fs::read_to_string(&self.path) else { return };
        if self.inputs.is_some() {
            self.inputs = project.layout_inputs(i).ok().map(Arc::new);
        }
        if now == self.disk {
            if self.dirty {
                self.schedule(ctx);
            } else {
                self.shown = None;
                self.job = None;
            }
            return;
        }
        if self.dirty {
            self.conflict = Some(now);
            return;
        }
        match now.parse() {
            Ok(doc) => {
                self.doc = doc;
                self.disk = now;
                self.undo.clear();
                self.redo.clear();
                self.shown = None;
                self.job = None;
                self.draft = None;
            }
            Err(e) => self.error = Some(format!("{e}")),
        }
    }

    pub fn keep_mine(&mut self) {
        if let Some(t) = self.conflict.take() {
            self.disk = t;
        }
    }

    pub fn take_theirs(&mut self, ctx: &egui::Context) {
        let Some(t) = self.conflict.take() else { return };
        self.undo.push(self.doc.to_string());
        self.disk = t.clone();
        self.restore(ctx, &t);
        self.dirty = false;
        self.shown = None;
        self.job = None;
        self.seq += 1;
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub fn replace_text(&mut self, ctx: &egui::Context, project: &Project, i: usize, text: &str) {
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| {
                *doc = text.parse().map_err(|e| format!("{e}"))?;
                Ok(())
            },
            |_, _| {},
        );
        self.sel = None;
        self.draft = None;
    }

    pub fn engine_get(&self, section: &str, key: &str) -> Option<&Item> {
        let engine = self.doc.get("engine")?;
        if section.is_empty() { engine.get(key) } else { engine.get(section)?.get(key) }
    }

    pub fn engine_set(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        section: &str,
        key: &str,
        value: Option<Value>,
    ) {
        let merge = format!("engine {section}.{key}");
        self.commit(
            ctx,
            project,
            i,
            Some(&merge),
            |doc| {
                set_engine(doc, section, key, value);
                Ok(())
            },
            |_, _| {},
        );
    }

    pub fn route_connection(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        net: usize,
        connection: Option<(P, P)>,
    ) {
        let inputs = match self.inputs(project, i) {
            Ok(x) => x,
            Err(e) => {
                self.note = Some(e);
                return;
            }
        };
        let layout = self.layout(project, i).clone();
        let opts = agentee_core::route::RouteOptions {
            nets: vec![layout.nets[net].name.clone()],
            connection,
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(agentee_core::route::route(&layout, &inputs.board, &opts));
            ctx.request_repaint();
        });
        self.route_job = Some(rx);
        self.note = None;
    }

    pub fn tie_planes(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        nets: &[String],
    ) {
        let inputs = match self.inputs(project, i) {
            Ok(x) => x,
            Err(e) => {
                self.note = Some(e);
                return;
            }
        };
        match agentee_core::tie::tie(self.layout(project, i), &inputs.board, nets) {
            Ok(t) => {
                let failed = t.failed.len();
                let r = RouteResult { tracks: t.tracks, vias: t.vias, ..Default::default() };
                self.apply_route(ctx, project, i, r);
                self.note = Some(format!(
                    "{} pads tied, {} had a via already{}",
                    t.tied,
                    t.already,
                    if failed > 0 { format!(", {failed} without room") } else { String::new() }
                ));
            }
            Err(e) => self.note = Some(e),
        }
    }

    fn apply_route(&mut self, ctx: &egui::Context, project: &Project, i: usize, r: RouteResult) {
        self.note = r.failed.first().map(|f| format!("{}: {}", f.net, f.reason));
        if r.tracks.is_empty() && r.vias.is_empty() {
            self.note.get_or_insert_with(|| "the router found nothing to add".into());
            return;
        }
        let tracks = r.tracks.clone();
        let vias = r.vias.clone();
        let note = self.note.clone();
        let (first_track, first_via) = (self.len_of("tracks"), self.len_of("vias"));
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| {
                for t in &tracks {
                    add_track(doc, &t.net, &t.layer, &t.points, t.width)?;
                }
                for v in &vias {
                    add_via(doc, &v.net, v.at, Some(&v.via))?;
                }
                Ok(())
            },
            |l, inputs| {
                for (k, t) in r.tracks.iter().enumerate() {
                    let Some(net) = net_index(l, &t.net) else { continue };
                    let width = t.width.unwrap_or(l.nets[net].width);
                    push_track(l, first_track + k, net, &t.layer, t.points.clone(), width);
                }
                for (k, v) in r.vias.iter().enumerate() {
                    let Some(net) = net_index(l, &v.net) else { continue };
                    let source = ViaSource::File { index: first_via + k, k: 0 };
                    push_via_with(l, inputs, net, v.at, Some(&v.via), source);
                }
            },
        );
        self.note = note;
        self.sel = None;
    }

    pub fn move_part(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        reference: &str,
        dragged_from: Option<P>,
        to: P,
    ) {
        let patched = dragged_from.is_some();
        let Some(from) = dragged_from.or_else(|| {
            self.layout(project, i)
                .parts
                .iter()
                .find(|p| p.reference == reference)
                .map(|p| p.at.to_mm())
        }) else {
            return;
        };
        let d = [to[0] - from[0], to[1] - from[1]];
        if d == [0.0, 0.0] {
            return;
        }
        let key = format!("part-at {reference}");
        self.commit(
            ctx,
            project,
            i,
            Some(&key),
            |doc| {
                let t = part(doc, reference)?;
                t.insert("at", Item::Value(point(to)));
                shift_label(t, d, 0.0, from);
                Ok(())
            },
            |l, _| {
                if !patched {
                    shift_part(l, reference, d);
                }
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_part(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        reference: &str,
        rotation: f64,
        bottom: bool,
        locked: bool,
    ) {
        let Some(p) = self.layout(project, i).parts.iter().find(|p| p.reference == reference)
        else {
            return;
        };
        let (old_rot, old_bottom, at) = (p.rotation, p.bottom, p.at.to_mm());
        let old_locked = locked_of(&self.doc, reference);
        let rotation = rotation.rem_euclid(360.0);
        if rotation == old_rot.rem_euclid(360.0) && bottom == old_bottom && locked == old_locked {
            return;
        }
        let key = format!("part {reference}");
        self.commit(
            ctx,
            project,
            i,
            Some(&key),
            |doc| {
                let t = part(doc, reference)?;
                if rotation == 0.0 {
                    t.remove("rotation");
                } else {
                    t.insert("rotation", Item::Value(angle(rotation)));
                }
                if bottom {
                    t.insert("side", Item::Value("bottom".into()));
                } else {
                    t.remove("side");
                }
                if locked {
                    t.insert("locked", Item::Value(true.into()));
                } else {
                    t.remove("locked");
                }
                shift_label(t, [0.0, 0.0], rotation - old_rot, at);
                Ok(())
            },
            |l, _| {
                if bottom == old_bottom {
                    turn_part(l, reference, rotation);
                }
            },
        );
    }

    pub fn track_points(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        track: usize,
        points: Vec<P>,
    ) {
        let pts = points.clone();
        self.commit(
            ctx,
            project,
            i,
            Some(&format!("track-points {track}")),
            |doc| {
                entry(doc, "tracks", track)?.insert("points", Item::Value(polyline(&pts)));
                Ok(())
            },
            |l, _| {
                if let Some(t) = l.tracks.iter_mut().find(|t| t.source == track) {
                    t.points = points;
                }
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_track(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        track: usize,
        net: usize,
        layer: &str,
        width: Option<f64>,
    ) {
        let l = self.layout(project, i);
        let name = l.nets[net].name.clone();
        let default_width = l.nets[net].width;
        self.commit(
            ctx,
            project,
            i,
            Some(&format!("track {track}")),
            |doc| {
                let t = entry(doc, "tracks", track)?;
                t.insert("net", Item::Value(name.as_str().into()));
                t.insert("layer", Item::Value(layer.into()));
                match width {
                    Some(w) => t.insert("width", Item::Value(num(w))),
                    None => t.remove("width"),
                };
                Ok(())
            },
            |l, inputs| {
                let class = l.nets[net].class.clone();
                if let Some(t) = l.tracks.iter_mut().find(|t| t.source == track) {
                    t.net = net;
                    t.layer = layer.to_string();
                    t.width = width.unwrap_or_else(|| {
                        inputs
                            .board
                            .netclasses
                            .iter()
                            .find(|c| c.name == class)
                            .map(|c| c.width_on(layer).to_mm())
                            .unwrap_or(default_width)
                    });
                }
            },
        );
    }

    pub fn delete_track(&mut self, ctx: &egui::Context, project: &Project, i: usize, track: usize) {
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| {
                list(doc, "tracks")?.remove(track);
                Ok(())
            },
            |l, _| {
                l.tracks.retain(|t| t.source != track);
                for t in &mut l.tracks {
                    if t.source > track {
                        t.source -= 1;
                    }
                }
            },
        );
        self.sel = None;
    }

    pub fn move_via(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        source: ViaSource,
        from: P,
        to: P,
    ) {
        if near(from, to) {
            return;
        }
        let Some((net, via_name)) = self.via_info(project, i, source, from) else { return };
        let name = self.layout(project, i).nets[net].name.clone();
        let plan = self.plan_detach(source);
        let key = format!("via-at {source:?}");
        let target = plan.map(|p| ViaSource::File { index: p.index, k: 0 }).unwrap_or(source);
        self.commit(
            ctx,
            project,
            i,
            plan.is_none().then_some(key.as_str()),
            |doc| {
                let index = match (plan, source) {
                    (Some(_), _) => detach_via(doc, source, from, &name, Some(&via_name))?,
                    (None, ViaSource::File { index, .. }) => index,
                    _ => return Err("this via cannot move".into()),
                };
                entry(doc, "vias", index)?.insert("at", Item::Value(point(to)));
                Ok(())
            },
            |l, _| {
                if let Some(p) = plan {
                    detach_patch(l, source, from, p);
                }
                drag_via(l, target, from, to);
            },
        );
        if matches!(self.sel, Some(Sel::Via(..))) {
            self.sel = Some(Sel::Via(target, to));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_via(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        source: ViaSource,
        at: P,
        net: usize,
        via: Option<String>,
    ) {
        let name = self.layout(project, i).nets[net].name.clone();
        let Some((_, old_name)) = self.via_info(project, i, source, at) else { return };
        let plan = self.plan_detach(source);
        let target = plan.map(|p| ViaSource::File { index: p.index, k: 0 }).unwrap_or(source);
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| {
                let index = match (plan, source) {
                    (Some(_), _) => detach_via(doc, source, at, &name, Some(&old_name))?,
                    (None, ViaSource::File { index, .. }) => index,
                    _ => return Err("this via cannot change".into()),
                };
                let t = entry(doc, "vias", index)?;
                t.insert("net", Item::Value(name.as_str().into()));
                match &via {
                    Some(v) => t.insert("via", Item::Value(v.as_str().into())),
                    None => t.remove("via"),
                };
                Ok(())
            },
            |l, inputs| {
                if let Some(p) = plan {
                    detach_patch(l, source, at, p);
                }
                l.vias.retain(|v| !(v.source == target && near(v.at, at)));
                push_via_with(l, inputs, net, at, via.as_deref(), target);
            },
        );
        self.sel = Some(Sel::Via(target, at));
    }

    pub fn delete_via(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        source: ViaSource,
        at: P,
    ) {
        let plan = self.plan_detach(source);
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| match source {
                ViaSource::File { index, k } if plan.is_some() => {
                    split_array(doc, index, k as usize)
                }
                ViaSource::File { index, .. } => {
                    list(doc, "vias")?.remove(index);
                    Ok(())
                }
                ViaSource::Fanout(j) => skip_at(doc, "fanouts", j, at),
                ViaSource::Stitch(j) => skip_at(doc, "stitching", j, at),
                ViaSource::Generated => Err("this via comes from a rule that has no skip".into()),
            },
            |l, _| {
                l.vias.retain(|v| !(v.source == source && near(v.at, at)));
                match (source, plan) {
                    (ViaSource::File { index, k }, Some(p)) => renumber_split(l, index, k, p.tail),
                    (ViaSource::File { index, .. }, None) => {
                        for v in &mut l.vias {
                            if let ViaSource::File { index: x, .. } = &mut v.source
                                && *x > index
                            {
                                *x -= 1;
                            }
                        }
                    }
                    _ => {}
                }
            },
        );
        self.sel = None;
    }

    pub fn place_via(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        i: usize,
        net: usize,
        at: P,
    ) {
        let name = self.layout(project, i).nets[net].name.clone();
        let source = ViaSource::File { index: self.len_of("vias"), k: 0 };
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| add_via(doc, &name, at, None).map(|_| ()),
            |l, inputs| push_via_with(l, inputs, net, at, None, source),
        );
        self.sel = Some(Sel::Via(source, at));
    }

    pub fn finish_draft(&mut self, ctx: &egui::Context, project: &Project, i: usize) {
        let Some(mut d) = self.draft.take() else { return };
        if d.points.len() >= 2 {
            d.runs.push((d.layer.clone(), std::mem::take(&mut d.points)));
        }
        d.runs.iter_mut().for_each(|(_, p)| p.dedup_by(|a, b| near(*a, *b)));
        d.runs.retain(|(_, p)| p.len() >= 2);
        if d.runs.is_empty() && d.vias.is_empty() {
            return;
        }
        let name = self.layout(project, i).nets[d.net].name.clone();
        let width = self.layout(project, i).nets[d.net].width;
        let (runs, vias) = (d.runs.clone(), d.vias.clone());
        let reach: Vec<(P, Vec<String>)> = d
            .vias
            .iter()
            .map(|v| {
                let touching: Vec<String> = d
                    .runs
                    .iter()
                    .filter(|(_, p)| p.iter().any(|q| near(*q, *v)))
                    .map(|(l, _)| l.clone())
                    .collect();
                (*v, touching)
            })
            .collect();
        let class = self.layout(project, i).nets[d.net].class.clone();
        let (first_track, first_via) = (self.len_of("tracks"), self.len_of("vias"));
        let names: Vec<Option<String>> = match self.inputs(project, i) {
            Ok(inputs) => reach
                .iter()
                .map(|(_, layers)| {
                    let r: Vec<&str> = layers.iter().map(String::as_str).collect();
                    let c = inputs.board.netclasses.iter().find(|c| c.name == class);
                    inputs.board.via_for(None, c, &r).map(|v| v.name.clone())
                })
                .collect(),
            Err(_) => vec![None; reach.len()],
        };
        self.commit(
            ctx,
            project,
            i,
            None,
            |doc| {
                for (layer, pts) in &runs {
                    add_track(doc, &name, layer, pts, None)?;
                }
                for (v, n) in vias.iter().zip(&names) {
                    add_via(doc, &name, *v, n.as_deref())?;
                }
                Ok(())
            },
            |l, inputs| {
                for (k, (layer, pts)) in d.runs.iter().enumerate() {
                    let w = inputs
                        .board
                        .netclasses
                        .iter()
                        .find(|c| c.name == class)
                        .map(|c| c.width_on(layer).to_mm())
                        .unwrap_or(width);
                    push_track(l, first_track + k, d.net, layer, pts.clone(), w);
                }
                for (k, (v, n)) in d.vias.iter().zip(&names).enumerate() {
                    let source = ViaSource::File { index: first_via + k, k: 0 };
                    push_via_with(l, inputs, d.net, *v, n.as_deref(), source);
                }
            },
        );
    }

    pub fn via_choices(&mut self, project: &Project, i: usize) -> Vec<String> {
        self.inputs(project, i)
            .map(|x| x.board.vias.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn reach(&mut self, project: &Project, i: usize, net: usize, from: &str) -> Option<String> {
        let inputs = self.inputs(project, i).ok()?;
        let l = self.layout(project, i);
        let class = inputs.board.netclasses.iter().find(|c| c.name == l.nets[net].class);
        let copper = &l.copper;
        let start = copper.iter().position(|c| c == from)?;
        (1..copper.len()).map(|k| &copper[(start + k) % copper.len()]).find_map(|to| {
            let v = inputs.board.via_for(None, class, &[from, to])?;
            let on = v.copper_layers(copper);
            (on.iter().any(|x| x == from) && on.iter().any(|x| x == to)).then(|| to.clone())
        })
    }

    pub fn is_array(&self, source: ViaSource) -> bool {
        let ViaSource::File { index, .. } = source else { return false };
        self.doc
            .get("vias")
            .and_then(Item::as_array_of_tables)
            .and_then(|a| a.get(index))
            .and_then(|t| t.get("count"))
            .and_then(Item::as_integer)
            .is_some_and(|c| c > 1)
    }

    pub fn origin_of(&self, source: ViaSource) -> String {
        match source {
            ViaSource::File { index, k } if self.is_array(source) => {
                format!("vias[{index}] element {k}")
            }
            ViaSource::File { index, .. } => format!("vias[{index}]"),
            ViaSource::Fanout(j) => format!("fanouts[{j}]"),
            ViaSource::Stitch(j) => format!("stitching[{j}]"),
            ViaSource::Generated => "generated".into(),
        }
    }

    pub fn track_width_set(&self, track: usize) -> bool {
        self.doc
            .get("tracks")
            .and_then(Item::as_array_of_tables)
            .and_then(|a| a.get(track))
            .is_some_and(|t| t.contains_key("width"))
    }

    pub fn via_named(&self, source: ViaSource) -> Option<String> {
        let ViaSource::File { index, .. } = source else { return None };
        self.doc
            .get("vias")
            .and_then(Item::as_array_of_tables)
            .and_then(|a| a.get(index))
            .and_then(|t| t.get("via"))
            .and_then(Item::as_str)
            .map(String::from)
    }

    pub fn locked(&self, reference: &str) -> bool {
        locked_of(&self.doc, reference)
    }

    fn via_info(
        &self,
        project: &Project,
        i: usize,
        source: ViaSource,
        at: P,
    ) -> Option<(usize, String)> {
        self.layout(project, i)
            .vias
            .iter()
            .find(|v| v.source == source && near(v.at, at))
            .map(|v| (v.net, v.name.clone()))
    }

    fn len_of(&self, key: &str) -> usize {
        match self.doc.get(key) {
            Some(Item::ArrayOfTables(a)) => a.len(),
            Some(Item::Value(Value::Array(a))) => a.len(),
            _ => 0,
        }
    }

    fn plan_detach(&self, source: ViaSource) -> Option<Detach> {
        let entries = self.len_of("vias");
        match source {
            ViaSource::File { index, k } => {
                let count = self
                    .doc
                    .get("vias")
                    .and_then(Item::as_array_of_tables)
                    .and_then(|a| a.get(index))
                    .and_then(|t| t.get("count"))
                    .and_then(Item::as_integer)
                    .unwrap_or(1)
                    .max(1) as u32;
                if count <= 1 {
                    return None;
                }
                let tail = (k > 0 && k + 1 < count).then_some(entries);
                Some(Detach { index: entries + tail.is_some() as usize, tail })
            }
            ViaSource::Fanout(_) | ViaSource::Stitch(_) => {
                Some(Detach { index: entries, tail: None })
            }
            ViaSource::Generated => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Detach {
    index: usize,
    tail: Option<usize>,
}

fn subtable<'a>(t: &'a mut Table, key: &str) -> Option<&'a mut Table> {
    if !t.contains_key(key) {
        let mut sub = Table::new();
        sub.set_implicit(true);
        sub.set_position(Some(0));
        t.insert(key, Item::Table(sub));
    }
    t.get_mut(key)?.as_table_mut()
}

fn set_engine(doc: &mut DocumentMut, section: &str, key: &str, value: Option<Value>) {
    let Some(engine) = subtable(doc.as_table_mut(), "engine") else { return };
    let target = if section.is_empty() { Some(engine) } else { subtable(engine, section) };
    let Some(t) = target else { return };
    match value {
        Some(v) => set(t, key, v),
        None => {
            t.remove(key);
        }
    }
    let Some(engine) = doc.get_mut("engine").and_then(Item::as_table_mut) else { return };
    if !section.is_empty()
        && engine.get(section).and_then(Item::as_table).is_some_and(|t| t.is_empty())
    {
        engine.remove(section);
    }
    if engine.is_empty() {
        doc.remove("engine");
    }
}

pub fn near(a: P, b: P) -> bool {
    geom::dist(a, b) < 1e-6
}

fn net_index(l: &Layout, name: &str) -> Option<usize> {
    l.nets.iter().position(|n| n.name == name)
}

fn locked_of(doc: &DocumentMut, reference: &str) -> bool {
    doc.get("footprints")
        .and_then(Item::as_array_of_tables)
        .and_then(|a| a.iter().find(|t| t.get("ref").and_then(Item::as_str) == Some(reference)))
        .and_then(|t| t.get("locked"))
        .and_then(Item::as_bool)
        .unwrap_or(false)
}

fn renumber_split(l: &mut Layout, index: usize, k: u32, tail: Option<usize>) {
    for v in &mut l.vias {
        if let ViaSource::File { index: x, k: j } = &mut v.source
            && *x == index
            && *j > k
        {
            match (k, tail) {
                (0, _) => *j -= 1,
                (_, Some(t)) => {
                    *x = t;
                    *j -= k + 1;
                }
                _ => {}
            }
        }
    }
}

fn detach_patch(l: &mut Layout, source: ViaSource, at: P, plan: Detach) {
    let moved = l.vias.iter().position(|v| v.source == source && near(v.at, at));
    if let ViaSource::File { index, k } = source {
        renumber_split(l, index, k, plan.tail);
    }
    if let Some(m) = moved {
        l.vias[m].source = ViaSource::File { index: plan.index, k: 0 };
    }
}

fn push_track(l: &mut Layout, source: usize, net: usize, layer: &str, points: Vec<P>, width: f64) {
    l.tracks.push(Track { source, net, layer: layer.to_string(), width, points });
}

fn push_via_with(
    l: &mut Layout,
    inputs: &LayoutInputs,
    net: usize,
    at: P,
    via: Option<&str>,
    source: ViaSource,
) {
    let class = inputs.board.netclasses.iter().find(|c| c.name == l.nets[net].class);
    if let Some(spec) = inputs.board.via_for(via, class, &[]) {
        l.vias.push(Via { source, ..Via::of(spec, net, at, &l.copper) });
    }
}

fn shift_part(l: &mut Layout, reference: &str, d: P) {
    let Some(p) = l.parts.iter_mut().find(|p| p.reference == reference) else { return };
    let at = p.at.to_mm();
    p.at = Point::mm(at[0] + d[0], at[1] + d[1]);
    let add = |q: &mut P| *q = [q[0] + d[0], q[1] + d[1]];
    for pad in &mut p.pads {
        pad.outlines.iter_mut().flatten().for_each(add);
        if let Some((c, _, _)) = &mut pad.drill {
            add(c);
        }
    }
    if let Some(lb) = &mut p.label {
        add(&mut lb.at);
    }
}

pub fn drag_part(l: &mut Layout, reference: &str, to: P) {
    let Some(p) = l.parts.iter().find(|p| p.reference == reference) else { return };
    let at = p.at.to_mm();
    shift_part(l, reference, [to[0] - at[0], to[1] - at[1]]);
}

fn turn_part(l: &mut Layout, reference: &str, rotation: f64) {
    let Some(p) = l.parts.iter_mut().find(|p| p.reference == reference) else { return };
    let old = p.transform();
    let new = Transform { rotation, ..old };
    let map = |q: &mut P| *q = new.apply(invert(&old, *q));
    for pad in &mut p.pads {
        pad.outlines.iter_mut().flatten().for_each(map);
        if let Some((c, _, r)) = &mut pad.drill {
            map(c);
            *r += rotation - old.rotation;
        }
    }
    p.rotation = rotation;
}

fn invert(t: &Transform, q: P) -> P {
    let local = geom::rotate([q[0] - t.at[0], q[1] - t.at[1]], -t.rotation);
    if t.mirror { [-local[0], local[1]] } else { local }
}

pub fn drag_via(l: &mut Layout, source: ViaSource, from: P, to: P) {
    if let Some(v) = l.vias.iter_mut().find(|v| v.source == source && near(v.at, from)) {
        v.at = to;
    }
}

pub fn drag_track(l: &mut Layout, track: usize, points: &[P]) {
    if let Some(t) = l.tracks.iter_mut().find(|t| t.source == track) {
        t.points = points.to_vec();
    }
}

fn round(v: f64) -> f64 {
    let r = (v * 1e4).round() / 1e4;
    if r == 0.0 { 0.0 } else { r }
}

fn num(v: f64) -> Value {
    Value::from(round(v))
}

fn angle(v: f64) -> Value {
    let r = round(v);
    if r.fract() == 0.0 { Value::from(r as i64) } else { Value::from(r) }
}

fn point(p: P) -> Value {
    let mut a = Array::new();
    a.push(round(p[0]));
    a.push(round(p[1]));
    Value::Array(a)
}

fn polyline(pts: &[P]) -> Value {
    let mut a = Array::new();
    for p in pts {
        a.push(point(*p));
    }
    Value::Array(a)
}

fn length(v: &Value) -> Option<f64> {
    v.as_float()
        .or_else(|| v.as_integer().map(|i| i as f64))
        .or_else(|| v.as_str().and_then(|s| Length::parse(s).ok()).map(Length::to_mm))
}

fn read_point(item: Option<&Item>) -> Option<P> {
    let a = item?.as_array()?;
    Some([length(a.get(0)?)?, length(a.get(1)?)?])
}

fn list<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut ArrayOfTables, String> {
    let inline: Option<Vec<Table>> = doc.get(key).and_then(Item::as_array).map(|a| {
        a.iter().filter_map(Value::as_inline_table).map(|t| t.clone().into_table()).collect()
    });
    if let Some(tables) = inline {
        let mut aot = ArrayOfTables::new();
        tables.into_iter().for_each(|t| aot.push(t));
        doc[key] = Item::ArrayOfTables(aot);
    }
    if doc.get(key).is_none() {
        doc[key] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    doc[key].as_array_of_tables_mut().ok_or_else(|| format!("`{key}` is not a list of tables"))
}

fn entry<'a>(doc: &'a mut DocumentMut, key: &str, i: usize) -> Result<&'a mut Table, String> {
    list(doc, key)?.get_mut(i).ok_or_else(|| format!("{key}[{i}] is not in the file"))
}

fn part<'a>(doc: &'a mut DocumentMut, reference: &str) -> Result<&'a mut Table, String> {
    list(doc, "footprints")?
        .iter_mut()
        .find(|t| t.get("ref").and_then(Item::as_str) == Some(reference))
        .ok_or_else(|| format!("{reference} has no [[footprints]] entry"))
}

fn shift_label(t: &mut Table, d: P, turn: f64, pivot: P) {
    let Some(label) = t.get_mut("label").and_then(Item::as_table_like_mut) else { return };
    let Some(at) = read_point(label.get("at")) else { return };
    let rel = geom::rotate([at[0] - pivot[0], at[1] - pivot[1]], turn);
    let to = [pivot[0] + rel[0] + d[0], pivot[1] + rel[1] + d[1]];
    set(label, "at", point(to));
}

fn set(t: &mut dyn TableLike, key: &str, v: Value) {
    match t.get_mut(key) {
        Some(Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = v;
            *old.decor_mut() = decor;
        }
        _ => {
            t.insert(key, Item::Value(v));
        }
    }
}

pub fn add_track(
    doc: &mut DocumentMut,
    net: &str,
    layer: &str,
    pts: &[P],
    width: Option<f64>,
) -> Result<usize, String> {
    let mut t = Table::new();
    t.insert("net", Item::Value(net.into()));
    t.insert("layer", Item::Value(layer.into()));
    if let Some(w) = width {
        t.insert("width", Item::Value(num(w)));
    }
    t.insert("points", Item::Value(polyline(pts)));
    let l = list(doc, "tracks")?;
    l.push(t);
    Ok(l.len() - 1)
}

pub fn add_via(
    doc: &mut DocumentMut,
    net: &str,
    at: P,
    via: Option<&str>,
) -> Result<usize, String> {
    let mut t = Table::new();
    t.insert("net", Item::Value(net.into()));
    t.insert("at", Item::Value(point(at)));
    if let Some(v) = via {
        t.insert("via", Item::Value(v.into()));
    }
    let l = list(doc, "vias")?;
    l.push(t);
    Ok(l.len() - 1)
}

fn skip_at(doc: &mut DocumentMut, key: &str, j: usize, at: P) -> Result<(), String> {
    let t = entry(doc, key, j)?;
    match t.get_mut("skip_at").and_then(Item::as_array_mut) {
        Some(a) => a.push(point(at)),
        None => {
            let mut a = Array::new();
            a.push(point(at));
            t.insert("skip_at", Item::Value(Value::Array(a)));
        }
    }
    Ok(())
}

fn split_array(doc: &mut DocumentMut, index: usize, k: usize) -> Result<(), String> {
    let vias = list(doc, "vias")?;
    let t = vias.get_mut(index).ok_or_else(|| format!("vias[{index}] is not in the file"))?;
    let at = read_point(t.get("at")).ok_or("the via has no readable `at`")?;
    let pitch = read_point(t.get("pitch")).unwrap_or([0.0, 0.0]);
    let count = t.get("count").and_then(Item::as_integer).unwrap_or(1).max(1) as usize;
    let after = count.saturating_sub(k + 1);
    let shifted = |n: usize| [at[0] + pitch[0] * n as f64, at[1] + pitch[1] * n as f64];
    let set_count = |t: &mut Table, n: usize| {
        if n > 1 {
            t.insert("count", Item::Value(Value::from(n as i64)));
        } else {
            t.remove("count");
            t.remove("pitch");
        }
    };
    if k == 0 {
        if after == 0 {
            vias.remove(index);
        } else {
            t.insert("at", Item::Value(point(shifted(1))));
            set_count(t, after);
        }
        return Ok(());
    }
    let mut tail = t.clone();
    set_count(t, k);
    if after > 0 {
        tail.insert("at", Item::Value(point(shifted(k + 1))));
        set_count(&mut tail, after);
        tail.decor_mut().clear();
        vias.push(tail);
    }
    Ok(())
}

fn detach_via(
    doc: &mut DocumentMut,
    source: ViaSource,
    at: P,
    net: &str,
    via: Option<&str>,
) -> Result<usize, String> {
    match source {
        ViaSource::File { index, k } => split_array(doc, index, k as usize)?,
        ViaSource::Fanout(j) => skip_at(doc, "fanouts", j, at)?,
        ViaSource::Stitch(j) => skip_at(doc, "stitching", j, at)?,
        ViaSource::Generated => return Err("this via comes from a rule that has no skip".into()),
    }
    add_via(doc, net, at, via)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    #[test]
    fn engine_settings_land_in_their_section_and_leave_when_cleared() {
        let mut d = doc("name = \"x\"\n\n[[footprints]]\nref = \"R1\"\nat = [0, 0]\n");
        set_engine(&mut d, "detail", "rip_limit", Some(Value::from(12)));
        let s = d.to_string();
        assert!(s.contains("[engine.detail]\nrip_limit = 12"), "{s}");
        assert!(s.find("[engine.detail]") < s.find("[[footprints]]"), "{s}");
        set_engine(&mut d, "detail", "rip_limit", None);
        assert!(!d.to_string().contains("engine"), "{d}");
    }

    #[test]
    fn splitting_an_array_keeps_both_sides() {
        let mut d =
            doc("[[vias]]\nnet = \"GND\"\nat = [0.0, 0.0]\ncount = 5\npitch = [1.0, 0.0]\n");
        split_array(&mut d, 0, 2).unwrap();
        let v = d["vias"].as_array_of_tables().unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v.get(0).unwrap()["count"].as_integer(), Some(2));
        assert_eq!(read_point(v.get(1).unwrap().get("at")), Some([3.0, 0.0]));
        assert_eq!(v.get(1).unwrap()["count"].as_integer(), Some(2));
    }

    #[test]
    fn splitting_the_first_of_two_leaves_a_single_via() {
        let mut d = doc("[[vias]]\nnet = \"GND\"\nat = [0, 0]\ncount = 2\npitch = [\"1mm\", 0]\n");
        split_array(&mut d, 0, 0).unwrap();
        let t = d["vias"].as_array_of_tables().unwrap().get(0).unwrap();
        assert_eq!(read_point(t.get("at")), Some([1.0, 0.0]));
        assert!(!t.contains_key("count") && !t.contains_key("pitch"));
    }

    #[test]
    fn moving_a_part_carries_its_label() {
        let mut d =
            doc("[[footprints]]\nref = \"R1\"\nat = [1.0, 1.0]\nlabel = { at = [1.0, 2.0] }\n");
        let t = part(&mut d, "R1").unwrap();
        t.insert("at", Item::Value(point([3.0, 1.0])));
        shift_label(t, [2.0, 0.0], 0.0, [1.0, 1.0]);
        let s = d.to_string();
        assert!(s.contains("at = [3.0, 1.0]"), "{s}");
        assert!(s.contains("label = { at = [3.0, 2.0] }"), "{s}");
    }

    #[test]
    fn detaching_a_fanout_via_skips_it_in_the_rule() {
        let mut d = doc("[[fanouts]]\nref = \"U1\"\n");
        let i = detach_via(&mut d, ViaSource::Fanout(0), [1.5, 2.0], "GND", Some("micro")).unwrap();
        assert_eq!(i, 0);
        let s = d.to_string();
        assert!(s.contains("skip_at = [[1.5, 2.0]]"), "{s}");
        assert!(s.contains("[[vias]]\nnet = \"GND\"\nat = [1.5, 2.0]\nvia = \"micro\""), "{s}");
    }
}

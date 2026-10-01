use crate::canvas::View;
use crate::edit::{Draft, Drag, Editor, Sel, Tool, drag_part, drag_track, drag_via, near};
use crate::paint::{Layers, Xf};
use crate::pcb::copper_color;
use agentee_core::geom::{self, P};
use agentee_core::graphic::{Bounds, Shape};
use agentee_core::layout::{Layout, Placed, ViaSource};
use agentee_core::project::Project;
use agentee_core::units::trim;
use egui::epaint::PathShape;
use egui::{Align2, Key, Modifiers, PointerButton, Pos2, Rect, Response, Stroke, Ui, Vec2};
use egui_bench::prelude::*;

const PICK_PX: f64 = 5.0;
const GRIDS: [f64; 8] = [0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 1.27];

#[derive(Clone, Copy, Debug)]
enum Pick {
    Part(usize),
    Track { index: usize, segment: usize, vertex: Option<usize> },
    Via(usize),
    Ratsnest(usize),
}

fn part_box(p: &Placed) -> Bounds {
    let mut b = Bounds::EMPTY;
    p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
    let t = p.transform();
    for g in &p.footprint.graphics {
        let body = g.layer.ends_with(".CrtYd") || g.layer.ends_with(".Fab");
        if !body || matches!(g.shape, Shape::Text { .. }) {
            continue;
        }
        let gb = g.bounds();
        if gb.is_empty() {
            continue;
        }
        for c in [gb.min, gb.max, [gb.min[0], gb.max[1]], [gb.max[0], gb.min[1]]] {
            b.add(t.apply(c));
        }
    }
    b
}

fn inside(b: &Bounds, m: P) -> bool {
    !b.is_empty() && m[0] >= b.min[0] && m[0] <= b.max[0] && m[1] >= b.min[1] && m[1] <= b.max[1]
}

fn side_shown(p: &Placed, layers: &Layers) -> bool {
    let side = if p.bottom { "B" } else { "F" };
    layers.shows(&format!("{side}.Cu")) || layers.shows(&format!("{side}.SilkS"))
}

fn pad_center(o: &[Vec<P>]) -> P {
    let mut b = Bounds::EMPTY;
    o.iter().flatten().for_each(|q| b.add(*q));
    b.center()
}

fn project_onto(m: P, a: P, b: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    if len < 1e-12 {
        return a;
    }
    let t = (((m[0] - a[0]) * d[0] + (m[1] - a[1]) * d[1]) / len).clamp(0.0, 1.0);
    [a[0] + d[0] * t, a[1] + d[1] * t]
}

fn picks(l: &Layout, layers: &Layers, m: P, tol: f64, ratsnest: bool) -> Vec<Pick> {
    let mut out = Vec::new();
    for (k, v) in l.vias.iter().enumerate().rev() {
        if v.layers.iter().any(|x| layers.shows(x)) && geom::dist(m, v.at) <= v.diameter / 2.0 + tol
        {
            out.push(Pick::Via(k));
        }
    }
    let shown: Vec<(usize, &Placed)> =
        l.parts.iter().enumerate().filter(|(_, p)| side_shown(p, layers)).collect();
    for (k, p) in &shown {
        if p.pads.iter().any(|q| q.outlines.iter().any(|o| geom::point_in_polygon(m, o))) {
            out.push(Pick::Part(*k));
        }
    }
    let mut tracks: Vec<(f64, Pick)> = Vec::new();
    for (k, t) in l.tracks.iter().enumerate().filter(|(_, t)| layers.shows(&t.layer)) {
        let reach = t.width / 2.0 + tol;
        let mut best: Option<(f64, Pick)> = None;
        for (s, w) in t.points.windows(2).enumerate() {
            let d = geom::point_segment_distance(m, w[0], w[1]);
            if d <= reach && best.is_none_or(|(b, _)| d < b) {
                let vertex = [s, s + 1].into_iter().find(|&j| geom::dist(m, t.points[j]) <= reach);
                best = Some((d, Pick::Track { index: k, segment: s, vertex }));
            }
        }
        tracks.extend(best);
    }
    tracks.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.extend(tracks.into_iter().map(|(_, p)| p));
    let mut bodies: Vec<(f64, usize)> = shown
        .iter()
        .filter(|(k, _)| !out.iter().any(|p| matches!(p, Pick::Part(x) if x == k)))
        .filter_map(|(k, p)| {
            let b = part_box(p);
            let [w, h] = b.size();
            inside(&b, m).then_some((w * h, *k))
        })
        .collect();
    bodies.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.extend(bodies.into_iter().map(|(_, k)| Pick::Part(k)));
    if ratsnest {
        let mut lines: Vec<(f64, usize)> = l
            .ratsnest
            .iter()
            .enumerate()
            .map(|(k, (a, b, _))| (geom::point_segment_distance(m, *a, *b), k))
            .filter(|(d, _)| *d <= tol)
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(lines.into_iter().map(|(_, k)| Pick::Ratsnest(k)));
    }
    out
}

fn is_selected(l: &Layout, p: Pick, sel: Option<&Sel>) -> bool {
    match (p, sel) {
        (Pick::Part(k), Some(Sel::Part(r))) => l.parts[k].reference == *r,
        (Pick::Track { index, .. }, Some(Sel::Track(t))) => l.tracks[index].source == *t,
        (Pick::Via(k), Some(Sel::Via(s, at))) => l.vias[k].source == *s && near(l.vias[k].at, *at),
        (Pick::Ratsnest(k), Some(Sel::Ratsnest(a, b, _))) => {
            near(l.ratsnest[k].0, *a) && near(l.ratsnest[k].1, *b)
        }
        _ => false,
    }
}

fn pick(
    l: &Layout,
    layers: &Layers,
    m: P,
    tol: f64,
    ratsnest: bool,
    sel: Option<&Sel>,
    cycle: bool,
) -> Option<Pick> {
    let all = picks(l, layers, m, tol, ratsnest);
    match all.iter().position(|p| is_selected(l, *p, sel)) {
        Some(k) if cycle => all.get((k + 1) % all.len()).copied(),
        Some(k) => all.get(k).copied(),
        None => all.first().copied(),
    }
}

struct Copper {
    net: usize,
    at: P,
    layer: Option<String>,
}

fn copper_at(l: &Layout, layers: &Layers, m: P, tol: f64, active: &str) -> Option<Copper> {
    for v in l.vias.iter().rev() {
        if v.layers.iter().any(|x| layers.shows(x)) && geom::dist(m, v.at) <= v.diameter / 2.0 + tol
        {
            let layer = v.layers.iter().any(|x| x == active).then(|| active.to_string());
            return Some(Copper { net: v.net, at: v.at, layer });
        }
    }
    for p in &l.parts {
        for pad in &p.pads {
            let Some(net) = pad.net else { continue };
            let shown: Vec<&String> = pad.copper.iter().filter(|c| layers.shows(c)).collect();
            if shown.is_empty() || !pad.outlines.iter().any(|o| geom::point_in_polygon(m, o)) {
                continue;
            }
            let layer = if pad.copper.iter().any(|c| c == active) {
                active.to_string()
            } else {
                shown[0].clone()
            };
            let at = pad.drill.map(|d| d.0).unwrap_or_else(|| pad_center(&pad.outlines));
            return Some(Copper { net, at, layer: Some(layer) });
        }
    }
    for t in l.tracks.iter().filter(|t| layers.shows(&t.layer)) {
        for w in t.points.windows(2) {
            if geom::point_segment_distance(m, w[0], w[1]) <= t.width / 2.0 + tol {
                let at = project_onto(m, w[0], w[1]);
                return Some(Copper { net: t.net, at, layer: Some(t.layer.clone()) });
            }
        }
    }
    for z in l.zones.iter().filter(|z| layers.shows(&z.layer)) {
        if z.filled(m) {
            return Some(Copper { net: z.net, at: m, layer: Some(z.layer.clone()) });
        }
    }
    None
}

pub fn route45(a: P, b: P, diagonal_first: bool) -> Vec<P> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let d = dx.abs().min(dy.abs());
    let corner = if diagonal_first {
        [a[0] + dx.signum() * d, a[1] + dy.signum() * d]
    } else if dx.abs() > dy.abs() {
        [b[0] - dx.signum() * d, a[1]]
    } else {
        [a[0], b[1] - dy.signum() * d]
    };
    let mut v = vec![a, corner, b];
    v.dedup_by(|x, y| near(*x, *y));
    v
}

fn draft_target(ed: &Editor, l: &Layout, layers: &Layers, m: P, tol: f64) -> (P, bool) {
    let Some(d) = &ed.draft else { return (ed.snap(m), false) };
    match copper_at(l, layers, m, tol, &d.layer) {
        Some(c) if c.net == d.net && c.layer.as_deref() == Some(d.layer.as_str()) => (c.at, true),
        _ => (ed.snap(m), false),
    }
}

pub struct Canvas<'a> {
    pub project: &'a Project,
    pub index: usize,
    pub layers: &'a Layers,
    pub ratsnest: bool,
}

impl Canvas<'_> {
    pub fn interact(&self, ui: &Ui, resp: &Response, xf: &Xf, view: &mut View, ed: &mut Editor) {
        let (project, i) = (self.project, self.index);
        let ctx = ui.ctx().clone();
        let tol = PICK_PX / xf.scale as f64;
        if resp.drag_started_by(PointerButton::Primary) {
            let origin = ui.input(|x| x.pointer.press_origin()).map(|p| xf.mm(p));
            ed.drag = Some(Drag::Pan);
            if ed.tool == Tool::Select
                && let Some(m) = origin
            {
                let l = ed.layout(project, i);
                let grabbed = match pick(l, self.layers, m, tol, false, ed.sel.as_ref(), false) {
                    Some(Pick::Part(k)) => {
                        let p = &l.parts[k];
                        let at = p.at.to_mm();
                        Some((
                            Sel::Part(p.reference.clone()),
                            Drag::Part {
                                reference: p.reference.clone(),
                                start: at,
                                grab: m,
                                now: at,
                            },
                        ))
                    }
                    Some(Pick::Track { index, segment, vertex }) => {
                        let t = &l.tracks[index];
                        let drag = match vertex {
                            Some(v) => Drag::Vertex {
                                track: t.source,
                                vertex: v,
                                points: t.points.clone(),
                            },
                            None => Drag::Segment {
                                track: t.source,
                                segment,
                                points: t.points.clone(),
                                grab: m,
                                now: t.points.clone(),
                            },
                        };
                        Some((Sel::Track(t.source), drag))
                    }
                    Some(Pick::Via(k)) => {
                        let v = &l.vias[k];
                        Some((
                            Sel::Via(v.source, v.at),
                            Drag::Via { source: v.source, start: v.at, grab: m, now: v.at },
                        ))
                    }
                    _ => None,
                };
                if let Some((sel, drag)) = grabbed {
                    ed.sel = Some(sel);
                    ed.drag = Some(drag);
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(m) = resp.interact_pointer_pos().map(|p| xf.mm(p))
        {
            self.drag_to(ed, view, resp.drag_delta(), m);
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.drop(&ctx, ed);
        }
        let hover = resp.hover_pos().map(|p| xf.mm(p));
        if resp.clicked_by(PointerButton::Primary)
            && let Some(m) = hover
        {
            match ed.tool {
                Tool::Select => {
                    let l = ed.layout(project, i);
                    ed.sel = pick(l, self.layers, m, tol, self.ratsnest, ed.sel.as_ref(), true)
                        .map(|p| match p {
                            Pick::Part(k) => Sel::Part(l.parts[k].reference.clone()),
                            Pick::Track { index, .. } => Sel::Track(l.tracks[index].source),
                            Pick::Via(k) => Sel::Via(l.vias[k].source, l.vias[k].at),
                            Pick::Ratsnest(k) => {
                                let (a, b, n) = l.ratsnest[k];
                                Sel::Ratsnest(a, b, n)
                            }
                        });
                }
                Tool::Route => self.route_click(&ctx, ed, m, tol),
                Tool::Via => {
                    let l = ed.layout(project, i);
                    match copper_at(l, self.layers, m, tol, &ed.layer) {
                        Some(c) => {
                            let at = if c.at == m { ed.snap(m) } else { c.at };
                            ed.place_via(&ctx, project, i, c.net, at);
                        }
                        None => ed.note = Some("place a via on copper that has a net".into()),
                    }
                }
            }
        }
        if resp.double_clicked_by(PointerButton::Primary) && ed.tool == Tool::Route {
            ed.finish_draft(&ctx, project, i);
        }
    }

    fn drag_to(&self, ed: &mut Editor, view: &mut View, delta: Vec2, m: P) {
        let (project, i) = (self.project, self.index);
        let Some(drag) = ed.drag.clone() else { return };
        match drag {
            Drag::Pan => view.pan(delta),
            Drag::Part { reference, start, grab, .. } => {
                let to = ed.snap([start[0] + m[0] - grab[0], start[1] + m[1] - grab[1]]);
                drag_part(ed.shown_mut(project, i), &reference, to);
                ed.drag = Some(Drag::Part { reference, start, grab, now: to });
            }
            Drag::Vertex { track, vertex, points } => {
                let mut p = points.clone();
                p[vertex] = ed.snap(m);
                drag_track(ed.shown_mut(project, i), track, &p);
            }
            Drag::Segment { track, segment, points, grab, .. } => {
                let (a, b) = (ed.snap(m), ed.snap(grab));
                let mut p = points.clone();
                for q in &mut p[segment..=segment + 1] {
                    *q = [q[0] + a[0] - b[0], q[1] + a[1] - b[1]];
                }
                drag_track(ed.shown_mut(project, i), track, &p);
                ed.drag = Some(Drag::Segment { track, segment, points, grab, now: p });
            }
            Drag::Via { source, start, grab, now } => {
                let to = ed.snap([start[0] + m[0] - grab[0], start[1] + m[1] - grab[1]]);
                drag_via(ed.shown_mut(project, i), source, now, to);
                ed.drag = Some(Drag::Via { source, start, grab, now: to });
            }
        }
    }

    fn drop(&self, ctx: &egui::Context, ed: &mut Editor) {
        let (project, i) = (self.project, self.index);
        match ed.drag.take() {
            Some(Drag::Part { reference, start, now, .. }) if !near(start, now) => {
                ed.move_part(ctx, project, i, &reference, Some(start), now);
            }
            Some(Drag::Vertex { track, points, .. } | Drag::Segment { track, points, .. }) => {
                let now = ed
                    .layout(project, i)
                    .tracks
                    .iter()
                    .find(|t| t.source == track)
                    .map(|t| t.points.clone());
                if let Some(now) = now
                    && now != points
                {
                    ed.track_points(ctx, project, i, track, now);
                }
            }
            Some(Drag::Via { source, start, now, .. }) if !near(start, now) => {
                drag_via(ed.shown_mut(project, i), source, now, start);
                ed.move_via(ctx, project, i, source, start, now);
            }
            _ => {}
        }
    }

    fn route_click(&self, ctx: &egui::Context, ed: &mut Editor, m: P, tol: f64) {
        let (project, i) = (self.project, self.index);
        let l = ed.layout(project, i);
        if ed.draft.is_none() {
            match copper_at(l, self.layers, m, tol, &ed.layer) {
                Some(c) => {
                    let layer = c.layer.unwrap_or_else(|| ed.layer.clone());
                    ed.layer = layer.clone();
                    ed.draft = Some(Draft {
                        net: c.net,
                        layer,
                        runs: Vec::new(),
                        points: vec![c.at],
                        vias: Vec::new(),
                        diagonal_first: false,
                    });
                    ed.sel = None;
                    ed.note = None;
                }
                None => ed.note = Some("start a track on a pad, via or track".into()),
            }
            return;
        }
        let (target, done) = draft_target(ed, l, self.layers, m, tol);
        let Some(d) = &mut ed.draft else { return };
        let Some(last) = d.points.last().copied() else { return };
        d.points.extend(route45(last, target, d.diagonal_first).into_iter().skip(1));
        if done && d.points.len() > 1 {
            ed.finish_draft(ctx, project, i);
        }
    }

    pub fn draft_via(&self, ed: &mut Editor, hover: Option<P>, tol: f64) {
        let (project, i) = (self.project, self.index);
        let Some(d) = &ed.draft else { return };
        let (net, layer) = (d.net, d.layer.clone());
        let Some(next) = ed.reach(project, i, net, &layer) else {
            ed.note = Some(format!("no via of this net's class leaves {layer}"));
            return;
        };
        let l = ed.layout(project, i);
        let target = hover.map(|m| draft_target(ed, l, self.layers, m, tol).0);
        let Some(d) = &mut ed.draft else { return };
        let Some(last) = d.points.last().copied() else { return };
        let at = target.unwrap_or(last);
        d.points.extend(route45(last, at, d.diagonal_first).into_iter().skip(1));
        let run = std::mem::replace(&mut d.points, vec![at]);
        if run.len() >= 2 {
            d.runs.push((layer, run));
        }
        d.vias.push(at);
        d.layer = next.clone();
        ed.layer = next;
    }

    pub fn keys(&self, ui: &Ui, ed: &mut Editor, hover: Option<P>, xf: &Xf, hovered: bool) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (project, i) = (self.project, self.index);
        let ctx = ui.ctx().clone();
        let tol = PICK_PX / xf.scale as f64;
        let key = |m: Modifiers, k: Key| ui.input_mut(|x| x.consume_key(m, k));
        let shift = Modifiers::SHIFT;
        let cmd = Modifiers::COMMAND;
        if key(cmd | shift, Key::Z) || key(cmd, Key::Y) {
            ed.redo(&ctx);
        } else if key(cmd, Key::Z) {
            ed.undo(&ctx);
        }
        if key(cmd, Key::S)
            && let Err(e) = ed.save()
        {
            ed.error = Some(e);
        }
        if !hovered && ed.draft.is_none() {
            return;
        }
        if key(Modifiers::NONE, Key::Escape) {
            if ed.draft.is_some() {
                ed.draft = None;
            } else if ed.tool != Tool::Select {
                ed.tool = Tool::Select;
            } else {
                ed.sel = None;
            }
        }
        if key(Modifiers::NONE, Key::X) {
            ed.tool = Tool::Route;
        }
        if key(Modifiers::NONE, Key::V) {
            if ed.draft.is_some() {
                self.draft_via(ed, hover, tol);
            } else {
                ed.tool = Tool::Via;
            }
        }
        if key(Modifiers::NONE, Key::Slash)
            && let Some(d) = &mut ed.draft
        {
            d.diagonal_first = !d.diagonal_first;
        }
        if key(Modifiers::NONE, Key::Enter) {
            ed.finish_draft(&ctx, project, i);
        }
        if ed.draft.is_none() {
            let copper = ed.layout(project, i).copper.clone();
            let at = copper.iter().position(|c| *c == ed.layer).unwrap_or(0);
            let n = copper.len().max(1);
            if key(Modifiers::NONE, Key::PageDown) {
                ed.layer = copper.get((at + 1) % n).cloned().unwrap_or_default();
            }
            if key(Modifiers::NONE, Key::PageUp) {
                ed.layer = copper.get((at + n - 1) % n).cloned().unwrap_or_default();
            }
        }
        let back = key(Modifiers::NONE, Key::Backspace);
        if let Some(d) = &mut ed.draft {
            if back {
                if d.points.len() > 1 {
                    d.points.pop();
                } else if let Some((layer, pts)) = d.runs.pop() {
                    d.vias.pop();
                    d.layer = layer;
                    d.points = pts;
                } else {
                    ed.draft = None;
                }
            }
            return;
        }
        if back || key(Modifiers::NONE, Key::Delete) {
            delete(&ctx, project, i, ed);
        }
        let turn = if key(shift, Key::R) {
            -90.0
        } else if key(Modifiers::NONE, Key::R) {
            90.0
        } else {
            0.0
        };
        if turn != 0.0
            && let Some(Sel::Part(r)) = ed.sel.clone()
            && let Some(p) = ed.layout(project, i).parts.iter().find(|p| p.reference == r)
        {
            let (rot, bottom) = (p.rotation, p.bottom);
            let locked = ed.locked(&r);
            ed.set_part(&ctx, project, i, &r, rot + turn, bottom, locked);
        }
    }

    pub fn overlay(&self, p: &egui::Painter, xf: &Xf, ed: &Editor, hover: Option<P>) {
        let l = ed.layout(self.project, self.index);
        let hi = READOUT;
        match &ed.sel {
            Some(Sel::Part(r)) => {
                if let Some(part) = l.parts.iter().find(|p| &p.reference == r) {
                    let b = part_box(part);
                    let rect = Rect::from_two_pos(xf.world(b.min), xf.world(b.max)).expand(3.0);
                    p.rect_stroke(rect, 2.0, Stroke::new(1.5, hi), egui::StrokeKind::Outside);
                    for pad in &part.pads {
                        for o in &pad.outlines {
                            let pts = o.iter().map(|q| xf.world(*q)).collect();
                            p.add(PathShape::closed_line(pts, Stroke::new(1.0, hi)));
                        }
                    }
                }
            }
            Some(Sel::Track(t)) => {
                if let Some(t) = l.tracks.iter().find(|x| x.source == *t) {
                    let pts: Vec<Pos2> = t.points.iter().map(|q| xf.world(*q)).collect();
                    let w = xf.len(t.width).max(1.0) + 4.0;
                    for s in pts.windows(2) {
                        p.line_segment([s[0], s[1]], Stroke::new(w, hi.gamma_multiply(0.35)));
                    }
                    p.add(PathShape::line(pts.clone(), Stroke::new(1.0, hi)));
                    for q in &pts {
                        p.rect_filled(Rect::from_center_size(*q, Vec2::splat(6.0)), 0.0, hi);
                    }
                }
            }
            Some(Sel::Via(s, at)) => {
                if let Some(v) = l.vias.iter().find(|v| v.source == *s && near(v.at, *at)) {
                    let r = xf.len(v.diameter / 2.0) + 3.0;
                    p.circle_stroke(xf.world(v.at), r, Stroke::new(1.5, hi));
                }
            }
            Some(Sel::Ratsnest(a, b, _)) => {
                p.line_segment([xf.world(*a), xf.world(*b)], Stroke::new(3.0, hi));
            }
            None => {}
        }
        let Some(d) = &ed.draft else { return };
        let width = l.nets.get(d.net).map(|n| n.width).unwrap_or(0.2);
        let draw = |layer: &str, pts: &[P], alpha: f32| {
            let c = copper_color(layer).gamma_multiply(alpha);
            let s: Vec<Pos2> = pts.iter().map(|q| xf.world(*q)).collect();
            let w = xf.len(width).max(1.5);
            for seg in s.windows(2) {
                p.line_segment([seg[0], seg[1]], Stroke::new(w, c));
            }
            for q in &s {
                p.circle_filled(*q, w / 2.0, c);
            }
        };
        for (layer, pts) in &d.runs {
            draw(layer, pts, 0.9);
        }
        draw(&d.layer, &d.points, 0.9);
        if let (Some(m), Some(last)) = (hover, d.points.last()) {
            let tol = PICK_PX / xf.scale as f64;
            let (target, done) = draft_target(ed, l, self.layers, m, tol);
            draw(&d.layer, &route45(*last, target, d.diagonal_first), 0.55);
            if done {
                p.circle_stroke(xf.world(target), 6.0, Stroke::new(1.5, OK));
            }
        }
        for v in &d.vias {
            p.circle_filled(xf.world(*v), xf.len(width * 1.5).max(4.0), crate::paint::PTH);
        }
    }
}

fn delete(ctx: &egui::Context, project: &Project, i: usize, ed: &mut Editor) {
    match ed.sel.clone() {
        Some(Sel::Track(t)) => ed.delete_track(ctx, project, i, t),
        Some(Sel::Via(s, at)) => ed.delete_via(ctx, project, i, s, at),
        Some(Sel::Part(_)) => {
            ed.note = Some("parts come from the schematic, remove them there".into());
        }
        _ => {}
    }
}

pub fn hint(ui: &Ui, rect: Rect, ed: &Editor) {
    let text = match (ed.tool, &ed.draft) {
        (Tool::Select, _) => {
            "click to select, drag to move, right or middle drag pans, R rotates, Del deletes, X routes, V places vias"
        }
        (Tool::Route, None) => "click a pad, via or track to start, PgUp/PgDn picks the layer",
        (Tool::Route, Some(_)) => {
            "click adds a corner, V drops a via, / flips the bend, Backspace steps back, Enter or double click ends"
        }
        (Tool::Via, _) => "click copper to place a via of its net",
    };
    let p = ui.painter_at(rect);
    let mut at = rect.left_top() + Vec2::new(10.0, 8.0);
    for (line, col) in
        [(Some(text.to_string()), LEGEND), (ed.note.clone(), WARN), (ed.error.clone(), FAULT)]
    {
        if let Some(line) = line {
            p.text(at, Align2::LEFT_TOP, line, egui::FontId::proportional(12.0), col);
            at.y += 16.0;
        }
    }
}

pub fn toolbar(ui: &mut Ui, project: &Project, i: usize, ed: &mut Editor) {
    let ctx = ui.ctx().clone();
    ui.add_space(12.0);
    for (tool, name) in [(Tool::Select, "select"), (Tool::Route, "route"), (Tool::Via, "via")] {
        if toggle(ui, name, ed.tool == tool).clicked() {
            ed.tool = tool;
            ed.draft = None;
        }
    }
    ui.add_space(8.0);
    let copper = ed.layout(project, i).copper.clone();
    let mut layer = ed.layer.clone();
    egui::ComboBox::from_id_salt("active-layer").width(76.0).selected_text(&layer).show_ui(
        ui,
        |ui| {
            for c in &copper {
                ui.selectable_value(&mut layer, c.clone(), c);
            }
        },
    );
    if layer != ed.layer && ed.draft.is_none() {
        ed.layer = layer;
    }
    let mut grid = ed.grid;
    egui::ComboBox::from_id_salt("grid")
        .width(70.0)
        .selected_text(format!("{} mm", trim(grid, 3)))
        .show_ui(ui, |ui| {
            for g in GRIDS {
                ui.selectable_value(&mut grid, g, format!("{} mm", trim(g, 3)));
            }
        });
    ed.grid = grid;
    if toggle(ui, "snap", ed.snap).clicked() {
        ed.snap = !ed.snap;
    }
    ui.add_space(8.0);
    if ui.add_enabled(ed.can_undo(), egui::Button::new("undo")).clicked() {
        ed.undo(&ctx);
    }
    if ui.add_enabled(ed.can_redo(), egui::Button::new("redo")).clicked() {
        ed.redo(&ctx);
    }
    if ui.add_enabled(ed.dirty, egui::Button::new("save")).on_hover_text("ctrl+S").clicked()
        && let Err(e) = ed.save()
    {
        ed.error = Some(e);
    }
    if ui.add_enabled(ed.dirty, egui::Button::new("revert")).clicked() {
        ed.revert(&ctx);
    }
    if ed.dirty {
        lamp(ui, "unsaved", false, false);
    }
    if ed.checking() {
        lamp(ui, "checking", true, false);
    }
    if ed.routing() {
        lamp(ui, "routing", true, false);
    }
}

pub fn conflict(ui: &mut Ui, ed: &mut Editor) {
    if ed.conflict.is_none() {
        return;
    }
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        status(ui, false, "the layout changed on disk while you have unsaved edits");
        if ui.button("keep mine").clicked() {
            ed.keep_mine();
        }
        if ui.button("load the file").clicked() {
            ed.take_theirs(&ctx);
        }
    });
}

fn mm_drag(ui: &mut Ui, v: &mut f64) -> bool {
    ui.add(
        egui::DragValue::new(v)
            .speed(0.01)
            .max_decimals(4)
            .suffix(" mm")
            .update_while_editing(false),
    )
    .changed()
}

struct Names {
    nets: Vec<String>,
    copper: Vec<String>,
}

pub fn selection(ui: &mut Ui, project: &Project, i: usize, ed: &mut Editor) {
    let Some(sel) = ed.sel.clone() else { return };
    let ctx = ui.ctx().clone();
    let l = ed.layout(project, i);
    let names =
        Names { nets: l.nets.iter().map(|n| n.name.clone()).collect(), copper: l.copper.clone() };
    let title = match &sel {
        Sel::Part(r) => format!("part {r}"),
        Sel::Track(t) => format!("tracks[{t}]"),
        Sel::Via(s, _) => ed.origin_of(*s),
        Sel::Ratsnest(..) => "unrouted connection".into(),
    };
    card(
        ui,
        Some(READOUT),
        |ui| {
            Line::new().legend("selected").value(&title).elided(ui);
        },
        |ui| match sel {
            Sel::Part(r) => part_props(ui, &ctx, project, i, ed, &r),
            Sel::Track(t) => track_props(ui, &ctx, project, i, ed, &names, t),
            Sel::Via(s, at) => via_props(ui, &ctx, project, i, ed, &names, s, at),
            Sel::Ratsnest(a, b, n) => {
                reading(ui, "net", names.nets[n].clone());
                reading(ui, "from", format!("{}, {}", trim(a[0], 3), trim(a[1], 3)));
                reading(ui, "to", format!("{}, {}", trim(b[0], 3), trim(b[1], 3)));
                reading(ui, "length", format!("{} mm", trim(geom::dist(a, b), 2)));
                ui.horizontal(|ui| {
                    let idle = !ed.routing();
                    if ui.add_enabled(idle, egui::Button::new("route connection")).clicked() {
                        ed.route_connection(&ctx, project, i, n, Some((a, b)));
                    }
                    if ui.add_enabled(idle, egui::Button::new("route net")).clicked() {
                        ed.route_connection(&ctx, project, i, n, None);
                    }
                });
            }
        },
    );
    ui.add_space(8.0);
}

fn part_props(
    ui: &mut Ui,
    ctx: &egui::Context,
    project: &Project,
    i: usize,
    ed: &mut Editor,
    r: &str,
) {
    let Some(p) = ed.layout(project, i).parts.iter().find(|p| p.reference == r) else { return };
    let (value, footprint) = (p.value.clone(), p.footprint_name.clone());
    let (at, rotation, side) = (p.at.to_mm(), p.rotation, p.bottom);
    reading(ui, "value", value);
    reading(ui, "footprint", footprint);
    let mut xy = at;
    row(ui, "x", |ui| {
        mm_drag(ui, &mut xy[0]);
    });
    row(ui, "y", |ui| {
        mm_drag(ui, &mut xy[1]);
    });
    if xy != at {
        ed.move_part(ctx, project, i, r, None, xy);
        return;
    }
    let (mut rot, mut bottom, mut locked) = (rotation, side, ed.locked(r));
    row(ui, "rotation", |ui| {
        ui.add(
            egui::DragValue::new(&mut rot)
                .speed(1.0)
                .range(-360.0..=360.0)
                .suffix("°")
                .update_while_editing(false),
        );
        if ui.button("-90").clicked() {
            rot -= 90.0;
        }
        if ui.button("+90").clicked() {
            rot += 90.0;
        }
    });
    row(ui, "side", |ui| {
        if toggle(ui, "top", !bottom).clicked() {
            bottom = false;
        }
        if toggle(ui, "bottom", bottom).clicked() {
            bottom = true;
        }
    });
    row(ui, "locked", |ui| {
        ui.checkbox(&mut locked, "the placer leaves it");
    });
    if rot != rotation || bottom != side || locked != ed.locked(r) {
        ed.set_part(ctx, project, i, r, rot, bottom, locked);
    }
}

fn net_choice(ui: &mut Ui, id: &str, names: &Names, net: &mut usize) {
    choice(ui, id, net, names.nets.iter().cloned().enumerate());
}

fn track_props(
    ui: &mut Ui,
    ctx: &egui::Context,
    project: &Project,
    i: usize,
    ed: &mut Editor,
    names: &Names,
    t: usize,
) {
    let l = ed.layout(project, i);
    let Some(tr) = l.tracks.iter().find(|x| x.source == t).cloned() else { return };
    let class = l.nets[tr.net].class.clone();
    let length: f64 = tr.points.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
    readouts(
        ui,
        &[
            ("points", tr.points.len().to_string(), VALUE),
            ("length", format!("{} mm", trim(length, 2)), TRACE),
            ("class", class, LEGEND),
        ],
    );
    let (mut net, mut layer) = (tr.net, tr.layer.clone());
    let own = ed.track_width_set(t);
    let (mut custom, mut width) = (own, tr.width);
    row(ui, "net", |ui| net_choice(ui, "track-net", names, &mut net));
    row(ui, "layer", |ui| {
        choice(ui, "track-layer", &mut layer, names.copper.iter().map(|c| (c.clone(), c.clone())));
    });
    row(ui, "width", |ui| {
        ui.checkbox(&mut custom, "own");
        ui.add_enabled_ui(custom, |ui| mm_drag(ui, &mut width));
    });
    if net != tr.net || layer != tr.layer || custom != own || (custom && width != tr.width) {
        ed.set_track(ctx, project, i, t, net, &layer, custom.then_some(width));
    }
    if ui.button("delete track").clicked() {
        ed.delete_track(ctx, project, i, t);
    }
}

#[allow(clippy::too_many_arguments)]
fn via_props(
    ui: &mut Ui,
    ctx: &egui::Context,
    project: &Project,
    i: usize,
    ed: &mut Editor,
    names: &Names,
    s: ViaSource,
    at: P,
) {
    let found = ed.layout(project, i).vias.iter().find(|v| v.source == s && near(v.at, at));
    let Some(v) = found.cloned() else { return };
    readouts(
        ui,
        &[
            ("drill", format!("{} mm", trim(v.drill, 3)), VALUE),
            ("pad", format!("{} mm", trim(v.diameter, 3)), VALUE),
            (
                "layers",
                format!(
                    "{}-{}",
                    v.layers.first().cloned().unwrap_or_default(),
                    v.layers.last().cloned().unwrap_or_default()
                ),
                LEGEND,
            ),
        ],
    );
    let detaches = !matches!(s, ViaSource::File { .. }) || ed.is_array(s);
    if detaches {
        note(
            ui,
            "an edit takes this via out of its rule and writes it as its own [[vias]]",
            LEGEND,
        );
    }
    let mut xy = v.at;
    row(ui, "x", |ui| {
        mm_drag(ui, &mut xy[0]);
    });
    row(ui, "y", |ui| {
        mm_drag(ui, &mut xy[1]);
    });
    if xy != v.at {
        ed.move_via(ctx, project, i, s, v.at, xy);
        return;
    }
    let mut net = v.net;
    row(ui, "net", |ui| net_choice(ui, "via-net", names, &mut net));
    let named = ed.via_named(s);
    let mut kind = named.clone();
    let names = ed.via_choices(project, i);
    row(ui, "type", |ui| {
        let options = std::iter::once((None, format!("class ({})", v.name)))
            .chain(names.iter().map(|n| (Some(n.clone()), n.clone())));
        choice(ui, "via-type", &mut kind, options);
    });
    if net != v.net || kind != named {
        ed.set_via(ctx, project, i, s, v.at, net, kind);
    }
    if ui.button("delete via").clicked() {
        ed.delete_via(ctx, project, i, s, v.at);
    }
}

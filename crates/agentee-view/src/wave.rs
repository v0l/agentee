use agentee_core::logic::{LogicResult, Mark, MarkKind, Trace, fmt_time, ps};
use agentee_core::sim::Sim;
use egui::{Align2, Color32, Key, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use egui_bench::prelude::*;

const ROW: f32 = 24.0;
const LABEL_W: f32 = 120.0;
const VALUE_W: f32 = 70.0;
const STRIP: f32 = 14.0;
const MIN_SPAN: u64 = 10;
const CONTENTION: Color32 = Color32::from_rgb(0xC0, 0x7A, 0xE0);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveView {
    pub key: String,
    pub from: u64,
    pub to: u64,
    pub cursor: Option<u64>,
    pub open: Vec<String>,
}

impl WaveView {
    pub fn window(&mut self, key: &str, end: u64) -> (u64, u64) {
        if self.key != key {
            *self = WaveView { key: key.to_string(), ..Default::default() };
        }
        if self.to == 0 || self.to > end {
            self.to = end;
        }
        if self.from + MIN_SPAN.min(end) > self.to {
            self.from = self.to.saturating_sub(MIN_SPAN.min(end));
        }
        (self.from, self.to)
    }

    pub fn fit(&mut self, end: u64) {
        self.from = 0;
        self.to = end;
    }

    pub fn zoom(&mut self, at: u64, factor: f64, end: u64) {
        let span = (self.to - self.from) as f64;
        let new = (span * factor).round().clamp(MIN_SPAN.min(end) as f64, end as f64);
        let frac = (at.clamp(self.from, self.to) - self.from) as f64 / span.max(1.0);
        let from = (at as f64 - frac * new).round().clamp(0.0, end as f64 - new);
        self.from = from as u64;
        self.to = self.from + new as u64;
    }

    pub fn pan(&mut self, dt: f64, end: u64) {
        let span = self.to - self.from;
        let from = (self.from as f64 + dt).round().clamp(0.0, (end - span) as f64);
        self.from = from as u64;
        self.to = self.from + span;
    }

    pub fn show(&mut self, t: u64, end: u64) {
        if t < self.from || t > self.to {
            let span = self.to - self.from;
            self.from = t.saturating_sub(span / 2).min(end - span);
            self.to = self.from + span;
        }
    }

    pub fn read(&mut self, key: &str, words: &[String]) {
        self.key = key.to_string();
        for w in words {
            let Some((k, v)) = w.split_once('=') else { continue };
            match k.trim() {
                "from" => self.from = ps(v).unwrap_or(self.from),
                "to" => self.to = ps(v).unwrap_or(self.to),
                "cursor" => self.cursor = ps(v).or(self.cursor),
                "open" => self.open.push(v.trim().to_string()),
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Bit { trace: usize, inner: bool },
    Bus { name: String, bits: Vec<usize>, open: bool },
}

fn bus_bit(name: &str) -> Option<(String, u32, String)> {
    if let Some(stem) = name.strip_suffix(']')
        && let Some((prefix, idx)) = stem.rsplit_once('[')
        && !prefix.is_empty()
        && let Ok(i) = idx.parse()
    {
        return Some((prefix.to_string(), i, String::new()));
    }
    let end = name.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = name[..end].rfind(|c: char| !c.is_ascii_digit())? + 1;
    let suffix = &name[end..];
    if !["", "_N", "_n", "#"].contains(&suffix) {
        return None;
    }
    let i = name[start..end].parse().ok()?;
    Some((name[..start].to_string(), i, suffix.to_string()))
}

type AutoBus = ((String, String), Vec<(u32, usize)>);

pub fn rows(traces: &[Trace], buses: &[agentee_core::logic::Bus], open: &[String]) -> Vec<Row> {
    let find = |n: &str| traces.iter().position(|t| t.name == n);
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut used = vec![false; traces.len()];
    for b in buses {
        let bits: Option<Vec<usize>> = b.nets.iter().map(|n| find(n)).collect();
        if let Some(bits) = bits
            && !bits.is_empty()
        {
            bits.iter().for_each(|k| used[*k] = true);
            groups.push((b.name.clone(), bits));
        }
    }
    let mut auto: Vec<AutoBus> = Vec::new();
    for (k, t) in traces.iter().enumerate() {
        if used[k] {
            continue;
        }
        let Some((prefix, i, suffix)) = bus_bit(&t.name) else { continue };
        let key = (prefix, suffix);
        match auto.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push((i, k)),
            None => auto.push((key, vec![(i, k)])),
        }
    }
    for ((prefix, suffix), mut bits) in auto {
        bits.sort_by_key(|b| std::cmp::Reverse(b.0));
        let distinct = bits.windows(2).all(|w| w[0].0 != w[1].0);
        if bits.len() < 2 || !distinct {
            continue;
        }
        let (hi, lo) = (bits[0].0, bits[bits.len() - 1].0);
        bits.iter().for_each(|b| used[b.1] = true);
        let name = format!("{prefix}[{hi}:{lo}]{suffix}");
        groups.push((name, bits.into_iter().map(|b| b.1).collect()));
    }
    let mut out = Vec::new();
    for (k, grouped) in used.iter().enumerate() {
        if !grouped {
            out.push(Row::Bit { trace: k, inner: false });
            continue;
        }
        for (name, bits) in &groups {
            if bits.iter().min() == Some(&k) {
                let is_open = open.contains(name);
                out.push(Row::Bus { name: name.clone(), bits: bits.clone(), open: is_open });
                if is_open {
                    out.extend(bits.iter().map(|b| Row::Bit { trace: *b, inner: true }));
                }
            }
        }
    }
    out
}

pub fn hex(bits: &[char]) -> String {
    let first = match bits.len() % 4 {
        0 => 4,
        n => n,
    };
    let mut out = String::new();
    let mut at = 0;
    let mut take = first.min(bits.len());
    while at < bits.len() {
        let nib = &bits[at..at + take];
        let c = if nib.iter().all(|c| *c == '0' || *c == '1') {
            let v = nib.iter().fold(0u32, |a, c| a << 1 | (*c == '1') as u32);
            char::from_digit(v, 16).unwrap_or('?').to_ascii_uppercase()
        } else if nib.iter().all(|c| *c == 'z') {
            'z'
        } else {
            'x'
        };
        out.push(c);
        at += take;
        take = 4;
    }
    out
}

fn value_at(tr: &Trace, t: u64) -> char {
    let k = tr.times.partition_point(|x| *x <= t);
    if k == 0 { 'z' } else { tr.values.chars().nth(k - 1).unwrap_or('x') }
}

fn bit_spans(tr: &Trace) -> Vec<(u64, char)> {
    let mut spans: Vec<(u64, char)> = vec![(0, 'z')];
    for (t, c) in tr.times.iter().zip(tr.values.chars()) {
        if *t == 0 {
            spans[0].1 = c;
        } else {
            spans.push((*t, c));
        }
    }
    spans
}

pub fn bus_spans(traces: &[Trace], bits: &[usize]) -> Vec<(u64, String)> {
    let mut times: Vec<u64> = bits.iter().flat_map(|b| traces[*b].times.iter().copied()).collect();
    times.push(0);
    times.sort_unstable();
    times.dedup();
    let mut out: Vec<(u64, String)> = Vec::new();
    for t in times {
        let v: Vec<char> = bits.iter().map(|b| value_at(&traces[*b], t)).collect();
        let h = hex(&v);
        if out.last().map(|x| &x.1) != Some(&h) {
            out.push((t, h));
        }
    }
    out
}

fn row_value(traces: &[Trace], row: &Row, t: u64) -> String {
    match row {
        Row::Bit { trace, .. } => value_at(&traces[*trace], t).to_string(),
        Row::Bus { bits, .. } => {
            let v: Vec<char> = bits.iter().map(|b| value_at(&traces[*b], t)).collect();
            hex(&v)
        }
    }
}

fn mark_colour(k: MarkKind) -> Color32 {
    match k {
        MarkKind::Assertion => FAULT,
        MarkKind::Timing => WARN,
        MarkKind::Contention => CONTENTION,
    }
}

fn row_has(traces: &[Trace], row: &Row, nets: &[String]) -> bool {
    match row {
        Row::Bit { trace, .. } => nets.contains(&traces[*trace].name),
        Row::Bus { bits, .. } => bits.iter().any(|b| nets.contains(&traces[*b].name)),
    }
}

fn edges_of(traces: &[Trace], row: &Row) -> Vec<u64> {
    match row {
        Row::Bit { trace, .. } => traces[*trace].times.clone(),
        Row::Bus { bits, .. } => bits.iter().flat_map(|b| traces[*b].times.clone()).collect(),
    }
}

fn keys(ui: &Ui, view: &mut WaveView, marks: &[Mark], end: u64) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
    let span = (view.to - view.from) as f64;
    let centre = view.cursor.filter(|c| *c >= view.from && *c <= view.to);
    let centre = centre.unwrap_or(view.from + (view.to - view.from) / 2);
    if pressed(Key::Plus) || pressed(Key::Equals) {
        view.zoom(centre, 0.5, end);
    }
    if pressed(Key::Minus) {
        view.zoom(centre, 2.0, end);
    }
    if pressed(Key::ArrowLeft) {
        view.pan(-span / 8.0, end);
    }
    if pressed(Key::ArrowRight) {
        view.pan(span / 8.0, end);
    }
    if pressed(Key::F) || pressed(Key::Home) {
        view.fit(end);
    }
    if pressed(Key::Escape) {
        view.cursor = None;
    }
    let next = pressed(Key::N);
    if next || pressed(Key::P) {
        let now = view.cursor.unwrap_or(if next { 0 } else { end });
        let hit = if next {
            marks.iter().map(|m| m.time).find(|t| *t > now || view.cursor.is_none())
        } else {
            marks.iter().rev().map(|m| m.time).find(|t| *t < now)
        };
        if let Some(t) = hit {
            view.cursor = Some(t);
            view.show(t, end);
        }
    }
}

pub fn canvas(ui: &mut Ui, r: &LogicResult, view: &mut WaveView, interactive: bool) {
    let end = r.end_ps.max(1);
    let (mut from, mut to) = view.window(&r.name, end);
    let rows = rows(&r.traces, &r.buses, &view.open);
    let sub = format!(
        "{} nets, {} to {} of {}, {} markers; wheel zooms, drag pans, click sets the cursor, N and P step markers, F fits",
        r.traces.len(),
        fmt_time(from),
        fmt_time(to),
        fmt_time(end),
        r.marks.len()
    );
    section(ui, "waveforms", &sub, |ui| {
        let height = (rows.len() as f32 * ROW + 34.0 + STRIP).max(120.0);
        let sense = if interactive { Sense::click_and_drag() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), sense);
        let area = Rect::from_min_max(
            rect.min + Vec2::new(LABEL_W + VALUE_W, 8.0 + STRIP),
            rect.max - Vec2::new(14.0, 26.0),
        );
        let t_at = |x: f32, from: u64, to: u64| -> u64 {
            let f = ((x - area.left()) / area.width()).clamp(0.0, 1.0) as f64;
            from + (f * (to - from) as f64).round() as u64
        };
        let row_at = |y: f32| -> Option<usize> {
            let k = ((y - area.top()) / ROW).floor();
            (k >= 0.0 && (k as usize) < rows.len()).then_some(k as usize)
        };
        if interactive {
            let px = (to - from) as f64 / area.width().max(1.0) as f64;
            if let Some(hp) = resp.hover_pos() {
                let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
                let at = t_at(hp.x, from, to);
                if scroll.y != 0.0 {
                    view.zoom(at, (-scroll.y as f64 * 0.004).exp(), end);
                }
                if scroll.x != 0.0 {
                    view.pan(-scroll.x as f64 * px, end);
                }
                if zoom != 1.0 {
                    view.zoom(at, 1.0 / zoom as f64, end);
                }
            }
            if resp.dragged_by(PointerButton::Primary) {
                view.pan(-resp.drag_delta().x as f64 * px, end);
            }
            if resp.double_clicked() {
                view.fit(end);
            } else if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                if p.x < area.left() {
                    if let Some(Row::Bus { name, .. }) = row_at(p.y).map(|k| &rows[k]) {
                        match view.open.iter().position(|n| n == name) {
                            Some(k) => {
                                view.open.remove(k);
                            }
                            None => view.open.push(name.clone()),
                        }
                    }
                } else if p.y < area.top() {
                    let t = t_at(p.x, from, to);
                    let near = r.marks.iter().map(|m| m.time).min_by_key(|m| m.abs_diff(t));
                    view.cursor = near.filter(|m| (m.abs_diff(t) as f64) < 8.0 * px).or(Some(t));
                } else {
                    let t = t_at(p.x, from, to);
                    let snap = row_at(p.y).and_then(|k| {
                        edges_of(&r.traces, &rows[k])
                            .into_iter()
                            .filter(|e| (*e as f64 - t as f64).abs() < 6.0 * px)
                            .min_by_key(|e| e.abs_diff(t))
                    });
                    view.cursor = Some(snap.unwrap_or(t));
                }
            }
            keys(ui, view, &r.marks, end);
            (from, to) = (view.from, view.to);
        }
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, WELL);
        let span = (to - from).max(1) as f64;
        let x = |t: u64| {
            let f = (t.clamp(from, to) - from) as f64 / span;
            area.left() + f as f32 * area.width()
        };
        let grid = Stroke::new(1.0, ETCH);
        let step = tick_step(to - from, (area.width() / 80.0).max(2.0) as u64);
        let mut t = from.div_ceil(step) * step;
        while t <= to {
            let gx = x(t);
            p.line_segment([Pos2::new(gx, area.top()), Pos2::new(gx, area.bottom())], grid);
            p.text(
                Pos2::new(gx, area.bottom() + 6.0),
                Align2::CENTER_TOP,
                fmt_time(t),
                theme::figure(10.5),
                LEGEND,
            );
            t += step;
        }
        let strip = Rect::from_min_max(
            Pos2::new(area.left(), area.top() - STRIP),
            Pos2::new(area.right(), area.top() - 2.0),
        );
        p.rect_filled(strip, 0.0, BAND);
        let hover = resp.hover_pos().filter(|h| area.contains(*h) || strip.contains(*h));
        let readout_t = view.cursor.or(hover.map(|h| t_at(h.x, from, to))).unwrap_or(to);
        let readout_colour = if view.cursor.is_some() { READOUT } else { LEGEND };
        p.text(
            Pos2::new(rect.left() + LABEL_W + VALUE_W - 6.0, strip.center().y),
            Align2::RIGHT_CENTER,
            fmt_time(readout_t),
            theme::figure(10.5),
            readout_colour,
        );
        let line = Stroke::new(1.4, TRACE);
        let float = Stroke::new(1.0, LEGEND);
        let unknown = Color32::from_rgba_unmultiplied(0xE2, 0x6D, 0x5A, 70);
        for (i, row) in rows.iter().enumerate() {
            let top = area.top() + i as f32 * ROW + 4.0;
            let bottom = top + ROW - 8.0;
            let mid = (top + bottom) / 2.0;
            let (label, indent) = match row {
                Row::Bit { trace, inner } => {
                    (r.traces[*trace].name.clone(), if *inner { 14.0 } else { 0.0 })
                }
                Row::Bus { name, open, .. } => {
                    (format!("{} {name}", if *open { "v" } else { ">" }), 0.0)
                }
            };
            p.text(
                Pos2::new(rect.left() + 8.0 + indent, mid),
                Align2::LEFT_CENTER,
                label,
                theme::figure(11.0),
                if matches!(row, Row::Bus { .. }) { READOUT } else { VALUE },
            );
            p.text(
                Pos2::new(rect.left() + LABEL_W + VALUE_W - 6.0, mid),
                Align2::RIGHT_CENTER,
                row_value(&r.traces, row, readout_t),
                theme::figure(11.0),
                readout_colour,
            );
            match row {
                Row::Bit { trace, .. } => {
                    let spans = bit_spans(&r.traces[*trace]);
                    let y = |c: char| match c {
                        '1' => Some(top),
                        '0' => Some(bottom),
                        'z' => Some(mid),
                        _ => None,
                    };
                    let first = spans.partition_point(|s| s.0 <= from).saturating_sub(1);
                    for k in first..spans.len() {
                        let (t0, c) = spans[k];
                        if t0 > to {
                            break;
                        }
                        let t1 = spans.get(k + 1).map(|s| s.0).unwrap_or(end);
                        let (x0, x1) = (x(t0), x(t1));
                        match y(c) {
                            Some(yy) => {
                                let s = if c == 'z' { float } else { line };
                                p.line_segment([Pos2::new(x0, yy), Pos2::new(x1, yy)], s);
                            }
                            None => {
                                let band =
                                    Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, bottom));
                                p.rect_filled(band, 0.0, unknown);
                            }
                        }
                        if k > 0 && t0 >= from {
                            let before = spans[k - 1].1;
                            let a = y(before).unwrap_or(top);
                            let b = y(c).unwrap_or(bottom);
                            let (a, b) =
                                if before == 'x' || c == 'x' { (top, bottom) } else { (a, b) };
                            p.line_segment([Pos2::new(x0, a), Pos2::new(x0, b)], line);
                        }
                    }
                }
                Row::Bus { bits, .. } => {
                    let spans = bus_spans(&r.traces, bits);
                    let first = spans.partition_point(|s| s.0 <= from).saturating_sub(1);
                    let bevel: f32 = 3.0;
                    for k in first..spans.len() {
                        let (t0, v) = (&spans[k].0, &spans[k].1);
                        if *t0 > to {
                            break;
                        }
                        let t1 = spans.get(k + 1).map(|s| s.0).unwrap_or(end);
                        let (x0, x1) = (x(*t0), x(t1));
                        if v.chars().all(|c| c == 'z') {
                            p.line_segment([Pos2::new(x0, mid), Pos2::new(x1, mid)], float);
                            continue;
                        }
                        let b = bevel.min((x1 - x0) / 2.0);
                        let (l, rr) = (
                            if *t0 > from { x0 + b } else { x0 },
                            if t1 < to { x1 - b } else { x1 },
                        );
                        let shape = vec![
                            Pos2::new(x0, mid),
                            Pos2::new(l, top),
                            Pos2::new(rr, top),
                            Pos2::new(x1, mid),
                            Pos2::new(rr, bottom),
                            Pos2::new(l, bottom),
                        ];
                        if v.contains('x') {
                            p.add(Shape::convex_polygon(shape.clone(), unknown, Stroke::NONE));
                        }
                        p.add(Shape::closed_line(shape, line));
                        let text = p.layout_no_wrap(v.clone(), theme::figure(10.5), VALUE);
                        if text.size().x + 6.0 < x1 - x0 {
                            let cx = (x0.max(area.left()) + x1.min(area.right())) / 2.0;
                            let at = Pos2::new(cx, mid) - text.size() / 2.0;
                            p.galley(at, text, VALUE);
                        }
                    }
                }
            }
        }
        let mut tip: Option<String> = None;
        for m in r.marks.iter().filter(|m| m.time >= from && m.time <= to) {
            let c = mark_colour(m.kind);
            let mx = x(m.time);
            let faint = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 90);
            p.line_segment(
                [Pos2::new(mx, area.top()), Pos2::new(mx, area.bottom())],
                Stroke::new(1.0, faint),
            );
            p.add(Shape::convex_polygon(
                vec![
                    Pos2::new(mx - 5.0, strip.top() + 1.0),
                    Pos2::new(mx + 5.0, strip.top() + 1.0),
                    Pos2::new(mx, strip.bottom()),
                ],
                c,
                Stroke::NONE,
            ));
            for (i, row) in rows.iter().enumerate() {
                if row_has(&r.traces, row, &m.nets) {
                    let cy = area.top() + i as f32 * ROW + ROW / 2.0;
                    p.circle_filled(Pos2::new(mx, cy), 3.5, c);
                }
            }
            if let Some(h) = hover
                && strip.contains(h)
                && (h.x - mx).abs() < 6.0
            {
                tip = Some(m.text.clone());
            }
        }
        if let Some(c) = view.cursor
            && c >= from
            && c <= to
        {
            let cx = x(c);
            p.line_segment(
                [Pos2::new(cx, strip.top()), Pos2::new(cx, area.bottom())],
                Stroke::new(1.4, READOUT),
            );
        }
        if let Some(hp) = hover
            && area.contains(hp)
        {
            let t = t_at(hp.x, from, to);
            p.line_segment(
                [Pos2::new(hp.x, area.top()), Pos2::new(hp.x, area.bottom())],
                Stroke::new(1.0, LEGEND),
            );
            let mut text = match row_at(hp.y).map(|k| &rows[k]) {
                Some(row) => {
                    let name = match row {
                        Row::Bit { trace, .. } => r.traces[*trace].name.clone(),
                        Row::Bus { name, .. } => name.clone(),
                    };
                    format!("{name} = {} at {}", row_value(&r.traces, row, t), fmt_time(t))
                }
                None => fmt_time(t),
            };
            if let Some(c) = view.cursor {
                let sign = if t >= c { "+" } else { "-" };
                text.push_str(&format!(", {sign}{} from the cursor", fmt_time(t.abs_diff(c))));
            }
            tip = tip.or(Some(text));
        }
        if let Some(t) = tip {
            resp.on_hover_text(t);
        }
    });
}

fn tick_step(span: u64, most: u64) -> u64 {
    let mut step = 1u64;
    loop {
        for m in [1, 2, 5] {
            if span / (step * m) <= most {
                return step * m;
            }
        }
        step *= 10;
    }
}

pub fn props(ui: &mut Ui, s: &Sim, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("logic").value(&s.name).elided(ui);
        },
        |ui| {
            let (passed, failed) = match &s.logic_result {
                Some(r) => (r.passed.to_string(), r.failures.len().to_string()),
                None => ("-".into(), "-".into()),
            };
            let (duration, cells) = match &s.logic {
                Some(l) => (fmt_time(l.duration), l.circuit.cells.len().to_string()),
                None => ("-".into(), "-".into()),
            };
            readouts(
                ui,
                &[
                    ("duration", duration, VALUE),
                    ("passed", passed, READOUT),
                    ("failures", failed, TRACE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            if let Some(l) = &s.logic {
                reading(ui, "schematic", l.schematic.clone());
                reading(ui, "cells", cells);
                reading(ui, "stimuli", l.stimuli.len().to_string());
                reading(ui, "assertions", l.expects.len().to_string());
            }
            if let Some(r) = &s.logic_result {
                reading(ui, "vcd", r.vcd.clone());
            }
        },
    );
    ui.add_space(8.0);
    if let Some(r) = &s.logic_result {
        Line::new().legend("readings").show(ui);
        let cols = [("what", 150.0), ("value", 90.0), ("detail", 150.0)];
        Table::new(&cols, r.readings.len()).show(ui, |i, p, row, at| {
            let x = &r.readings[i];
            cell(p, row, at(0), cols[0].1, &x.label, VALUE);
            cell(p, row, at(1), cols[1].1, &format!("{}", x.value), TRACE);
            cell(p, row, at(2), cols[2].1, &x.detail, LEGEND);
        });
        ui.add_space(6.0);
        for f in &r.failures {
            note(ui, f, FAULT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentee_core::logic::Bus;

    fn trace(name: &str, times: &[u64], values: &str) -> Trace {
        Trace { name: name.into(), times: times.to_vec(), values: values.into() }
    }

    #[test]
    fn bus_bits_read_brackets_digits_and_active_low() {
        assert_eq!(bus_bit("D[12]"), Some(("D".into(), 12, "".into())));
        assert_eq!(bus_bit("Q3"), Some(("Q".into(), 3, "".into())));
        assert_eq!(bus_bit("Y7_N"), Some(("Y".into(), 7, "_N".into())));
        assert_eq!(bus_bit("CLK"), None);
        assert_eq!(bus_bit("5V"), None);
        assert_eq!(bus_bit("U1_OUT"), None);
    }

    #[test]
    fn rows_group_buses_in_trace_order() {
        let t = |n: &str| trace(n, &[0], "0");
        let traces = vec![
            t("CLK"),
            t("Q3"),
            t("Q2"),
            t("Q1"),
            t("Q0"),
            t("A[1]"),
            t("A[0]"),
            t("TC"),
            t("SDA"),
            t("SCL"),
            t("X1"),
        ];
        let explicit = vec![Bus { name: "I2C".into(), nets: vec!["SCL".into(), "SDA".into()] }];
        let r = rows(&traces, &explicit, &[]);
        assert_eq!(
            r,
            vec![
                Row::Bit { trace: 0, inner: false },
                Row::Bus { name: "Q[3:0]".into(), bits: vec![1, 2, 3, 4], open: false },
                Row::Bus { name: "A[1:0]".into(), bits: vec![5, 6], open: false },
                Row::Bit { trace: 7, inner: false },
                Row::Bus { name: "I2C".into(), bits: vec![9, 8], open: false },
                Row::Bit { trace: 10, inner: false },
            ]
        );
        let r = rows(&traces, &explicit, &["A[1:0]".to_string()]);
        assert_eq!(r[2], Row::Bus { name: "A[1:0]".into(), bits: vec![5, 6], open: true });
        assert_eq!(r[3], Row::Bit { trace: 5, inner: true });
        assert_eq!(r[4], Row::Bit { trace: 6, inner: true });
    }

    #[test]
    fn buses_read_as_hex_with_unknown_nibbles() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(hex(&c("1010")), "A");
        assert_eq!(hex(&c("110100101")), "1A5");
        assert_eq!(hex(&c("1x001111")), "xF");
        assert_eq!(hex(&c("zzzz0001")), "z1");
        assert_eq!(hex(&c("zz01")), "x");
        let traces = vec![trace("B1", &[0, 50, 90], "011"), trace("B0", &[0, 50, 70], "10x")];
        assert_eq!(
            bus_spans(&traces, &[0, 1]),
            vec![(0, "1".into()), (50, "2".into()), (70, "x".into())]
        );
    }

    #[test]
    fn view_zooms_about_a_point_and_pans_inside_the_run() {
        let mut v = WaveView::default();
        assert_eq!(v.window("a", 1000), (0, 1000));
        v.zoom(250, 0.5, 1000);
        assert_eq!((v.from, v.to), (125, 625));
        v.pan(-500.0, 1000);
        assert_eq!((v.from, v.to), (0, 500));
        v.pan(10_000.0, 1000);
        assert_eq!((v.from, v.to), (500, 1000));
        v.zoom(900, 100.0, 1000);
        assert_eq!((v.from, v.to), (0, 1000));
        v.zoom(3, 1e-9, 1000);
        assert_eq!(v.to - v.from, MIN_SPAN);
        v.show(800, 1000);
        assert!(v.from <= 800 && v.to >= 800);
        v.cursor = Some(5);
        assert_eq!(v.window("b", 1000), (0, 1000));
        assert_eq!(v.cursor, None);
        let words: Vec<String> = ["from=100ns", "to=0.5us", "cursor=250ns", "open=Q[3:0]", "3d"]
            .map(String::from)
            .into();
        v.read("c", &words);
        assert_eq!(v.window("c", 2_000_000), (100_000, 500_000));
        assert_eq!(v.cursor, Some(250_000));
        assert_eq!(v.open, vec!["Q[3:0]".to_string()]);
    }

    #[test]
    fn mouse_wheel_drag_and_click_drive_the_view() {
        use egui::{Event, Modifiers, MouseWheelUnit, RawInput, TouchPhase};
        let ctx = egui::Context::default();
        egui_bench::install(&ctx);
        let r = LogicResult {
            name: "w".into(),
            kind: "logic".into(),
            spec_hash: 0,
            duration_ps: 1_000_000,
            end_ps: 1_000_000,
            traces: vec![trace("A", &[0, 500_000], "01")],
            buses: Vec::new(),
            marks: Vec::new(),
            passed: 0,
            failures: Vec::new(),
            readings: Vec::new(),
            events: 0,
            seconds: 0.0,
            vcd: String::new(),
        };
        let mut view = WaveView::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, view: &mut WaveView| {
            time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 400.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| canvas(ui, &r, view, true));
        };
        let at = Pos2::new(600.0, 100.0);
        let button = |pos: Pos2, pressed: bool| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(Vec::new(), &mut view);
        assert_eq!((view.from, view.to), (0, 1_000_000));
        frame(vec![Event::PointerMoved(at)], &mut view);
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 120.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        frame(vec![wheel], &mut view);
        for _ in 0..60 {
            frame(Vec::new(), &mut view);
        }
        let span = view.to - view.from;
        assert!(span < 900_000, "{view:?}");
        assert!(view.from > 0 && view.to < 1_000_000, "{view:?}");
        let before = view.from;
        frame(vec![button(at, true)], &mut view);
        frame(vec![Event::PointerMoved(Pos2::new(500.0, 100.0))], &mut view);
        frame(vec![Event::PointerMoved(Pos2::new(400.0, 100.0))], &mut view);
        frame(vec![button(Pos2::new(400.0, 100.0), false)], &mut view);
        assert!(view.from > before, "{view:?}");
        assert_eq!(view.to - view.from, span);
        assert_eq!(view.cursor, None);
        let spot = Pos2::new(700.0, 100.0);
        frame(vec![Event::PointerMoved(spot)], &mut view);
        frame(vec![button(spot, true)], &mut view);
        frame(vec![button(spot, false)], &mut view);
        let c = view.cursor.expect("a click sets the cursor");
        assert!(c > view.from && c < view.to, "{view:?}");
        let key = |key: Key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(vec![key(Key::F)], &mut view);
        assert_eq!((view.from, view.to), (0, 1_000_000));
        frame(vec![key(Key::Equals)], &mut view);
        assert_eq!(view.to - view.from, 500_000);
        frame(vec![key(Key::Escape)], &mut view);
        assert_eq!(view.cursor, None);
    }

    fn count(rgba: &[u8], c: Color32) -> usize {
        rgba.chunks(4).filter(|p| p[0] == c.r() && p[1] == c.g() && p[2] == c.b()).count()
    }

    #[test]
    fn headless_render_draws_the_cursor_zoom_buses_and_markers() {
        use crate::headless::{RenderOptions, render_rgba};
        use agentee_core::project::ItemRef;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
        let mut p = agentee_core::Project::load(&root).unwrap();
        let i = p.sims.iter().position(|s| s.name == "counter").unwrap();
        let render = |p: &agentee_core::Project, show: &[&str]| {
            let opts = RenderOptions {
                width: 1000,
                height: 500,
                panels: false,
                show: show.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            render_rgba(p, ItemRef::Sim(i), &opts).2
        };
        let plain = render(&p, &[]);
        let cursor = render(&p, &["cursor=550ns"]);
        assert!(count(&cursor, READOUT) > count(&plain, READOUT) + 100);
        let zoomed = render(&p, &["from=300ns", "to=900ns"]);
        assert_ne!(zoomed, plain);
        let open = render(&p, &["open=Q[3:0]"]);
        assert!(count(&open, TRACE) > count(&plain, TRACE));
        assert_eq!(count(&plain, WARN), 0);
        let r = p.sims[i].item.logic_result.as_mut().unwrap();
        r.marks.push(Mark {
            time: 700_000,
            kind: MarkKind::Timing,
            nets: vec!["Q1".into()],
            text: "setup".into(),
        });
        assert!(count(&render(&p, &[]), WARN) > 30);
    }
}

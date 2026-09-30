use agentee_core::logic::{LogicResult, fmt_time};
use agentee_core::sim::Sim;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;

const ROW: f32 = 24.0;
const LABEL_W: f32 = 120.0;

fn tick_step(span: u64) -> u64 {
    let mut step = 1u64;
    loop {
        for m in [1, 2, 5] {
            if span / (step * m) <= 10 {
                return step * m;
            }
        }
        step *= 10;
    }
}

pub fn canvas(ui: &mut Ui, r: &LogicResult) {
    let end = r.end_ps.max(1);
    let sub = format!("{} nets, 0 to {}", r.traces.len(), fmt_time(end));
    section(ui, "waveforms", &sub, |ui| {
        let height = (r.traces.len() as f32 * ROW + 34.0).max(120.0);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, WELL);
        let area = Rect::from_min_max(
            rect.min + Vec2::new(LABEL_W, 8.0),
            rect.max - Vec2::new(14.0, 26.0),
        );
        let x = |t: u64| area.left() + (t.min(end) as f64 / end as f64) as f32 * area.width();
        let grid = Stroke::new(1.0, ETCH);
        let step = tick_step(end);
        let mut t = 0;
        while t <= end {
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
        let line = Stroke::new(1.4, TRACE);
        let float = Stroke::new(1.0, LEGEND);
        let unknown = Color32::from_rgba_unmultiplied(0xE2, 0x6D, 0x5A, 70);
        for (i, tr) in r.traces.iter().enumerate() {
            let top = area.top() + i as f32 * ROW + 4.0;
            let bottom = top + ROW - 8.0;
            let mid = (top + bottom) / 2.0;
            p.text(
                Pos2::new(rect.left() + 8.0, mid),
                Align2::LEFT_CENTER,
                &tr.name,
                theme::figure(11.0),
                VALUE,
            );
            let y = |c: char| match c {
                '1' => Some(top),
                '0' => Some(bottom),
                'z' => Some(mid),
                _ => None,
            };
            let mut spans: Vec<(u64, char)> = vec![(0, 'z')];
            for (t, c) in tr.times.iter().zip(tr.values.chars()) {
                if *t == 0 {
                    spans[0].1 = c;
                } else {
                    spans.push((*t, c));
                }
            }
            for (k, (t0, c)) in spans.iter().enumerate() {
                let t1 = spans.get(k + 1).map(|s| s.0).unwrap_or(end);
                let (x0, x1) = (x(*t0), x(t1));
                match y(*c) {
                    Some(yy) => {
                        let s = if *c == 'z' { float } else { line };
                        p.line_segment([Pos2::new(x0, yy), Pos2::new(x1, yy)], s);
                    }
                    None => {
                        let band = Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, bottom));
                        p.rect_filled(band, 0.0, unknown);
                    }
                }
                if k > 0 {
                    let a = y(spans[k - 1].1).unwrap_or(top);
                    let b = y(*c).unwrap_or(bottom);
                    let (a, b) =
                        if spans[k - 1].1 == 'x' || *c == 'x' { (top, bottom) } else { (a, b) };
                    p.line_segment([Pos2::new(x0, a), Pos2::new(x0, b)], line);
                }
            }
        }
        if let Some(hp) = resp.hover_pos()
            && area.contains(hp)
        {
            let t = (((hp.x - area.left()) / area.width()) as f64 * end as f64).round() as u64;
            p.line_segment(
                [Pos2::new(hp.x, area.top()), Pos2::new(hp.x, area.bottom())],
                Stroke::new(1.0, READOUT),
            );
            let row = ((hp.y - area.top()) / ROW).floor() as usize;
            let text = match r.traces.get(row) {
                Some(tr) => {
                    let k = tr.times.partition_point(|x| *x <= t);
                    let v = if k == 0 { 'z' } else { tr.values.chars().nth(k - 1).unwrap_or('x') };
                    format!("{} = {v} at {}", tr.name, fmt_time(t))
                }
                None => fmt_time(t),
            };
            resp.on_hover_text(text);
        }
    });
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

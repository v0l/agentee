use agentee_core::circuit::{CircuitResult, Curve, fmt_si};
use agentee_core::sim::Sim;
use egui::epaint::PathShape;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;

fn nice(step: f64) -> f64 {
    let p = 10f64.powf(step.log10().floor());
    let m = step / p;
    let n = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    n * p
}

pub fn props(ui: &mut Ui, s: &Sim, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("circuit").value(&s.name).elided(ui);
        },
        |ui| {
            if let Some(c) = &s.circuit {
                readouts(
                    ui,
                    &[
                        ("parts", c.elements.len().to_string(), VALUE),
                        ("nets", c.nets.len().to_string(), READOUT),
                        ("drives", c.drives.len().to_string(), TRACE),
                    ],
                );
                reading(ui, "schematic", c.schematic.clone());
                let mut analyses = vec!["op"];
                analyses.extend(c.ac.map(|_| "ac"));
                analyses.extend(c.tran.map(|_| "tran"));
                analyses.extend(c.noise.as_ref().map(|_| "noise"));
                reading(ui, "analyses", analyses.join(", "));
            }
            if let Some(r) = &s.circuit_result {
                let (text, colour) = if r.passed {
                    ("passed".to_string(), OK)
                } else {
                    (format!("{} failed", r.failures.len()), FAULT)
                };
                reading(ui, "result", text);
                for f in r.failures.iter().take(6) {
                    note(ui, f, colour);
                }
                if let Some(rms) = r.noise_rms {
                    reading(ui, "noise rms", fmt_si(rms, "V"));
                }
            }
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
        },
    );
    ui.add_space(8.0);
    if let Some(r) = &s.circuit_result {
        let list: Vec<agentee_core::sim::Reading> = r
            .checks
            .iter()
            .map(|c| agentee_core::sim::Reading {
                label: c.label.clone(),
                value: c.value,
                unit: c.unit.clone(),
                detail: if c.passed { "ok".into() } else { "out of range".into() },
            })
            .collect();
        if !list.is_empty() {
            crate::heat::readings(ui, &list);
        }
    }
}

pub fn canvas(ui: &mut Ui, r: &CircuitResult) {
    let w = ui.available_width();
    let shares =
        [(!r.times.is_empty(), 1.0), (!r.freqs.is_empty(), 1.7), (!r.noise_freqs.is_empty(), 1.0)];
    let plots = shares.iter().filter(|s| s.0).count() + usize::from(!r.freqs.is_empty());
    let weight: f32 = shares.iter().filter(|s| s.0).map(|s| s.1).sum::<f32>().max(1.0);
    let h = ((ui.available_height() - SECTION * plots as f32) / weight).clamp(140.0, 420.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if !r.times.is_empty() {
            section(ui, "transient", "node voltages against time", |ui| {
                plot(ui, &r.times, &r.waves, false, "s", "V", 1.0, Vec2::new(w, h));
            });
            ui.add_space(8.0);
        }
        if !r.freqs.is_empty() {
            section(ui, "gain", "relative to the ac drive", |ui| {
                plot(ui, &r.freqs, &r.gain_db, true, "Hz", "dB", 1.0, Vec2::new(w, h));
            });
            ui.add_space(8.0);
            let phase = live_phase(&r.gain_db, &r.phase_deg);
            section(ui, "phase", "degrees, where the gain is above -200 dB", |ui| {
                plot(ui, &r.freqs, &phase, true, "Hz", "deg", 1.0, Vec2::new(w, h * 0.7));
            });
            ui.add_space(8.0);
        }
        if !r.noise_freqs.is_empty() {
            section(ui, "noise", "spectral density", |ui| {
                plot(ui, &r.noise_freqs, &r.noise, true, "Hz", "nV/rtHz", 1e9, Vec2::new(w, h));
            });
            ui.add_space(8.0);
        }
        section(
            ui,
            "operating point",
            "every net, and the current through each two-pin part",
            |ui| {
                let rows: Vec<(String, String)> =
                    r.op.iter()
                        .map(|(n, v)| (n.clone(), fmt_si(*v, "V")))
                        .chain(r.currents.iter().map(|(n, i)| (format!("i({n})"), fmt_si(*i, "A"))))
                        .collect();
                let cols = [("what", 180.0), ("value", 120.0)];
                Table::new(&cols, rows.len()).show(ui, |i, p, row, at| {
                    cell(p, row, at(0), cols[0].1, &rows[i].0, VALUE);
                    cell(p, row, at(1), cols[1].1, &rows[i].1, TRACE);
                });
            },
        );
    });
}

const DEAD_DB: f64 = -200.0;
const SECTION: f32 = 52.0;

fn live_phase(gain: &[Curve], phase: &[Curve]) -> Vec<Curve> {
    phase
        .iter()
        .map(|c| {
            let g = gain.iter().find(|g| g.name == c.name);
            let values = c
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| match g.and_then(|g| g.values.get(i)) {
                    Some(&db) if db < DEAD_DB => f64::NAN,
                    _ => v,
                })
                .collect();
            Curve { name: c.name.clone(), values }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn plot(
    ui: &mut Ui,
    xs: &[f64],
    curves: &[Curve],
    logx: bool,
    xunit: &str,
    yunit: &str,
    scale: f64,
    size: Vec2,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let keyw = curves
        .iter()
        .map(|c| p.layout_no_wrap(c.name.clone(), theme::legend_font(10.5), LEGEND).size().x)
        .fold(0.0, f32::max)
        + 24.0;
    let area = Rect::from_min_max(
        rect.min + Vec2::new(58.0, 22.0),
        rect.max - Vec2::new(14.0 + keyw.min(rect.width() * 0.3), 26.0),
    );
    let ys: Vec<f64> = curves
        .iter()
        .flat_map(|c| c.values.iter().map(|v| v * scale))
        .filter(|v| v.is_finite())
        .collect();
    if xs.len() < 2 || ys.is_empty() {
        return;
    }
    let tx = |x: f64| if logx { x.max(1e-30).log10() } else { x };
    let (x0, x1) = (tx(xs[0]), tx(xs[xs.len() - 1]));
    let (mut lo, mut hi) = ys.iter().fold((f64::MAX, f64::MIN), |(a, b), &y| (a.min(y), b.max(y)));
    if hi - lo < 1e-9 {
        lo -= 1.0;
        hi += 1.0;
    }
    let step = nice((hi - lo) / 5.0);
    let (lo, hi) = ((lo / step).floor() * step, (hi / step).ceil() * step);
    let x = |v: f64| area.left() + ((tx(v) - x0) / (x1 - x0).max(1e-30)) as f32 * area.width();
    let y = |v: f64| area.bottom() - ((v - lo) / (hi - lo)) as f32 * area.height();
    let grid = Stroke::new(1.0, ETCH);
    let mut v = lo;
    while v <= hi + step * 0.01 {
        p.line_segment([Pos2::new(area.left(), y(v)), Pos2::new(area.right(), y(v))], grid);
        p.text(
            Pos2::new(area.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            fmt_si(v, ""),
            theme::figure(10.5),
            LEGEND,
        );
        v += step;
    }
    let ticks: Vec<f64> = if logx {
        (x0.floor() as i32..=x1.ceil() as i32)
            .map(|k| 10f64.powi(k))
            .filter(|f| tx(*f) >= x0 - 1e-9 && tx(*f) <= x1 + 1e-9)
            .collect()
    } else {
        let s = nice((x1 - x0) / 6.0);
        let mut out = Vec::new();
        let mut t = (x0 / s).ceil() * s;
        while t <= x1 + s * 1e-6 {
            out.push(t);
            t += s;
        }
        out
    };
    for t in ticks {
        p.line_segment([Pos2::new(x(t), area.top()), Pos2::new(x(t), area.bottom())], grid);
        p.text(
            Pos2::new(x(t), area.bottom() + 4.0),
            Align2::CENTER_TOP,
            fmt_si(t, xunit),
            theme::figure(10.5),
            LEGEND,
        );
    }
    p.text(
        rect.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        yunit,
        theme::legend_font(10.5),
        LEGEND,
    );
    for (k, c) in curves.iter().enumerate() {
        let colour = crate::plot::color(k);
        let pts: Vec<Pos2> = xs
            .iter()
            .zip(&c.values)
            .filter(|(_, v)| v.is_finite())
            .map(|(a, b)| Pos2::new(x(*a), y(b * scale)))
            .collect();
        if pts.len() > 1 {
            p.add(PathShape::line(pts, Stroke::new(1.6, colour)));
        }
        let at = Pos2::new(area.right() + 12.0, area.top() + 4.0 + 15.0 * k as f32);
        p.line_segment([at, at + Vec2::new(10.0, 0.0)], Stroke::new(2.0, colour));
        p.text(
            at + Vec2::new(14.0, 0.0),
            Align2::LEFT_CENTER,
            &c.name,
            theme::legend_font(10.5),
            colour,
        );
    }
    if let Some(hp) = resp.hover_pos()
        && area.contains(hp)
    {
        let u = x0 + ((hp.x - area.left()) / area.width()) as f64 * (x1 - x0);
        let xv = if logx { 10f64.powf(u) } else { u };
        let i = xs.iter().position(|v| *v >= xv).unwrap_or(xs.len() - 1);
        let mut lines = vec![fmt_si(xs[i], xunit)];
        for c in curves {
            if let Some(v) = c.values.get(i) {
                lines.push(format!("{} {} {yunit}", c.name, fmt_si(v * scale, "")));
            }
        }
        resp.on_hover_text(lines.join("\n"));
    }
}

#[cfg(test)]
mod phase_tests {
    use super::*;

    #[test]
    fn phase_is_dropped_where_the_net_carries_no_signal() {
        let c = |n: &str, v: &[f64]| Curve { name: n.into(), values: v.to_vec() };
        let gain = [c("OUT", &[-3.0, -6.0]), c("DEAD", &[-400.0, -390.0])];
        let phase = [c("OUT", &[-45.0, -90.0]), c("DEAD", &[137.0, -12.0])];
        let shown = live_phase(&gain, &phase);
        assert_eq!(shown[0].values, [-45.0, -90.0]);
        assert!(shown[1].values.iter().all(|v| v.is_nan()));
    }
}

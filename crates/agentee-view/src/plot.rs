use agentee_core::sim::SimResult;
use egui::epaint::PathShape;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::theme::{self, ETCH, FAULT, LEGEND, OK, READOUT, TRACE, VALUE, WELL};

pub const CURVES: [Color32; 8] = [
    TRACE,
    OK,
    READOUT,
    FAULT,
    Color32::from_rgb(0xB0, 0x8C, 0xE8),
    Color32::from_rgb(0xE8, 0x8C, 0xC8),
    Color32::from_rgb(0x8C, 0xE8, 0xC8),
    Color32::from_rgb(0xE8, 0xE0, 0x8C),
];

pub fn curves(r: &SimResult) -> Vec<(usize, usize)> {
    let n = r.ports.len();
    (0..n).filter(|j| r.excited[*j]).flat_map(|j| (0..n).map(move |i| (i, j))).collect()
}

pub fn color(k: usize) -> Color32 {
    CURVES[k % CURVES.len()]
}

pub fn label(i: usize, j: usize) -> String {
    format!("S{}{}", i + 1, j + 1)
}

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

fn freq_text(f: f64) -> String {
    if f >= 1e9 {
        format!("{} GHz", agentee_core::units::trim(f / 1e9, 2))
    } else {
        format!("{} MHz", agentee_core::units::trim(f / 1e6, 1))
    }
}

pub fn db_plot(
    ui: &mut Ui,
    r: &SimResult,
    hidden: &[(usize, usize)],
    height: f32,
    interactive: bool,
) {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let area =
        Rect::from_min_max(rect.min + Vec2::new(52.0, 14.0), rect.max - Vec2::new(14.0, 30.0));
    let shown: Vec<(usize, (usize, usize))> =
        curves(r).into_iter().enumerate().filter(|(_, c)| !hidden.contains(c)).collect();
    let (f0, f1) = (r.freqs[0], *r.freqs.last().unwrap());
    let (mut lo, mut top) = (f64::MAX, f64::MIN);
    for (_, (i, j)) in &shown {
        for v in r.db(*i, *j) {
            if v.is_finite() {
                lo = lo.min(v);
                top = top.max(v);
            }
        }
    }
    let lo = (lo.max(-80.0) / 10.0).floor() * 10.0 - 5.0;
    let hi = if top > 0.0 { (top / 10.0).ceil() * 10.0 + 5.0 } else { 5.0 };
    let x = |f: f64| area.left() + ((f - f0) / (f1 - f0)) as f32 * area.width();
    let y = |d: f64| area.bottom() - ((d.clamp(lo, hi) - lo) / (hi - lo)) as f32 * area.height();
    let grid = Stroke::new(1.0, ETCH);
    let ystep = nice((hi - lo) / 6.0);
    let mut d = (lo / ystep).ceil() * ystep;
    while d <= hi {
        p.line_segment([Pos2::new(area.left(), y(d)), Pos2::new(area.right(), y(d))], grid);
        p.text(
            Pos2::new(area.left() - 6.0, y(d)),
            Align2::RIGHT_CENTER,
            format!("{d:.0}"),
            theme::figure(10.5),
            LEGEND,
        );
        d += ystep;
    }
    let xstep = nice((f1 - f0) / 8.0);
    let mut f = (f0 / xstep).ceil() * xstep;
    while f <= f1 + 1.0 {
        p.line_segment([Pos2::new(x(f), area.top()), Pos2::new(x(f), area.bottom())], grid);
        p.text(
            Pos2::new(x(f), area.bottom() + 6.0),
            Align2::CENTER_TOP,
            freq_text(f),
            theme::figure(10.5),
            LEGEND,
        );
        f += xstep;
    }
    p.text(
        area.left_top() - Vec2::new(46.0, 8.0),
        Align2::LEFT_TOP,
        "dB",
        theme::legend_font(10.5),
        LEGEND,
    );
    for (k, (i, j)) in &shown {
        let db = r.db(*i, *j);
        let pts: Vec<Pos2> = r
            .freqs
            .iter()
            .zip(&db)
            .filter(|(_, v)| v.is_finite())
            .map(|(f, v)| Pos2::new(x(*f), y(*v)))
            .collect();
        p.add(PathShape::line(pts, Stroke::new(if i == j { 1.6 } else { 2.0 }, color(*k))));
    }
    let mut ly = area.top() + 6.0;
    for (k, (i, j)) in &shown {
        let at = Pos2::new(area.right() - 60.0, ly);
        p.line_segment([at, at + Vec2::new(14.0, 0.0)], Stroke::new(2.0, color(*k)));
        let name = format!("{} {}>{}", label(*i, *j), r.ports[*j], r.ports[*i]);
        p.text(
            at + Vec2::new(-6.0, 0.0),
            Align2::RIGHT_CENTER,
            name,
            theme::legend_font(10.5),
            color(*k),
        );
        ly += 15.0;
    }
    if interactive
        && let Some(h) = resp.hover_pos()
        && area.contains(h)
    {
        let fh = f0 + ((h.x - area.left()) / area.width()) as f64 * (f1 - f0);
        let idx = r.freqs.iter().position(|v| *v >= fh).unwrap_or(r.freqs.len() - 1);
        p.line_segment(
            [Pos2::new(x(r.freqs[idx]), area.top()), Pos2::new(x(r.freqs[idx]), area.bottom())],
            Stroke::new(1.0, VALUE),
        );
        let mut lines = vec![freq_text(r.freqs[idx])];
        for (_, (i, j)) in &shown {
            lines.push(format!("{} {:.2} dB", label(*i, *j), r.db(*i, *j)[idx]));
        }
        resp.on_hover_text(lines.join("\n"));
    }
}

pub fn curve_plot(
    ui: &mut Ui,
    r: &SimResult,
    c: &agentee_core::sim::Curve,
    colour: Color32,
    size: Vec2,
    interactive: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let area =
        Rect::from_min_max(rect.min + Vec2::new(52.0, 26.0), rect.max - Vec2::new(14.0, 26.0));
    let known: Vec<(f64, f64)> = r
        .freqs
        .iter()
        .zip(&c.values)
        .filter_map(|(f, v)| v.filter(|v| v.is_finite()).map(|v| (*f, v)))
        .collect();
    p.text(
        rect.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        if c.unit.is_empty() { c.name.clone() } else { format!("{}, {}", c.name, c.unit) },
        theme::legend_font(10.5),
        colour,
    );
    if known.len() < 2 {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "no data in this band",
            theme::legend_font(10.5),
            LEGEND,
        );
        return;
    }
    let (f0, f1) = (r.freqs[0], *r.freqs.last().unwrap());
    let lo = known.iter().map(|v| v.1).fold(f64::MAX, f64::min);
    let hi = known.iter().map(|v| v.1).fold(f64::MIN, f64::max);
    let step = nice(((hi - lo).max(1e-3)) / 4.0);
    let (lo, hi) =
        ((lo / step).floor() * step, (hi / step).ceil() * step + if hi == lo { step } else { 0.0 });
    let x = |f: f64| area.left() + ((f - f0) / (f1 - f0)) as f32 * area.width();
    let y = |v: f64| area.bottom() - ((v - lo) / (hi - lo)) as f32 * area.height();
    let grid = Stroke::new(1.0, ETCH);
    let mut v = lo;
    while v <= hi + step * 0.01 {
        p.line_segment([Pos2::new(area.left(), y(v)), Pos2::new(area.right(), y(v))], grid);
        p.text(
            Pos2::new(area.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            trim_num(v, step),
            theme::figure(10.5),
            LEGEND,
        );
        v += step;
    }
    let xstep = nice((f1 - f0) / 5.0);
    let mut f = (f0 / xstep).ceil() * xstep;
    while f <= f1 + 1.0 {
        p.line_segment([Pos2::new(x(f), area.top()), Pos2::new(x(f), area.bottom())], grid);
        p.text(
            Pos2::new(x(f), area.bottom() + 5.0),
            Align2::CENTER_TOP,
            freq_text(f),
            theme::figure(10.5),
            LEGEND,
        );
        f += xstep;
    }
    let mut run: Vec<Pos2> = Vec::new();
    for (f, v) in r.freqs.iter().zip(&c.values) {
        match v {
            Some(v) if v.is_finite() => run.push(Pos2::new(x(*f), y(*v))),
            _ => {
                if run.len() > 1 {
                    p.add(PathShape::line(std::mem::take(&mut run), Stroke::new(1.8, colour)));
                }
                run.clear();
            }
        }
    }
    if run.len() > 1 {
        p.add(PathShape::line(run, Stroke::new(1.8, colour)));
    }
    if interactive
        && let Some(h) = resp.hover_pos()
        && area.contains(h)
    {
        let fh = f0 + ((h.x - area.left()) / area.width()) as f64 * (f1 - f0);
        let idx = r.freqs.iter().position(|v| *v >= fh).unwrap_or(r.freqs.len() - 1);
        if let Some(Some(v)) = c.values.get(idx) {
            resp.on_hover_text(format!(
                "{}\n{} {v:.3} {}",
                freq_text(r.freqs[idx]),
                c.name,
                c.unit
            ));
        }
    }
}

fn trim_num(v: f64, step: f64) -> String {
    if step >= 1.0 {
        format!("{v:.0}")
    } else if step >= 0.1 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

pub fn smith(ui: &mut Ui, r: &SimResult, hidden: &[(usize, usize)], size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    let c = rect.center();
    let rad = size / 2.0 - 14.0;
    let to = |g: [f64; 2]| c + Vec2::new(g[0] as f32 * rad, -(g[1] as f32) * rad);
    let grid = Stroke::new(1.0, ETCH);
    p.circle_stroke(c, rad, Stroke::new(1.2, LEGEND.gamma_multiply(0.6)));
    p.line_segment([c - Vec2::X * rad, c + Vec2::X * rad], grid);
    for rr in [0.2, 0.5, 1.0, 2.0, 5.0] {
        let cc = rr / (1.0 + rr);
        let r2 = 1.0 / (1.0 + rr);
        p.circle_stroke(to([cc, 0.0]), r2 as f32 * rad, grid);
    }
    for xx in [0.2f64, 0.5, 1.0, 2.0, 5.0] {
        for s in [1.0, -1.0] {
            let x = xx * s;
            let pts: Vec<Pos2> = (0..=200)
                .map(|k| {
                    let rr = 100f64.powf(k as f64 / 200.0 * 2.0 - 1.0) - 0.01;
                    let (zr, zi) = (rr.max(0.0), x);
                    let (nr, ni) = (zr - 1.0, zi);
                    let (dr, di) = (zr + 1.0, zi);
                    let d = dr * dr + di * di;
                    [(nr * dr + ni * di) / d, (ni * dr - nr * di) / d]
                })
                .filter(|g| g[0] * g[0] + g[1] * g[1] <= 1.0001)
                .map(to)
                .collect();
            p.add(PathShape::line(pts, grid));
        }
    }
    for (k, (i, j)) in curves(r).into_iter().enumerate() {
        if i != j || hidden.contains(&(i, j)) {
            continue;
        }
        let pts: Vec<Pos2> =
            r.s[i][j].iter().filter(|g| g[0].is_finite()).map(|g| to(*g)).collect();
        if let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) {
            p.add(PathShape::line(pts, Stroke::new(1.8, color(k))));
            p.circle_filled(first, 3.0, color(k));
            p.circle_stroke(last, 4.0, Stroke::new(1.5, color(k)));
        }
    }
    p.text(
        rect.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        "SMITH",
        theme::legend_font(10.5),
        LEGEND,
    );
}

pub type Series = (String, Vec<f64>, Vec<f64>);

pub fn tdr_series(r: &SimResult, s: &agentee_core::sim::Sim) -> Vec<Series> {
    use agentee_core::rf::Cx;
    let fmax = *r.freqs.last().unwrap_or(&1e9);
    let rise = (1.3 / fmax).max(10e-12);
    (0..r.ports.len())
        .filter(|k| r.excited.get(*k).copied().unwrap_or(false))
        .map(|k| {
            let s11: Vec<Cx> = r.s[k][k].iter().map(|c| Cx::new(c[0], c[1])).collect();
            let st = agentee_core::sparam::step(&r.freqs, &s11, rise, Some(40.0 * rise));
            let z0 = s.ports.get(k).map(|p| p.impedance).unwrap_or(50.0);
            let z = agentee_core::sparam::tdr_impedance(&st, z0);
            (r.ports[k].clone(), st.time_ps, z)
        })
        .collect()
}

pub fn xy_plot(
    ui: &mut Ui,
    series: &[(String, Vec<f64>, Vec<f64>)],
    xunit: &str,
    yunit: &str,
    size: Vec2,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let area =
        Rect::from_min_max(rect.min + Vec2::new(52.0, 14.0), rect.max - Vec2::new(14.0, 30.0));
    let finite = |v: &f64| v.is_finite();
    let (x0, x1) = series
        .iter()
        .flat_map(|s| s.1.iter().copied())
        .fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)));
    let ys: Vec<f64> = series.iter().flat_map(|s| s.2.iter().copied().filter(finite)).collect();
    if ys.is_empty() || x1 <= x0 {
        return;
    }
    let (mut lo, mut hi) = ys.iter().fold((f64::MAX, f64::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
    lo = lo.max(0.0);
    hi = hi.min(lo + 200.0);
    let step = nice(((hi - lo).max(1.0)) / 6.0);
    let (lo, hi) = ((lo / step).floor() * step, (hi / step).ceil() * step);
    let x = |v: f64| area.left() + ((v - x0) / (x1 - x0)) as f32 * area.width();
    let y = |v: f64| area.bottom() - ((v.clamp(lo, hi) - lo) / (hi - lo)) as f32 * area.height();
    let grid = Stroke::new(1.0, ETCH);
    let mut v = lo;
    while v <= hi + step * 0.01 {
        p.line_segment([Pos2::new(area.left(), y(v)), Pos2::new(area.right(), y(v))], grid);
        p.text(
            Pos2::new(area.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            format!("{v:.0}"),
            theme::figure(10.5),
            LEGEND,
        );
        v += step;
    }
    let xs = nice((x1 - x0) / 8.0);
    let mut t = (x0 / xs).ceil() * xs;
    while t <= x1 {
        p.line_segment([Pos2::new(x(t), area.top()), Pos2::new(x(t), area.bottom())], grid);
        p.text(
            Pos2::new(x(t), area.bottom() + 6.0),
            Align2::CENTER_TOP,
            format!("{t:.0} {xunit}"),
            theme::figure(10.5),
            LEGEND,
        );
        t += xs;
    }
    p.text(
        area.left_top() - Vec2::new(46.0, 8.0),
        Align2::LEFT_TOP,
        yunit,
        theme::legend_font(10.5),
        LEGEND,
    );
    for (k, (name, xs, ys)) in series.iter().enumerate() {
        let pts: Vec<Pos2> = xs
            .iter()
            .zip(ys)
            .filter(|(_, v)| v.is_finite())
            .map(|(a, b)| Pos2::new(x(*a), y(*b)))
            .collect();
        p.add(PathShape::line(pts, Stroke::new(1.8, color(k))));
        let at = Pos2::new(area.right() - 60.0, area.top() + 6.0 + 15.0 * k as f32);
        p.line_segment([at, at + Vec2::new(14.0, 0.0)], Stroke::new(2.0, color(k)));
        p.text(
            at + Vec2::new(-6.0, 0.0),
            Align2::RIGHT_CENTER,
            name,
            theme::legend_font(10.5),
            color(k),
        );
    }
    if let Some(h) = resp.hover_pos()
        && area.contains(h)
    {
        let tx = x0 + ((h.x - area.left()) / area.width()) as f64 * (x1 - x0);
        let mut lines = vec![format!("{tx:.0} {xunit}")];
        for (name, xs, ys) in series {
            if let Some(i) = xs.iter().position(|v| *v >= tx) {
                lines.push(format!("{name} {:.1} {yunit}", ys[i]));
            }
        }
        resp.on_hover_text(lines.join("\n"));
    }
}

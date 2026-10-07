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

const ROW: f32 = 15.0;

struct Legend {
    rows: usize,
    cols: usize,
    colw: f32,
}

impl Legend {
    fn new(p: &egui::Painter, entries: &[(String, Color32)], height: f32, max: f32) -> Legend {
        let rows = ((height - 6.0) / ROW).floor().max(1.0) as usize;
        let widest = entries
            .iter()
            .map(|(name, _)| {
                p.layout_no_wrap(name.clone(), theme::legend_font(10.5), LEGEND).size().x
            })
            .fold(0.0, f32::max);
        let colw = widest + 30.0;
        let fit = (((max - 6.0) / colw).floor() as usize).max(1);
        Legend { rows, cols: entries.len().div_ceil(rows).min(fit), colw }
    }

    fn width(&self, entries: &[(String, Color32)]) -> f32 {
        if entries.is_empty() { 0.0 } else { self.colw * self.cols as f32 + 6.0 }
    }
}

fn legend(p: &egui::Painter, strip: Rect, entries: &[(String, Color32)], l: &Legend) {
    if entries.is_empty() {
        return;
    }
    let (rows, colw) = (l.rows, l.colw);
    let room = rows * l.cols;
    let more = entries.len().saturating_sub(room);
    let shown = if more > 0 { room - 1 } else { entries.len() };
    let p = p.with_clip_rect(strip);
    if more > 0 {
        let k = shown;
        let at = strip.left_top()
            + Vec2::new(6.0 + (k / rows) as f32 * colw, 6.0 + (k % rows) as f32 * ROW + ROW / 2.0);
        p.text(
            at + Vec2::new(20.0, 0.0),
            Align2::LEFT_CENTER,
            format!("+{} more", entries.len() - shown),
            theme::legend_font(10.5),
            LEGEND,
        );
    }
    for (k, (name, colour)) in entries.iter().take(shown).enumerate() {
        let at = strip.left_top()
            + Vec2::new(6.0 + (k / rows) as f32 * colw, 6.0 + (k % rows) as f32 * ROW + ROW / 2.0);
        p.line_segment([at, at + Vec2::new(14.0, 0.0)], Stroke::new(2.0, *colour));
        p.text(
            at + Vec2::new(20.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            theme::legend_font(10.5),
            *colour,
        );
    }
}

fn plot_area(
    p: &egui::Painter,
    rect: Rect,
    left: f32,
    entries: &[(String, Color32)],
) -> (Rect, Rect, Legend) {
    let inner =
        Rect::from_min_max(rect.min + Vec2::new(left, 14.0), rect.max - Vec2::new(14.0, 30.0));
    let l = Legend::new(p, entries, inner.height(), rect.width() * 0.45);
    let w = l.width(entries);
    let area = Rect::from_min_max(inner.min, inner.max - Vec2::new(w, 0.0));
    let strip = Rect::from_min_max(
        egui::pos2(area.right() + 16.0, inner.top()),
        egui::pos2(rect.right() - 4.0, inner.bottom()),
    );
    (area, strip, l)
}

fn x_ticks(width: f32) -> f64 {
    (width / 90.0).floor().clamp(2.0, 8.0) as f64
}

fn x_label(p: &egui::Painter, rect: Rect, at: Pos2, text: String) {
    let g = p.layout_no_wrap(text, theme::figure(10.5), LEGEND);
    let w = g.size().x;
    let x = (at.x - w / 2.0).min(rect.right() - w - 2.0).max(rect.left() + 2.0);
    p.galley(Pos2::new(x, at.y), g, LEGEND);
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
    let shown: Vec<(usize, (usize, usize))> =
        curves(r).into_iter().enumerate().filter(|(_, c)| !hidden.contains(c)).collect();
    let entries: Vec<(String, Color32)> = shown
        .iter()
        .map(|(k, (i, j))| {
            (format!("{} {}>{}", label(*i, *j), r.ports[*j], r.ports[*i]), color(*k))
        })
        .collect();
    let (area, strip, keys) = plot_area(&p, rect, 52.0, &entries);
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
    let xstep = nice((f1 - f0) / x_ticks(area.width()));
    let mut f = (f0 / xstep).ceil() * xstep;
    while f <= f1 + 1.0 {
        p.line_segment([Pos2::new(x(f), area.top()), Pos2::new(x(f), area.bottom())], grid);
        x_label(&p, rect, Pos2::new(x(f), area.bottom() + 6.0), freq_text(f));
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
    legend(&p, strip, &entries, &keys);
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
        Rect::from_min_max(rect.min + Vec2::new(52.0, 24.0), rect.max - Vec2::new(14.0, 24.0));
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
    let ticks = (area.height() / 20.0).floor().clamp(1.0, 4.0) as f64;
    let step = nice(((hi - lo).max(1e-3)) / ticks);
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
    let xstep = nice((f1 - f0) / x_ticks(area.width()).min(5.0));
    let mut f = (f0 / xstep).ceil() * xstep;
    while f <= f1 + 1.0 {
        p.line_segment([Pos2::new(x(f), area.top()), Pos2::new(x(f), area.bottom())], grid);
        x_label(&p, rect, Pos2::new(x(f), area.bottom() + 5.0), freq_text(f));
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
    series: &[Series],
    xunit: &str,
    yunit: &str,
    size: Vec2,
    span: Option<f64>,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let entries: Vec<(String, Color32)> =
        series.iter().enumerate().map(|(k, s)| (s.0.clone(), color(k))).collect();
    let (area, strip, keys) = plot_area(&p, rect, 52.0, &entries);
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
    if let Some(sp) = span {
        lo = lo.max(0.0);
        hi = hi.min(lo + sp);
    }
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
    let xs = nice((x1 - x0) / x_ticks(area.width()));
    let mut t = (x0 / xs).ceil() * xs;
    while t <= x1 {
        p.line_segment([Pos2::new(x(t), area.top()), Pos2::new(x(t), area.bottom())], grid);
        x_label(&p, rect, Pos2::new(x(t), area.bottom() + 6.0), format!("{t:.0} {xunit}"));
        t += xs;
    }
    p.text(
        area.left_top() - Vec2::new(46.0, 8.0),
        Align2::LEFT_TOP,
        yunit,
        theme::legend_font(10.5),
        LEGEND,
    );
    for (k, (_, xs, ys)) in series.iter().enumerate() {
        let pts: Vec<Pos2> = xs
            .iter()
            .zip(ys)
            .filter(|(_, v)| v.is_finite())
            .map(|(a, b)| Pos2::new(x(*a), y(*b)))
            .collect();
        p.add(PathShape::line(pts, Stroke::new(1.8, color(k))));
    }
    legend(&p, strip, &entries, &keys);
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

pub fn loglog(ui: &mut Ui, r: &SimResult, size: Vec2) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, WELL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
    let colour_of = |k: usize, c: &agentee_core::sim::Curve| {
        if c.name == "target" { FAULT } else { color(k) }
    };
    let entries: Vec<(String, Color32)> =
        r.curves.iter().enumerate().map(|(k, c)| (c.name.clone(), colour_of(k, c))).collect();
    let (area, strip, keys) = plot_area(&p, rect, 70.0, &entries);
    let vals: Vec<f64> = r
        .curves
        .iter()
        .flat_map(|c| c.values.iter().flatten().copied())
        .filter(|v| *v > 0.0)
        .collect();
    if vals.is_empty() || r.freqs.len() < 2 {
        return;
    }
    let (f0, f1) = (r.freqs[0].log10(), r.freqs.last().unwrap().log10());
    let lo = vals.iter().copied().fold(f64::MAX, f64::min).log10().floor();
    let hi = vals.iter().copied().fold(f64::MIN, f64::max).log10().ceil().max(lo + 1.0);
    let x = |f: f64| area.left() + ((f.log10() - f0) / (f1 - f0)) as f32 * area.width();
    let y =
        |v: f64| area.bottom() - ((v.max(1e-12).log10() - lo) / (hi - lo)) as f32 * area.height();
    let grid = Stroke::new(1.0, ETCH);
    let ohm =
        |v: f64| if v >= 1.0 { format!("{v:.0} ohm") } else { format!("{:.0} mohm", v * 1e3) };
    let mut e = lo;
    while e <= hi + 1e-9 {
        let v = 10f64.powf(e);
        p.line_segment([Pos2::new(area.left(), y(v)), Pos2::new(area.right(), y(v))], grid);
        p.text(
            Pos2::new(area.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            ohm(v),
            theme::figure(10.5),
            LEGEND,
        );
        e += 1.0;
    }
    let mut d = f0.ceil();
    while d <= f1 + 1e-9 {
        let f = 10f64.powf(d);
        p.line_segment([Pos2::new(x(f), area.top()), Pos2::new(x(f), area.bottom())], grid);
        x_label(&p, rect, Pos2::new(x(f), area.bottom() + 6.0), freq_text(f));
        d += 1.0;
    }
    for (k, c) in r.curves.iter().enumerate() {
        let target = c.name == "target";
        let colour = colour_of(k, c);
        let pts: Vec<Pos2> = r
            .freqs
            .iter()
            .zip(&c.values)
            .filter_map(|(f, v)| v.filter(|v| *v > 0.0).map(|v| Pos2::new(x(*f), y(v))))
            .collect();
        p.add(PathShape::line(pts, Stroke::new(if target { 1.2 } else { 1.8 }, colour)));
    }
    legend(&p, strip, &entries, &keys);
    if let Some(h) = resp.hover_pos()
        && area.contains(h)
    {
        let f = 10f64.powf(f0 + ((h.x - area.left()) / area.width()) as f64 * (f1 - f0));
        let i = r.freqs.iter().position(|v| *v >= f).unwrap_or(r.freqs.len() - 1);
        let mut lines = vec![freq_text(r.freqs[i])];
        for c in &r.curves {
            if let Some(Some(v)) = c.values.get(i) {
                lines.push(format!("{} {}", c.name, ohm(*v)));
            }
        }
        resp.on_hover_text(lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_columns_stay_inside_their_share_and_count_the_rest() {
        let ctx = egui::Context::default();
        egui_bench::install(&ctx);
        let _ = ctx.run_ui(Default::default(), |ui| {
            let p = ui.painter();
            let entries: Vec<(String, Color32)> =
                (0..36).map(|k| (format!("S{k} AMP_OUT>AMP_IN"), color(k))).collect();
            let l = Legend::new(p, &entries, 200.0, 300.0);
            assert!(l.width(&entries) <= 300.0);
            assert!(l.rows * l.cols < entries.len());
            let few = &entries[..4];
            let l = Legend::new(p, few, 200.0, 300.0);
            assert_eq!((l.cols, l.rows >= 4), (1, true));
        });
    }

    #[test]
    fn x_ticks_thin_out_on_narrow_plots() {
        assert_eq!(x_ticks(1200.0), 8.0);
        assert_eq!(x_ticks(400.0), 4.0);
        assert_eq!(x_ticks(100.0), 2.0);
    }
}

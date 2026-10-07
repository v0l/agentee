use super::{Category, Ctx, Report, Rule, Setup, every};
use crate::calc::{COPLANAR_REACH, Line, TraceGeometry};
use crate::diag::Severity;
use crate::geom;
use crate::geom::P;
use crate::layout::{NECKDOWN, class_of, unit};
use crate::units::Length;

pub static RULES: &[Rule] = &[
    Rule {
        id: "neckdown",
        category: Category::Copper,
        severity: Severity::Info,
        summary: "a short track narrower than its class width, within the class `neckdown` length (0.5 mm by default) and not under min_track_width",
        when: "every board",
        applies: every,
        check: neckdown,
    },
    Rule {
        id: "class-width",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a track narrower than its net class width that is not a neck-down",
        when: "every board",
        applies: every,
        check: class_width,
    },
    Rule {
        id: "impedance-width",
        category: Category::Signal,
        severity: Severity::Warning,
        summary: "a track of an impedance class at another width than the class, so its impedance moves",
        when: "impedance classes",
        applies: with_impedance,
        check: impedance_width,
    },
    Rule {
        id: "impedance-trace",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a track of an impedance class whose width, neck-downs included, or the copper beside it on an outer layer moves its impedance outside the class tolerance",
        when: "impedance classes",
        applies: with_impedance,
        check: impedance_trace,
    },
    Rule {
        id: "track-overlap",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "tracks of one net that run on top of each other, doubling the copper",
        when: "every board",
        applies: every,
        check: track_overlap,
    },
    Rule {
        id: "acute-turn",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a track that turns back more than 90 degrees, an acid trap",
        when: "every board",
        applies: every,
        check: acute_turn,
    },
];

const GAP_SAMPLE: f64 = 0.2;
const GAP_STEP: f64 = 0.01;

fn with_impedance(s: &Setup) -> bool {
    s.impedance
}

struct Width {
    at: String,
    width: f64,
    class_w: f64,
    length: f64,
    limit: f64,
    neckdown: bool,
}

fn widths(cx: &Ctx) -> Vec<(usize, Width)> {
    let board = cx.board;
    cx.tracks
        .iter()
        .enumerate()
        .map(|(ti, t)| {
            let n = &cx.nets[t.net];
            let class = class_of(board, &n.class);
            let class_w = class.map(|c| c.width_on(&t.layer).to_mm()).unwrap_or(n.width);
            let length: f64 = t.points.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
            let limit = class.and_then(|c| c.neckdown.map(Length::to_mm)).unwrap_or(NECKDOWN);
            let width = t.width;
            let neckdown = width + 1e-6 < class_w
                && length <= limit
                && width + 1e-6 >= board.rules.min_track_width.to_mm();
            let at = format!("tracks[{}] {}", t.source, n.name);
            (ti, Width { at, width, class_w, length, limit, neckdown })
        })
        .collect()
}

fn neckdown(cx: &Ctx, r: &mut Report) {
    for (_, w) in widths(cx).into_iter().filter(|(_, w)| w.neckdown) {
        r.emit(
            w.at,
            format!(
                "{:.3} mm neck-down to {}, allowed on runs up to {} mm into a pad",
                w.length,
                Length::mm(w.width),
                w.limit
            ),
        );
    }
}

fn class_width(cx: &Ctx, r: &mut Report) {
    for (ti, w) in widths(cx) {
        if !w.neckdown && w.width + 1e-6 < w.class_w {
            r.emit(
                w.at,
                format!(
                    "{} is narrower than the {} class width {}",
                    Length::mm(w.width),
                    cx.nets[cx.tracks[ti].net].class,
                    Length::mm(w.class_w)
                ),
            );
        }
    }
}

fn impedance_width(cx: &Ctx, r: &mut Report) {
    for (ti, w) in widths(cx) {
        if let Some(c) = class_of(cx.board, &cx.nets[cx.tracks[ti].net].class)
            && c.impedance.is_some()
            && (w.width - w.class_w).abs() > 1e-3
            && !w.neckdown
        {
            r.emit(
                w.at,
                format!(
                    "{} differs from the {} class width {}, its impedance moves",
                    Length::mm(w.width),
                    c.name,
                    Length::mm(w.class_w)
                ),
            );
        }
    }
}

fn impedance_trace(cx: &Ctx, r: &mut Report) {
    let items = cx.copper_items();
    for (ti, t) in cx.tracks.iter().enumerate() {
        let n = &cx.nets[t.net];
        let Some(c) = class_of(cx.board, &n.class) else { continue };
        let (Some(target), Some(g)) = (c.impedance, cx.board.stackup.geometry(&t.layer)) else {
            continue;
        };
        let line = c.line();
        let nominal = g.impedance(c.width_on(&t.layer).to_mm(), line);
        let reach = match g {
            TraceGeometry::Microstrip { h_mm, .. } if line.diff_gap_mm.is_none() => {
                COPLANAR_REACH * h_mm
            }
            _ => 0.0,
        };
        let entry = c.neckdown.map(Length::to_mm).unwrap_or(NECKDOWN);
        let own: Vec<&Vec<Vec<P>>> = cx
            .parts
            .iter()
            .flat_map(|p| &p.pads)
            .filter(|q| q.net == Some(t.net) && q.copper.contains(&t.layer))
            .map(|q| &q.outlines)
            .collect();
        let z_at = |s: Option<f64>| match s {
            Some(s) => g.impedance(t.width, Line::coplanar(s)),
            None => g.impedance(t.width, Line::SINGLE),
        };
        let flat = g.impedance(t.width, line);
        let mut worst: Option<(f64, [Option<f64>; 2])> = None;
        for w in t.points.windows(2) {
            let len = geom::dist(w[0], w[1]);
            let Some(u) = unit(w[0], w[1]) else { continue };
            let mut b = crate::graphic::Bounds::EMPTY;
            b.add_circle(w[0], t.width / 2.0);
            b.add_circle(w[1], t.width / 2.0);
            let (mine, near): (Vec<&super::Cu>, Vec<&super::Cu>) = cx
                .items_near(&b, reach)
                .into_iter()
                .map(|i| &items[i])
                .filter(|it| it.layers.contains(&t.layer))
                .filter(|it| !matches!(it.owner, super::Owner::Track(k) if k == ti))
                .partition(|it| it.net == Some(t.net));
            let zones: Vec<&crate::layout::ZoneFill> =
                cx.zones.iter().filter(|z| z.net != t.net && z.layer == t.layer).collect();
            let copper = |q: P| {
                near.iter().any(|it| it.shape.circle_gap(q, 0.0) <= 0.0)
                    || zones.iter().any(|z| z.filled(q))
            };
            let joined = |q: P| mine.iter().any(|it| it.shape.circle_gap(q, 0.0) <= 0.0);
            let steps = (len / GAP_SAMPLE).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let p = [
                    w[0][0] + u[0] * len * k as f64 / steps as f64,
                    w[0][1] + u[1] * len * k as f64 / steps as f64,
                ];
                let entering = own.iter().any(|o| super::rings_point_gap(o, p) <= entry)
                    || mine.iter().any(|it| it.shape.circle_gap(p, t.width / 2.0) <= entry);
                let (z, gaps) = if reach <= 0.0 || entering {
                    (flat, [None, None])
                } else {
                    let gap = |side: f64| -> Result<Option<f64>, ()> {
                        let nrm = [-u[1] * side, u[0] * side];
                        let mut d = 0.0;
                        while d < reach {
                            let off = t.width / 2.0 + d + GAP_STEP / 2.0;
                            let q = [p[0] + nrm[0] * off, p[1] + nrm[1] * off];
                            if copper(q) {
                                return Ok(Some(d));
                            }
                            if joined(q) {
                                return Err(());
                            }
                            d += GAP_STEP;
                        }
                        Ok(None)
                    };
                    match (gap(1.0), gap(-1.0)) {
                        (Ok(a), Ok(b)) => (2.0 / (1.0 / z_at(a) + 1.0 / z_at(b)), [a, b]),
                        _ => (flat, [None, None]),
                    }
                };
                let off = z / nominal - 1.0;
                if worst.is_none_or(|(o, _)| off.abs() > o.abs()) {
                    worst = Some((off, gaps));
                }
            }
        }
        let Some((off, gaps)) = worst else { continue };
        if off.abs() * 100.0 <= c.impedance_tolerance.0 + 1e-9 {
            continue;
        }
        let beside = match gaps {
            [None, None] if reach > 0.0 && (flat / nominal - 1.0 - off).abs() > 1e-9 => {
                format!(" with no copper within {} either side", Length::mm(reach))
            }
            [None, None] => String::new(),
            [a, b] => format!(
                " with copper {} and {} beside it",
                a.map_or("none".into(), |v| format!("{v:.2} mm")),
                b.map_or("none".into(), |v| format!("{v:.2} mm"))
            ),
        };
        r.emit(
            format!("tracks[{}] {}", t.source, n.name),
            format!(
                "{} wide{beside} puts it near {:.1} ohm, outside {} +/- {}; the class is {} wide{}",
                Length::mm(t.width),
                target.0 * (1.0 + off),
                target,
                c.impedance_tolerance,
                Length::mm(c.width_on(&t.layer).to_mm()),
                c.coplanar_gap.map_or(String::new(), |s| format!(" with the pour {} away", s))
            ),
        );
    }
}

fn acute_turn(cx: &Ctx, r: &mut Report) {
    for (ti, t) in cx.tracks.iter().enumerate() {
        for (k, w) in t.points.windows(3).enumerate() {
            let (Some(u), Some(v)) = (unit(w[0], w[1]), unit(w[1], w[2])) else { continue };
            let turn = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos().to_degrees();
            if turn > 90.5 {
                r.emit(
                    format!("tracks[{ti}] {}", cx.nets[t.net].name),
                    format!(
                        "turns back {:.0} degrees at [{:.3}, {:.3}] (point {}), an acute angle traps etchant; keep bends at 90 degrees or less",
                        turn,
                        w[1][0],
                        w[1][1],
                        k + 1
                    ),
                );
            }
        }
    }
}

fn track_overlap(cx: &Ctx, r: &mut Report) {
    let placed = crate::rules::Placed::new(cx);
    let mut out = Vec::new();
    crate::rules::Rule::eval(&crate::rules::TrackOverlap, &placed, &mut out);
    for v in out {
        r.emit(v.group, v.detail);
    }
}

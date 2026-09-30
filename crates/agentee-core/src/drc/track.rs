use super::{Category, Ctx, Report, Rule, Setup, every};
use crate::diag::Severity;
use crate::geom;
use crate::layout::{NECKDOWN, class_of, parallel_overlap, unit};
use crate::units::Length;
use std::collections::BTreeMap;

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

fn acute_turn(cx: &Ctx, r: &mut Report) {
    for (ti, t) in cx.tracks.iter().enumerate() {
        for (k, w) in t.points.windows(3).enumerate() {
            let (Some(u), Some(v)) = (unit(w[0], w[1]), unit(w[1], w[2])) else { continue };
            let turn = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos().to_degrees();
            if turn > 90.0 + 1e-6 {
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
    let tracks = cx.tracks;
    let mut groups: BTreeMap<(usize, &str), Vec<(usize, usize)>> = BTreeMap::new();
    for (ti, t) in tracks.iter().enumerate() {
        for k in 0..t.points.len().saturating_sub(1) {
            groups.entry((t.net, t.layer.as_str())).or_default().push((ti, k));
        }
    }
    let mut seen: std::collections::HashSet<(usize, usize)> = Default::default();
    for ((net, layer), segs) in groups {
        for x in 0..segs.len() {
            for y in x + 1..segs.len() {
                let ((ta, ka), (tb, kb)) = (segs[x], segs[y]);
                if ta == tb && ka.abs_diff(kb) <= 1 {
                    continue;
                }
                let (a, b) = (&tracks[ta], &tracks[tb]);
                let (a0, a1, b0, b1) =
                    (a.points[ka], a.points[ka + 1], b.points[kb], b.points[kb + 1]);
                let Some((overlap, sep)) = parallel_overlap(a0, a1, b0, b1) else { continue };
                let touch = (a.width + b.width) / 2.0;
                if sep < touch - 1e-6
                    && overlap > a.width.max(b.width)
                    && seen.insert((ta.min(tb), ta.max(tb)))
                {
                    r.emit(
                        format!("tracks[{ta}] {}", cx.nets[net].name),
                        format!(
                            "runs on top of tracks[{tb}] on {layer} for {overlap:.2} mm, {sep:.3} mm apart; the copper is doubled, merge or remove one"
                        ),
                    );
                }
            }
        }
    }
}

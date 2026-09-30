use super::courtyard::body_outlines;
use super::{
    Category, Ctx, HoleOf, Owner, Report, Rule, Setup, is_smd, list, rings_bounds, rings_point_gap,
    vias_in_pads,
};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::layout::{Placed, PlacedPad};
use crate::units::Length;

pub static RULES: &[Rule] = &[
    Rule {
        id: "mlcc-flex-zone-case",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "a ceramic capacitor of case 0805 (2012 metric) or larger inside flex_zone of the board edge, a corner or a mounting hole, where bending cracks it",
        when: "ceramic capacitors",
        applies: with_mlcc,
        check: mlcc_flex_zone_case,
    },
    Rule {
        id: "mlcc-flex-zone",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "a ceramic capacitor inside flex_zone with its long axis pointing at the nearest edge, corner or mounting hole",
        when: "ceramic capacitors",
        applies: with_mlcc,
        check: mlcc_flex_zone,
    },
    Rule {
        id: "mlcc-flex-zone-info",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "counts the small ceramic capacitors inside flex_zone that lie across the stress",
        when: "ceramic capacitors",
        applies: with_mlcc,
        check: mlcc_flex_zone_info,
    },
    Rule {
        id: "tombstone-risk",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "a two-pad chip of 0603 (1608 metric) or smaller whose pads differ in size, shape, via-in-pad or attached copper, so one end wets first and stands the part up",
        when: "chips of 0603 or smaller",
        applies: with_small_chips,
        check: tombstone_risk,
    },
    Rule {
        id: "tall-part-shadow",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "a chip of 0603 or smaller closer to a part taller than 3 mm than that part's height",
        when: "part heights over 3 mm",
        applies: with_tall_parts,
        check: tall_part_shadow,
    },
];

fn with_mlcc(s: &Setup) -> bool {
    s.mlcc
}

fn with_small_chips(s: &Setup) -> bool {
    s.small_chips
}

fn with_tall_parts(s: &Setup) -> bool {
    s.tall_parts
}

fn mm(v: f64) -> Length {
    Length::mm((v * 1000.0).round() / 1000.0)
}

const CASES: &[(&str, &str)] = &[
    ("0402", "01005"),
    ("0603", "0201"),
    ("1005", "0402"),
    ("1608", "0603"),
    ("2012", "0805"),
    ("2520", "1008"),
    ("3216", "1206"),
    ("3225", "1210"),
    ("4516", "1806"),
    ("4532", "1812"),
    ("5025", "2010"),
    ("5750", "2220"),
    ("6332", "2512"),
];

const LARGE_CASE: f64 = 1.8;
const TALL: f64 = 3.0;

pub fn case_of(name: &str) -> Option<(&'static str, f64)> {
    let tokens: Vec<String> =
        name.split(|c: char| !c.is_ascii_alphanumeric()).map(|t| t.to_ascii_lowercase()).collect();
    let metric = tokens
        .iter()
        .filter_map(|t| t.strip_suffix("metric"))
        .find_map(|m| CASES.iter().find(|c| c.0 == m));
    let found = metric.or_else(|| tokens.iter().find_map(|t| CASES.iter().find(|c| c.1 == t)));
    found.map(|(m, imperial)| (*imperial, m[..2].parse::<f64>().unwrap_or(0.0) / 10.0))
}

pub struct Chip {
    pub part: usize,
    pub pads: [usize; 2],
    pub centre: P,
    pub axis: P,
    pub length: f64,
    pub case: Option<&'static str>,
}

impl Chip {
    fn small(&self) -> bool {
        self.length < LARGE_CASE
    }

    fn label(&self) -> String {
        match self.case {
            Some(c) => c.to_string(),
            None => format!("{} pad pitch", mm(self.length)),
        }
    }
}

pub fn chip(part: usize, p: &Placed) -> Option<Chip> {
    let cu: Vec<usize> = p
        .pads
        .iter()
        .enumerate()
        .filter(|(_, q)| !q.copper.is_empty() || q.drill.is_some())
        .map(|(k, _)| k)
        .collect();
    if cu.len() != 2 || !cu.iter().all(|&k| is_smd(&p.pads[k])) {
        return None;
    }
    let a = rings_bounds(&p.pads[cu[0]].outlines).center();
    let b = rings_bounds(&p.pads[cu[1]].outlines).center();
    let d = geom::dist(a, b);
    if d < 1e-6 {
        return None;
    }
    let (case, length) = match case_of(&p.footprint_name) {
        Some((c, l)) => (Some(c), l),
        None => (None, d),
    };
    Some(Chip {
        part,
        pads: [cu[0], cu[1]],
        centre: [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
        axis: [(b[0] - a[0]) / d, (b[1] - a[1]) / d],
        length,
        case,
    })
}

fn is_mlcc(p: &Placed) -> bool {
    if let Some(v) = p.mlcc {
        return v;
    }
    let n = p.footprint_name.to_ascii_lowercase();
    if n.starts_with("cp_") || ["tantal", "elec", "polymer"].iter().any(|w| n.contains(w)) {
        return false;
    }
    let c_ref =
        p.reference.strip_prefix('C').is_some_and(|d| d.starts_with(|c: char| c.is_ascii_digit()));
    n.starts_with("c_") || n.contains("capacitor") || c_ref
}

pub fn mlcc_chips(parts: &[Placed]) -> Vec<Chip> {
    parts.iter().enumerate().filter(|(_, p)| is_mlcc(p)).filter_map(|(i, p)| chip(i, p)).collect()
}

pub fn has_small_chips(parts: &[Placed]) -> bool {
    parts.iter().enumerate().any(|(i, p)| chip(i, p).is_some_and(|c| c.small()))
}

pub fn has_tall_parts(parts: &[Placed]) -> bool {
    parts.iter().any(|p| crate::height::body_height(&p.footprint).is_some_and(|h| h > TALL))
}

struct StressHole {
    name: String,
    a: P,
    b: P,
    r: f64,
}

fn stress_holes(cx: &Ctx) -> Vec<StressHole> {
    let mut out: Vec<StressHole> = Vec::new();
    let mut mounting: Vec<(usize, usize)> = Vec::new();
    for h in cx.holes() {
        let HoleOf::Pad(pi, _) = h.of else { continue };
        let p = &cx.parts[pi];
        if p.footprint_name.to_ascii_lowercase().contains("mountinghole") {
            let hole = StressHole {
                name: format!("mounting hole {}", p.reference),
                a: h.a,
                b: h.b,
                r: h.r,
            };
            match mounting.iter().find(|m| m.0 == pi) {
                Some(&(_, i)) if out[i].r >= h.r => {}
                Some(&(_, i)) => out[i] = hole,
                None => {
                    mounting.push((pi, out.len()));
                    out.push(hole);
                }
            }
        } else if !h.plated && h.size[0].min(h.size[1]) >= 2.0 - 1e-6 {
            out.push(StressHole { name: cx.hole_name(&h), a: h.a, b: h.b, r: h.r });
        }
    }
    out
}

struct Flex {
    chip: Chip,
    gap: f64,
    what: String,
    pointing: bool,
}

fn closest_on(p: P, a: P, b: P) -> P {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    };
    [a[0] + t * dx, a[1] + t * dy]
}

fn turn_at(outline: &[P], i: usize) -> f64 {
    let n = outline.len();
    let (p, c, q) = (outline[(i + n - 1) % n], outline[i], outline[(i + 1) % n]);
    let (u, v) = ([c[0] - p[0], c[1] - p[1]], [q[0] - c[0], q[1] - c[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu < 1e-9 || lv < 1e-9 {
        return 0.0;
    }
    ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).clamp(-1.0, 1.0).acos().to_degrees()
}

fn flex_zone_hits(cx: &Ctx) -> Vec<Flex> {
    let zone = cx.board.rules.flex_zone.to_mm();
    let holes = stress_holes(cx);
    let outline = cx.outline;
    let n = outline.len();
    let mut out = Vec::new();
    for c in mlcc_chips(cx.parts) {
        let p = &cx.parts[c.part];
        let rings: Vec<&Vec<P>> = c.pads.iter().flat_map(|&k| p.pads[k].outlines.iter()).collect();
        let mut best: Option<(f64, P, String)> = None;
        if n >= 3 {
            for j in 0..n {
                let (a, b) = (outline[j], outline[(j + 1) % n]);
                let gap = rings
                    .iter()
                    .flat_map(|o| (0..o.len()).map(move |i| (o[i], o[(i + 1) % o.len()])))
                    .map(|(s, e)| geom::segment_segment_distance(s, e, a, b))
                    .fold(f64::MAX, f64::min);
                if best.as_ref().is_none_or(|b| gap < b.0) {
                    let at = closest_on(c.centre, a, b);
                    let corner = [j, (j + 1) % n]
                        .into_iter()
                        .find(|&v| geom::dist(at, outline[v]) < 1e-6 && turn_at(outline, v) > 30.0);
                    let what = match corner {
                        Some(v) => format!(
                            "the board corner at [{:.3}, {:.3}]",
                            outline[v][0], outline[v][1]
                        ),
                        None => format!("the board edge at [{:.3}, {:.3}]", at[0], at[1]),
                    };
                    best = Some((gap, at, what));
                }
            }
        }
        for h in &holes {
            let gap = rings
                .iter()
                .map(|o| geom::polyline_polygon_distance(&[h.a, h.b], o) - h.r)
                .fold(f64::MAX, f64::min)
                .max(0.0);
            if best.as_ref().is_none_or(|b| gap < b.0) {
                let at = [(h.a[0] + h.b[0]) / 2.0, (h.a[1] + h.b[1]) / 2.0];
                best = Some((gap, at, h.name.clone()));
            }
        }
        let Some((gap, at, what)) = best else { continue };
        if gap + 1e-6 >= zone {
            continue;
        }
        let d = geom::dist(at, c.centre);
        let pointing = d > 1e-6
            && ((at[0] - c.centre[0]) * c.axis[0] + (at[1] - c.centre[1]) * c.axis[1]).abs() / d
                > std::f64::consts::FRAC_1_SQRT_2 + 1e-9;
        out.push(Flex { chip: c, gap, what, pointing });
    }
    out
}

fn mlcc_flex_zone_case(cx: &Ctx, r: &mut Report) {
    let zone = cx.board.rules.flex_zone;
    for f in flex_zone_hits(cx).into_iter().filter(|f| !f.chip.small()) {
        r.emit(
            format!("part {}", cx.parts[f.chip.part].reference),
            format!(
                "{} ceramic capacitor {} from {}, inside the {zone} flex zone where board bending cracks 0805 and larger MLCCs; move it out, use a smaller case or a soft termination part",
                f.chip.label(),
                mm(f.gap),
                f.what
            ),
        );
    }
}

fn mlcc_flex_zone(cx: &Ctx, r: &mut Report) {
    let zone = cx.board.rules.flex_zone;
    for f in flex_zone_hits(cx).into_iter().filter(|f| f.chip.small() && f.pointing) {
        r.emit(
            format!("part {}", cx.parts[f.chip.part].reference),
            format!(
                "{} ceramic capacitor {} from {} with its long axis pointing at it, inside the {zone} flex zone; turn it 90 degrees so bending does not pull its ends apart, or move it out",
                f.chip.label(),
                mm(f.gap),
                f.what
            ),
        );
    }
}

fn mlcc_flex_zone_info(cx: &Ctx, r: &mut Report) {
    let zone = cx.board.rules.flex_zone;
    let refs: Vec<String> = flex_zone_hits(cx)
        .into_iter()
        .filter(|f| f.chip.small() && !f.pointing)
        .map(|f| cx.parts[f.chip.part].reference.clone())
        .collect();
    if !refs.is_empty() {
        r.emit(
            "parts",
            format!(
                "{} small ceramic capacitors sit inside the {zone} flex zone, lying along the edge: {}",
                refs.len(),
                list(&refs)
            ),
        );
    }
}

const REACH: f64 = 0.3;
const STEP: f64 = 0.025;

fn pad_area(q: &PlacedPad) -> f64 {
    q.outlines.iter().map(|o| crate::contour::area(o).abs()).sum()
}

fn attached_copper(cx: &Ctx, pi: usize, k: usize) -> f64 {
    let p = &cx.parts[pi];
    let q = &p.pads[k];
    let Some(net) = q.net else { return 0.0 };
    let b = rings_bounds(&q.outlines);
    let items = cx.copper_items();
    let near: Vec<usize> = cx
        .items_near(&b, REACH)
        .into_iter()
        .filter(|&i| {
            let c = &items[i];
            c.net == Some(net)
                && c.layers.iter().any(|l| q.copper.contains(l))
                && !matches!(c.owner, Owner::Pad(o, _) if o == pi)
        })
        .collect();
    let zones: Vec<&crate::layout::ZoneFill> =
        cx.zones.iter().filter(|z| z.net == net && q.copper.contains(&z.layer)).collect();
    let own: Vec<&Vec<P>> = p.pads.iter().flat_map(|o| o.outlines.iter()).collect();
    let size = b.size();
    let nx = ((size[0] + 2.0 * REACH) / STEP).ceil() as usize;
    let ny = ((size[1] + 2.0 * REACH) / STEP).ceil() as usize;
    let mut hit = 0usize;
    for i in 0..nx {
        for j in 0..ny {
            let s = [
                b.min[0] - REACH + (i as f64 + 0.5) * STEP,
                b.min[1] - REACH + (j as f64 + 0.5) * STEP,
            ];
            if own.iter().any(|o| geom::point_in_polygon(s, o))
                || rings_point_gap(&q.outlines, s) > REACH
            {
                continue;
            }
            if zones.iter().any(|z| z.filled(s))
                || near.iter().any(|&i| items[i].shape.circle_gap(s, 0.0) <= 0.0)
            {
                hit += 1;
            }
        }
    }
    hit as f64 * STEP * STEP
}

struct Feed {
    spokes: f64,
    tracks: f64,
    vias: usize,
}

impl Feed {
    fn unfed(&self) -> bool {
        self.spokes <= 0.0 && self.tracks <= 0.0 && self.vias == 0
    }
}

fn pad_feed(cx: &Ctx, pi: usize, k: usize) -> Option<Feed> {
    let q = &cx.parts[pi].pads[k];
    let net = q.net?;
    let step = 0.02;
    let pb = rings_bounds(&q.outlines);
    let mut spokes = 0.0;
    for (z, f) in cx.zones.iter().zip(cx.fills()) {
        if z.net != net
            || !q.copper.contains(&z.layer)
            || !super::near(&f.bounds, pb.center(), pb.size()[0].max(pb.size()[1]))
        {
            continue;
        }
        let samples = super::copper::boundary_samples(&q.outlines, step, 0.05);
        let hit = samples.iter().filter(|s| f.contains(**s)).count();
        if hit * 2 >= samples.len() && hit > 0 {
            return None;
        }
        spokes += hit as f64 * step;
    }
    let items = cx.copper_items();
    let mut fed: Vec<usize> = Vec::new();
    let mut vias = 0;
    for i in cx.items_near(&pb, 0.0) {
        let c = &items[i];
        if c.net != Some(net) || !c.layers.iter().any(|l| q.copper.contains(l)) {
            continue;
        }
        if let (Owner::Via(_), super::CuShape::Circle(o, rv)) = (&c.owner, &c.shape) {
            if rings_point_gap(&q.outlines, *o) <= *rv + 1e-6 {
                vias += 1;
            }
            continue;
        }
        let Owner::Track(ti) = c.owner else { continue };
        if fed.contains(&ti) {
            continue;
        }
        let super::CuShape::Seg(a, b, hw) = c.shape else { continue };
        let touches = q.outlines.iter().any(|o| {
            geom::point_in_polygon(a, o)
                || geom::point_in_polygon(b, o)
                || geom::polyline_polygon_distance(&[a, b], o) <= hw + 1e-6
        });
        if touches {
            fed.push(ti);
        }
    }
    let tracks = fed.iter().map(|&t| cx.tracks[t].width).sum();
    Some(Feed { spokes, tracks, vias })
}

fn tombstone_risk(cx: &Ctx, r: &mut Report) {
    let ratio = cx.board.drc.tombstone_ratio.unwrap_or(3.0);
    let vip = vias_in_pads(cx.parts, cx.vias);
    for (pi, p) in cx.parts.iter().enumerate() {
        let Some(c) = chip(pi, p).filter(Chip::small) else { continue };
        let [ka, kb] = c.pads;
        let (qa, qb) = (&p.pads[ka], &p.pads[kb]);
        let (na, nb) = (&qa.number, &qb.number);
        let mut why = Vec::new();
        let dims = |k: usize| {
            let s = p.footprint.pads.get(k).map(|f| f.size.to_mm()).unwrap_or([0.0; 2]);
            [s[0].min(s[1]), s[0].max(s[1])]
        };
        let (da, db) = (dims(ka), dims(kb));
        let (aa, ab) = (pad_area(qa), pad_area(qb));
        let shape = |k: usize| p.footprint.pads.get(k).map(|f| f.shape);
        if shape(ka) != shape(kb)
            || (da[0] - db[0]).abs() > 0.02
            || (da[1] - db[1]).abs() > 0.02
            || (aa - ab).abs() > 0.05 * aa.max(ab)
        {
            why.push(format!("pads {na} and {nb} differ in size or shape"));
        }
        let in_pad = |k: usize| vip.iter().any(|v| v.1 == pi && v.2 == k);
        match (in_pad(ka), in_pad(kb)) {
            (true, false) => why.push(format!("pad {na} has a via in it and pad {nb} not")),
            (false, true) => why.push(format!("pad {nb} has a via in it and pad {na} not")),
            _ => {}
        }
        let feeds = (pad_feed(cx, pi, ka), pad_feed(cx, pi, kb));
        let unfed = |f: &Option<Feed>| f.as_ref().is_some_and(|f| f.unfed());
        let necks = match &feeds {
            _ if unfed(&feeds.0) || unfed(&feeds.1) => None,
            (Some(fa), Some(fb))
                if fa.spokes + fb.spokes > 0.0
                    && fa.spokes + fa.tracks > 0.0
                    && fb.spokes + fb.tracks > 0.0 =>
            {
                Some((fa.spokes + fa.tracks, fb.spokes + fb.tracks))
            }
            _ => None,
        };
        if let Some((wa, wb)) = necks {
            let (lo, hi) = (wa.min(wb), wa.max(wb));
            if hi > ratio * lo {
                let (big, small) = if wa >= wb { (na, nb) } else { (nb, na) };
                why.push(format!(
                    "pad {big} is fed by {} of spokes and tracks and pad {small} by {}",
                    mm(hi),
                    mm(lo)
                ));
            }
        } else if qa.net.is_some() && qb.net.is_some() && !unfed(&feeds.0) && !unfed(&feeds.1) {
            let (ca, cb) = (attached_copper(cx, pi, ka), attached_copper(cx, pi, kb));
            let (lo, hi) = (ca.min(cb), ca.max(cb));
            if hi >= 0.1 && hi > ratio * lo.max(0.02) {
                let (big, small) = if ca >= cb { (na, nb) } else { (nb, na) };
                why.push(format!(
                    "pad {big} has {hi:.2} mm2 of copper within {} and pad {small} {lo:.2} mm2",
                    mm(REACH)
                ));
            }
        }
        if !why.is_empty() {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} chip may tombstone: {}; the end that heats slower wets last and the other end stands the part up, balance the lands and the copper on them",
                    c.label(),
                    why.join("; ")
                ),
            );
        }
    }
}

fn tall_part_shadow(cx: &Ctx, r: &mut Report) {
    let tall: Vec<(usize, f64, Vec<Vec<P>>)> = cx
        .parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            crate::height::body_height(&p.footprint)
                .filter(|h| *h > TALL)
                .map(|h| (i, h, body_outlines(p).1))
        })
        .collect();
    for (pi, p) in cx.parts.iter().enumerate() {
        let Some(c) = chip(pi, p).filter(Chip::small) else { continue };
        let rings: Vec<&Vec<P>> = c.pads.iter().flat_map(|&k| p.pads[k].outlines.iter()).collect();
        let mut worst: Option<(f64, f64, usize)> = None;
        for (ti, h, body) in &tall {
            if *ti == pi || cx.parts[*ti].bottom != p.bottom {
                continue;
            }
            let gap = rings
                .iter()
                .flat_map(|a| body.iter().map(move |b| geom::polygon_distance(a, b)))
                .fold(f64::MAX, f64::min);
            if gap + 1e-6 < *h && worst.is_none_or(|w| gap / h < w.0 / w.1) {
                worst = Some((gap, *h, *ti));
            }
        }
        if let Some((gap, h, ti)) = worst {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} chip {} from {}, which is {} tall: it sits in that part's shadow for reflow heat and inspection, keep small parts at least the tall part's height away",
                    c.label(),
                    mm(gap),
                    cx.parts[ti].reference,
                    mm(h)
                ),
            );
        }
    }
}

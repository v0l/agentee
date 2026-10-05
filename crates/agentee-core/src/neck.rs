use crate::board::Board;
use crate::geom::{self, P};
use crate::layout::{DRC_EPSILON, Layout, NECKDOWN, class_of, glob};
use crate::tune::{Obstacle, obstacles_of};
use crate::units::Length;
use serde::Serialize;

const TAPER_STEPS: usize = 3;

#[derive(Clone, Debug)]
pub struct NeckOptions {
    pub nets: Vec<String>,
    pub taper: bool,
}

impl Default for NeckOptions {
    fn default() -> Self {
        NeckOptions { nets: vec!["*".into()], taper: false }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Necked {
    pub track: usize,
    pub net: String,
    pub pad: String,
    pub why: String,
    pub width_mm: f64,
    pub length_mm: f64,
    pub segments: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct NeckFailed {
    pub track: usize,
    pub net: String,
    pub pad: String,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NeckSegment {
    pub width: f64,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NeckEdit {
    pub track: usize,
    pub points: Vec<P>,
    pub necks: Vec<NeckSegment>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NeckResult {
    pub necked: Vec<Necked>,
    pub failed: Vec<NeckFailed>,
    #[serde(skip)]
    pub edits: Vec<NeckEdit>,
}

struct Rules<'a> {
    net: usize,
    layer: &'a str,
    clearance: f64,
    edge: f64,
    outline: &'a [P],
    near: Vec<&'a Obstacle>,
}

impl Rules<'_> {
    fn gap(&self, line: &[P]) -> f64 {
        let n = self.outline.len();
        let edges = (0..n).map(|i| (self.outline[i], self.outline[(i + 1) % n])).map(|(a, b)| {
            line.windows(2)
                .map(|w| geom::segment_segment_distance(w[0], w[1], a, b))
                .fold(f64::MAX, f64::min)
                - self.edge
        });
        self.near
            .iter()
            .filter(|o| o.net != Some(self.net) && o.layers.iter().any(|l| l == self.layer))
            .map(|o| o.distance(line) - self.clearance.max(o.clearance))
            .chain(edges)
            .fold(f64::MAX, f64::min)
    }
}

fn length(line: &[P]) -> f64 {
    line.windows(2).map(|w| geom::dist(w[0], w[1])).sum()
}

fn along(line: &[P], s: f64) -> P {
    let mut left = s;
    for w in line.windows(2) {
        let d = geom::dist(w[0], w[1]);
        if left <= d && d > 1e-12 {
            let t = left / d;
            return [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
        }
        left -= d;
    }
    line[line.len() - 1]
}

fn cut_point(line: &[P], s: f64) -> P {
    if s <= 1e-12 {
        return line[0];
    }
    if s >= length(line) - 1e-12 {
        return line[line.len() - 1];
    }
    let q = along(line, s);
    [(q[0] * 1e4).round() / 1e4, (q[1] * 1e4).round() / 1e4]
}

fn sub(line: &[P], a: f64, b: f64) -> Vec<P> {
    let mut out = vec![cut_point(line, a)];
    let mut s = 0.0;
    for w in line.windows(2) {
        s += geom::dist(w[0], w[1]);
        if s > a + 1e-9 && s < b - 1e-9 {
            out.push(w[1]);
        }
    }
    out.push(cut_point(line, b));
    out
}

fn floor_01(w: f64) -> f64 {
    (w * 100.0 + 1e-6).floor() / 100.0
}

pub(crate) fn neck_width(raw: f64, min_w: f64) -> f64 {
    let w = floor_01(raw);
    if w < min_w - 1e-9 && raw >= min_w - 1e-9 { min_w } else { w }
}

struct Cut {
    at: f64,
    necks: Vec<NeckSegment>,
}

#[allow(clippy::too_many_arguments)]
fn cut(
    path: &[P],
    pad: &[P],
    pad_w: f64,
    rules: &Rules,
    wide: f64,
    min_w: f64,
    limit: f64,
    taper: bool,
) -> Result<Option<Cut>, String> {
    let total = length(path);
    let narrow = wide > pad_w + 1e-6;
    let window = (limit + wide).min(total);
    let crowded = rules.gap(&sub(path, 0.0, window)) + DRC_EPSILON < wide / 2.0;
    if !narrow && !crowded {
        return Ok(None);
    }
    if wide <= min_w + 1e-9 {
        return Err(format!("the track is already at min_track_width {}", Length::mm(min_w)));
    }
    let step = 0.01;
    let mut k = 1;
    let mut why = format!("no neck within {} keeps clearance", Length::mm(limit));
    while k as f64 * step <= limit + 1e-9 {
        let at = k as f64 * step;
        k += 1;
        if at > total - 1e-6 {
            why = "the track is too short to neck down".into();
            break;
        }
        let head = sub(path, 0.0, at);
        let width = neck_width(wide.min(pad_w).min(2.0 * rules.gap(&head)), min_w);
        if width < min_w - 1e-9 {
            why = format!(
                "a neck of {} would be under min_track_width {}",
                Length::mm(width.max(0.0)),
                Length::mm(min_w)
            );
            break;
        }
        if width >= wide - 1e-6 {
            continue;
        }
        if narrow && geom::point_in_polygon(cut_point(path, at), pad) {
            continue;
        }
        let tail = sub(path, at, window.max(at));
        if rules.gap(&tail) + DRC_EPSILON < wide / 2.0 {
            continue;
        }
        let mut necks = vec![NeckSegment { width, points: head }];
        let mut reach = at;
        if taper {
            let end = (3.0 * at).min(limit).min(total - step);
            let piece = (end - at) / (TAPER_STEPS - 1) as f64;
            if piece >= step - 1e-9 {
                for i in 1..TAPER_STEPS {
                    let w = floor_01(width + (wide - width) * i as f64 / TAPER_STEPS as f64);
                    let (a, b) = (at + piece * (i - 1) as f64, at + piece * i as f64);
                    if w >= wide - 1e-6 {
                        break;
                    }
                    let last = necks.last_mut().unwrap();
                    if w <= last.width + 1e-9 {
                        last.points.extend(sub(path, a, b).into_iter().skip(1));
                    } else {
                        necks.push(NeckSegment { width: w, points: sub(path, a, b) });
                    }
                    reach = b;
                }
            }
        }
        return Ok(Some(Cut { at: reach, necks }));
    }
    Err(why)
}

fn deepest_pad<'a>(p: P, pads: &[(String, &'a Vec<P>)]) -> Option<(String, &'a Vec<P>)> {
    let depth = |o: &[P]| crate::drc::edge_distance(o, p);
    pads.iter()
        .filter(|(_, o)| geom::point_in_polygon(p, o))
        .fold(None, |best: Option<&(String, &Vec<P>)>, c| match best {
            Some(b) if depth(b.1) >= depth(c.1) => Some(b),
            _ => Some(c),
        })
        .map(|(n, o)| (n.clone(), *o))
}

fn joined_pad<'a>(outline: &Vec<P>, others: impl Iterator<Item = &'a Vec<P>>) -> bool {
    let mut ring = outline.clone();
    ring.push(outline[0]);
    others
        .into_iter()
        .any(|o| !std::ptr::eq(o, outline) && geom::polyline_polygon_distance(&ring, o) < 1e-6)
}

pub fn neck(layout: &Layout, board: &Board, opts: &NeckOptions) -> Result<NeckResult, String> {
    let obstacles = obstacles_of(layout, board);
    let edge = board.rules.min_copper_to_edge.to_mm();
    let min_w = board.rules.min_track_width.to_mm();
    let mut out = NeckResult::default();
    for t in &layout.tracks {
        let net = &layout.nets[t.net];
        if t.points.len() < 2 || !opts.nets.iter().any(|g| glob(g, &net.name)) {
            continue;
        }
        let class = class_of(board, &net.class);
        let limit = class.and_then(|c| c.neckdown).map(Length::to_mm).unwrap_or(NECKDOWN);
        let mut points = t.points.clone();
        let mut necks: Vec<NeckSegment> = Vec::new();
        for end in 0..2 {
            let p = if end == 0 { points[0] } else { points[points.len() - 1] };
            let same_net: Vec<(String, &Vec<P>)> = layout
                .parts
                .iter()
                .flat_map(|part| part.pads.iter().map(move |pad| (part, pad)))
                .filter(|(_, pad)| pad.net == Some(t.net) && pad.copper.contains(&t.layer))
                .flat_map(|(part, pad)| {
                    pad.outlines
                        .iter()
                        .map(move |o| (format!("{}.{}", part.reference, pad.number), o))
                })
                .collect();
            let Some((name, outline)) = deepest_pad(p, &same_net) else { continue };
            let joined = joined_pad(outline, same_net.iter().map(|(_, o)| *o));
            let pad_w = if joined { f64::INFINITY } else { geom::min_extent(outline) };
            let reach = limit + t.width + net.clearance + 1.0;
            let near: Vec<&Obstacle> = obstacles
                .iter()
                .filter(|o| {
                    o.lo[0] <= p[0] + reach
                        && o.lo[1] <= p[1] + reach
                        && o.hi[0] >= p[0] - reach
                        && o.hi[1] >= p[1] - reach
                })
                .collect();
            let rules = Rules {
                net: t.net,
                layer: &t.layer,
                clearance: net.clearance,
                edge,
                outline: &layout.outline,
                near,
            };
            let path: Vec<P> =
                if end == 0 { points.clone() } else { points.iter().rev().copied().collect() };
            match cut(&path, outline, pad_w, &rules, t.width, min_w, limit, opts.taper) {
                Ok(None) => {}
                Ok(Some(c)) => {
                    let why = if t.width > pad_w + 1e-6 {
                        "wider than the pad"
                    } else {
                        "clearance near the pad"
                    };
                    out.necked.push(Necked {
                        track: t.source,
                        net: net.name.clone(),
                        pad: name,
                        why: why.into(),
                        width_mm: c.necks[0].width,
                        length_mm: (c.at * 1e4).round() / 1e4,
                        segments: c.necks.len(),
                    });
                    let rest = sub(&path, c.at, length(&path));
                    points = if end == 0 { rest } else { rest.into_iter().rev().collect() };
                    necks.extend(c.necks);
                }
                Err(why) => out.failed.push(NeckFailed {
                    track: t.source,
                    net: net.name.clone(),
                    pad: name,
                    why,
                }),
            }
        }
        if !necks.is_empty() {
            out.edits.push(NeckEdit { track: t.source, points, necks });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: P, h: f64) -> Vec<P> {
        vec![[c[0] - h, c[1] - h], [c[0] + h, c[1] - h], [c[0] + h, c[1] + h], [c[0] - h, c[1] + h]]
    }

    #[test]
    fn a_pad_touching_another_same_net_pad_counts_as_joined() {
        let pad = square([0.0, 0.0], 0.5);
        let touching = square([1.0, 0.0], 0.5);
        let apart = square([1.2, 0.0], 0.5);
        assert!(joined_pad(&pad, [&pad, &touching].into_iter()));
        assert!(!joined_pad(&pad, [&pad, &apart].into_iter()));
        assert!(!joined_pad(&pad, std::iter::once(&pad)));
        let copy = pad.clone();
        assert!(joined_pad(&pad, std::iter::once(&copy)));
    }

    #[test]
    fn the_pad_that_holds_the_end_deepest_wins() {
        let big = square([0.0, 0.0], 1.0);
        let small = square([0.8, 0.0], 0.3);
        let pads = vec![("U1.1".to_string(), &small), ("U1.2".to_string(), &big)];
        assert_eq!(deepest_pad([0.7, 0.0], &pads).unwrap().0, "U1.2");
        assert_eq!(deepest_pad([0.8, 0.0], &pads).unwrap().0, "U1.1");
        assert!(deepest_pad([3.0, 0.0], &pads).is_none());
    }
}

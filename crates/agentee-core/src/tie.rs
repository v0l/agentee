use crate::board::Board;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::{Layout, glob};
use crate::route::{RoutedTrack, RoutedVia};
use serde::Serialize;

const NEAR_VIA: f64 = 0.8;
const PAD_GAP: f64 = 0.05;
const REACH: f64 = 1.2;
const STEP: f64 = 0.05;

#[derive(Clone, Debug, Default, Serialize)]
pub struct TieResult {
    pub tied: usize,
    pub already: usize,
    pub tracks: Vec<RoutedTrack>,
    pub vias: Vec<RoutedVia>,
    pub failed: Vec<String>,
}

#[derive(Clone)]
enum Shape {
    Poly(Vec<P>),
    Seg(P, P, f64),
    Circle(P, f64),
}

impl Shape {
    fn to_point(&self, c: P) -> f64 {
        match self {
            Shape::Poly(v) if geom::point_in_polygon(c, v) => 0.0,
            Shape::Poly(v) => (0..v.len())
                .map(|i| geom::point_segment_distance(c, v[i], v[(i + 1) % v.len()]))
                .fold(f64::MAX, f64::min),
            Shape::Seg(a, b, r) => (geom::point_segment_distance(c, *a, *b) - r).max(0.0),
            Shape::Circle(o, r) => (geom::dist(c, *o) - r).max(0.0),
        }
    }

    fn to_segment(&self, a: P, b: P) -> f64 {
        match self {
            Shape::Poly(v) => geom::polyline_polygon_distance(&[a, b], v),
            Shape::Seg(p, q, r) => (geom::segment_segment_distance(a, b, *p, *q) - r).max(0.0),
            Shape::Circle(o, r) => (geom::point_segment_distance(*o, a, b) - r).max(0.0),
        }
    }
}

struct Item {
    net: Option<usize>,
    layers: Vec<String>,
    shape: Shape,
    smd: bool,
}

struct Hole {
    at: P,
    r: f64,
}

pub fn plane_nets(layout: &Layout) -> Vec<usize> {
    let mut v: Vec<usize> = layout.zones.iter().map(|z| z.net).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn is_bga(fp_name: &str, pads: usize) -> bool {
    fp_name.to_ascii_lowercase().contains("bga") || pads >= 64
}

pub fn tie(layout: &Layout, board: &Board, nets: &[String]) -> Result<TieResult, String> {
    let planes: Vec<usize> = plane_nets(layout)
        .into_iter()
        .filter(|n| nets.is_empty() || nets.iter().any(|g| glob(g, &layout.nets[*n].name)))
        .collect();
    if planes.is_empty() {
        return Err("no net with a zone to tie pads to; add a [[zones]] first".into());
    }
    let rules = &board.rules;
    let hole_hole = rules.min_hole_to_hole.to_mm();
    let hole_cu = rules.min_via_hole_to_copper.to_mm();
    let hole_smd = rules.min_hole_to_smd_pad.to_mm();
    let edge_gap = rules.min_copper_to_edge.to_mm();
    let edge = layout.edge();
    let copper = &layout.copper;

    let mut items: Vec<Item> = Vec::new();
    let mut holes: Vec<Hole> = Vec::new();
    for p in &layout.parts {
        for pad in &p.pads {
            for o in &pad.outlines {
                items.push(Item {
                    net: pad.net,
                    layers: pad.copper.clone(),
                    shape: Shape::Poly(o.clone()),
                    smd: pad.drill.is_none() && !pad.copper.is_empty(),
                });
            }
            if let Some((c, s, _)) = pad.drill {
                holes.push(Hole { at: c, r: s[0].max(s[1]) / 2.0 });
            }
        }
    }
    for t in &layout.tracks {
        for w in t.points.windows(2) {
            items.push(Item {
                net: Some(t.net),
                layers: vec![t.layer.clone()],
                shape: Shape::Seg(w[0], w[1], t.width / 2.0),
                smd: false,
            });
        }
    }
    let stitched =
        |v: &&crate::layout::Via| matches!(v.source, crate::layout::ViaSource::Stitch(_));
    for v in layout.vias.iter().filter(|v| !stitched(v)) {
        items.push(Item {
            net: Some(v.net),
            layers: v.layers.clone(),
            shape: Shape::Circle(v.at, v.diameter / 2.0),
            smd: false,
        });
        holes.push(Hole { at: v.at, r: v.drill / 2.0 });
    }
    let mut placed: Vec<(usize, P)> = layout
        .vias
        .iter()
        .filter(|v| !stitched(v) && planes.contains(&v.net))
        .map(|v| (v.net, v.at))
        .collect();

    let mut out = TieResult::default();
    for part in &layout.parts {
        if is_bga(&part.footprint_name, part.pads.len()) {
            continue;
        }
        let mut body = Bounds::EMPTY;
        part.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| body.add(*q));
        let centre = body.center();
        for pad in &part.pads {
            let Some(net) = pad.net else { continue };
            if !planes.contains(&net) || pad.drill.is_some() || pad.kind == PadKind::Npth {
                continue;
            }
            let Some(layer) = pad.copper.first().cloned() else { continue };
            let zone_layers: Vec<&String> = layout
                .zones
                .iter()
                .filter(|z| z.net == net && z.layer != layer)
                .map(|z| &z.layer)
                .collect();
            let at_layer = |l: &String| copper.iter().position(|c| c == l).unwrap_or(0);
            let here = at_layer(&layer);
            let Some(target) = zone_layers.iter().min_by_key(|l| at_layer(l).abs_diff(here)) else {
                continue;
            };
            let mut pb = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| pb.add(*q));
            let pc = pb.center();
            let reach = pb.size()[0].max(pb.size()[1]) / 2.0 + NEAR_VIA;
            if placed.iter().any(|(n, at)| *n == net && geom::dist(*at, pc) <= reach) {
                out.already += 1;
                continue;
            }
            let class = board.netclasses.iter().find(|c| c.name == layout.nets[net].class);
            let reach_layers = [layer.as_str(), target.as_str()];
            let Some(spec) = board.via_for(None, class, &reach_layers) else {
                out.failed.push(format!(
                    "{}.{}: the board defines no [[vias]]",
                    part.reference, pad.number
                ));
                continue;
            };
            let vlayers = spec.copper_layers(copper);
            let (vr, dr) = (spec.diameter.to_mm() / 2.0, spec.drill.to_mm() / 2.0);
            let width = layout.nets[net].width.min(pb.size()[0].min(pb.size()[1]));
            let clear_of = |other: Option<usize>| {
                let own = layout.nets[net].clearance;
                other.map(|o| own.max(layout.nets[o].clearance)).unwrap_or(own)
            };
            let own_pad: Vec<Shape> = pad.outlines.iter().map(|o| Shape::Poly(o.clone())).collect();
            let off_own = |c: P| {
                own_pad.iter().map(|s| s.to_point(c)).fold(f64::MAX, f64::min)
                    >= (vr + PAD_GAP).max(dr + hole_smd)
            };
            let legal = |c: P| -> bool {
                if !edge.contains(c) || edge.distance(c) < edge_gap + vr {
                    return false;
                }
                if holes.iter().any(|h| geom::dist(h.at, c) - h.r - dr < hole_hole) {
                    return false;
                }
                for it in &items {
                    let shares = it.layers.iter().any(|l| vlayers.contains(l));
                    let d = it.shape.to_point(c);
                    if it.smd && shares && (d < vr + PAD_GAP || d < dr + hole_smd) {
                        return false;
                    }
                    if it.net == Some(net) {
                        continue;
                    }
                    if shares && d < vr + clear_of(it.net) {
                        return false;
                    }
                    if d < dr + hole_cu {
                        return false;
                    }
                }
                items.iter().filter(|it| it.net != Some(net)).all(|it| {
                    !it.layers.contains(&layer)
                        || it.shape.to_segment(pc, c) >= width / 2.0 + clear_of(it.net)
                })
            };
            let out_dir = {
                let d = [pc[0] - centre[0], pc[1] - centre[1]];
                let n = (d[0] * d[0] + d[1] * d[1]).sqrt();
                if n < 1e-6 { [0.0, -1.0] } else { [d[0] / n, d[1] / n] }
            };
            let mut dirs: Vec<P> = (0..16)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / 16.0;
                    [a.cos(), a.sin()]
                })
                .collect();
            dirs.sort_by(|a, b| {
                let da = a[0] * out_dir[0] + a[1] * out_dir[1];
                let db = b[0] * out_dir[0] + b[1] * out_dir[1];
                db.total_cmp(&da)
            });
            let mut found = None;
            'dirs: for u in &dirs {
                let mut t = 0.0;
                while !off_own([pc[0] + u[0] * t, pc[1] + u[1] * t]) && t < 5.0 {
                    t += STEP;
                }
                let start = t;
                while t <= start + REACH {
                    let c = [pc[0] + u[0] * t, pc[1] + u[1] * t];
                    if legal(c) {
                        found = Some(c);
                        break 'dirs;
                    }
                    t += STEP;
                }
            }
            let Some(c) = found else {
                out.failed.push(format!(
                    "{}.{} ({}): no room for a via beside the pad",
                    part.reference, pad.number, layout.nets[net].name
                ));
                continue;
            };
            let c = c.map(|v| (v * 1e4).round() / 1e4);
            let name = layout.nets[net].name.clone();
            let own_width = (width < layout.nets[net].width - 1e-9).then_some(width);
            out.tracks.push(RoutedTrack {
                net: name.clone(),
                layer: layer.clone(),
                width: own_width,
                points: vec![pc, c],
            });
            out.vias.push(RoutedVia { net: name, at: c, via: spec.name.clone() });
            items.push(Item {
                net: Some(net),
                layers: vlayers.clone(),
                shape: Shape::Circle(c, vr),
                smd: false,
            });
            items.push(Item {
                net: Some(net),
                layers: vec![layer.clone()],
                shape: Shape::Seg(pc, c, width / 2.0),
                smd: false,
            });
            holes.push(Hole { at: c, r: dr });
            placed.push((net, c));
            out.tied += 1;
        }
    }
    Ok(out)
}

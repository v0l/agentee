use crate::board::Board;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::{Layout, ZoneFill, glob};
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

pub fn plane_nets(layout: &Layout) -> Vec<usize> {
    let mut v: Vec<usize> = layout.zones.iter().map(|z| z.net).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn is_bga(fp_name: &str, pads: usize) -> bool {
    fp_name.to_ascii_lowercase().contains("bga") || pads >= 64
}

fn islands(z: &ZoneFill) -> Vec<u32> {
    let (w, h) = (z.width, z.height);
    let mut label = vec![0u32; w * h];
    let mut next = 0;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if z.mask[start] == 0 || label[start] != 0 {
            continue;
        }
        next += 1;
        label[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let around = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
            ];
            for j in around.into_iter().flatten() {
                if z.mask[j] != 0 && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            }
        }
    }
    label
}

fn filled_cells(z: &ZoneFill, b: Bounds, inside: impl Fn(P) -> bool) -> Vec<usize> {
    let mut out = Vec::new();
    if b.is_empty() {
        return out;
    }
    let cell = |v: f64, o: f64| ((v - o) / z.cell).floor();
    let (x0, y0) = (cell(b.min[0], z.origin[0]).max(0.0), cell(b.min[1], z.origin[1]).max(0.0));
    let (x1, y1) = (cell(b.max[0], z.origin[0]), cell(b.max[1], z.origin[1]));
    if x1 < 0.0 || y1 < 0.0 {
        return out;
    }
    let x1 = (x1 as usize).min(z.width.saturating_sub(1));
    let y1 = (y1 as usize).min(z.height.saturating_sub(1));
    for y in y0 as usize..=y1 {
        for x in x0 as usize..=x1 {
            let i = y * z.width + x;
            let c =
                [z.origin[0] + (x as f64 + 0.5) * z.cell, z.origin[1] + (y as f64 + 0.5) * z.cell];
            if z.mask[i] != 0 && inside(c) {
                out.push(i);
            }
        }
    }
    out
}

fn on_pad(z: &ZoneFill, outlines: &[Vec<P>]) -> Vec<usize> {
    let mut b = Bounds::EMPTY;
    outlines.iter().flatten().for_each(|q| b.add(*q));
    filled_cells(z, b, |c| outlines.iter().any(|o| geom::point_in_polygon(c, o)))
}

struct Joins {
    root: Vec<usize>,
}

impl Joins {
    fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.root[r] != r {
            r = self.root[r];
        }
        self.root[i] = r;
        r
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.root[a] = b;
    }
}

fn track_gap(points: &[P], c: P) -> f64 {
    points.windows(2).map(|w| geom::point_segment_distance(c, w[0], w[1])).fold(f64::MAX, f64::min)
}

fn reaches_plane(
    layout: &Layout,
    net: usize,
    labels: &mut std::collections::HashMap<usize, Vec<u32>>,
) -> std::collections::HashSet<(usize, usize)> {
    let pads: Vec<(usize, usize)> = layout
        .parts
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| p.pads.iter().enumerate().map(move |(qi, q)| (pi, qi, q)))
        .filter(|(_, _, q)| q.net == Some(net) && q.kind != PadKind::Npth)
        .map(|(pi, qi, _)| (pi, qi))
        .collect();
    let pad = |k: usize| &layout.parts[pads[k].0].pads[pads[k].1];
    let vias: Vec<&crate::layout::Via> = layout.vias.iter().filter(|v| v.net == net).collect();
    let tracks: Vec<&crate::layout::Track> =
        layout.tracks.iter().filter(|t| t.net == net).collect();
    let zones: Vec<usize> =
        (0..layout.zones.len()).filter(|&zi| layout.zones[zi].net == net).collect();
    let mut pieces: Vec<(usize, u32)> = Vec::new();
    let mut main: Vec<(usize, u32)> = Vec::new();
    for &zi in &zones {
        let label = labels.entry(zi).or_insert_with(|| islands(&layout.zones[zi]));
        let mut size: std::collections::BTreeMap<u32, usize> = Default::default();
        for &l in label.iter().filter(|&&l| l != 0) {
            *size.entry(l).or_default() += 1;
        }
        if let Some((&l, _)) = size.iter().max_by_key(|(l, n)| (**n, std::cmp::Reverse(**l))) {
            main.push((zi, l));
        }
        pieces.extend(size.into_keys().map(|l| (zi, l)));
    }
    let (np, nv, nt) = (pads.len(), vias.len(), tracks.len());
    let mut j = Joins { root: (0..np + nv + nt + pieces.len()).collect() };
    let island_at = |zi: usize, l: u32| pieces.iter().position(|&x| x == (zi, l));
    for &zi in &zones {
        let z = &layout.zones[zi];
        let label = &labels[&zi];
        let touch = |node: usize, cells: Vec<usize>, j: &mut Joins| {
            for c in cells {
                if let Some(ii) = island_at(zi, label[c]) {
                    j.join(node, np + nv + nt + ii);
                }
            }
        };
        for k in 0..np {
            let q = pad(k);
            if q.copper.contains(&z.layer) {
                touch(k, on_pad(z, &q.outlines), &mut j);
            }
        }
        for (k, v) in vias.iter().enumerate() {
            if v.layers.contains(&z.layer) {
                let r = v.diameter / 2.0;
                let b = Bounds { min: [v.at[0] - r, v.at[1] - r], max: [v.at[0] + r, v.at[1] + r] };
                touch(np + k, filled_cells(z, b, |c| geom::dist(c, v.at) <= r), &mut j);
            }
        }
        for (k, tr) in tracks.iter().enumerate() {
            if tr.layer == z.layer {
                let mut b = Bounds::EMPTY;
                tr.points.iter().for_each(|q| b.add(*q));
                let h = tr.width / 2.0;
                let b =
                    Bounds { min: [b.min[0] - h, b.min[1] - h], max: [b.max[0] + h, b.max[1] + h] };
                touch(np + nv + k, filled_cells(z, b, |c| track_gap(&tr.points, c) <= h), &mut j);
            }
        }
    }
    let pad_box = |k: usize| {
        let mut b = Bounds::EMPTY;
        pad(k).outlines.iter().flatten().for_each(|q| b.add(*q));
        b
    };
    let boxes: Vec<Bounds> = (0..np).map(pad_box).collect();
    for a in 0..np {
        for b in a + 1..np {
            let (qa, qb) = (pad(a), pad(b));
            let (ba, bb) = (&boxes[a], &boxes[b]);
            let apart = ba.max[0] < bb.min[0]
                || bb.max[0] < ba.min[0]
                || ba.max[1] < bb.min[1]
                || bb.max[1] < ba.min[1];
            if !apart
                && qa.copper.iter().any(|l| qb.copper.contains(l))
                && qa
                    .outlines
                    .iter()
                    .any(|x| qb.outlines.iter().any(|y| geom::polygon_distance(x, y) <= 1e-6))
            {
                j.join(a, b);
            }
        }
    }
    for (k, v) in vias.iter().enumerate() {
        for a in 0..np {
            let q = pad(a);
            if q.copper.iter().any(|l| v.layers.contains(l))
                && crate::drc::rings_point_gap(&q.outlines, v.at) <= v.diameter / 2.0
            {
                j.join(a, np + k);
            }
        }
    }
    for (k, tr) in tracks.iter().enumerate() {
        let me = np + nv + k;
        let ends = [tr.points[0], *tr.points.last().unwrap_or(&tr.points[0])];
        let h = tr.width / 2.0;
        for a in 0..np {
            let q = pad(a);
            if q.copper.contains(&tr.layer)
                && ends.iter().any(|&e| crate::drc::rings_point_gap(&q.outlines, e) <= h)
            {
                j.join(a, me);
            }
        }
        for (vk, v) in vias.iter().enumerate() {
            if v.layers.contains(&tr.layer) && track_gap(&tr.points, v.at) <= h + v.diameter / 2.0 {
                j.join(np + vk, me);
            }
        }
        for (o, other) in tracks.iter().enumerate().skip(k + 1) {
            if other.layer == tr.layer
                && (ends.iter().any(|&e| track_gap(&other.points, e) <= h + other.width / 2.0)
                    || [other.points[0], *other.points.last().unwrap_or(&other.points[0])]
                        .iter()
                        .any(|&e| track_gap(&tr.points, e) <= h + other.width / 2.0))
            {
                j.join(np + nv + o, me);
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    for k in 0..np {
        let q = pad(k);
        let own = q.copper.first().cloned().unwrap_or_default();
        let r = j.find(k);
        let elsewhere = pieces.iter().enumerate().any(|(ii, &(zi, l))| {
            layout.zones[zi].layer != own
                && main.contains(&(zi, l))
                && j.find(np + nv + nt + ii) == r
        });
        if elsewhere {
            out.insert(pads[k]);
        }
    }
    out
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
    let hole_smd = rules.min_hole_to_smd_pad.to_mm();
    let copper = &layout.copper;

    let stitched =
        |v: &&crate::layout::Via| matches!(v.source, crate::layout::ViaSource::Stitch(_));
    let mut placed: Vec<(usize, P)> = layout
        .vias
        .iter()
        .filter(|v| !stitched(v) && planes.contains(&v.net))
        .map(|v| (v.net, v.at))
        .collect();
    let fixed: Vec<crate::layout::Via> =
        layout.vias.iter().filter(|v| !stitched(v)).cloned().collect();
    let world = crate::drc::Ctx::new(
        board,
        &layout.copper,
        &layout.outline,
        &layout.board_cutouts,
        &layout.parts,
        &layout.tracks,
        &fixed,
        &[],
        &layout.nets,
    );
    let base = crate::rules::Placed::new(&world);
    let mut kept = crate::rules::Plan::default();

    let mut out = TieResult::default();
    let mut labels: std::collections::HashMap<usize, Vec<u32>> = std::collections::HashMap::new();
    let mut reached: std::collections::HashMap<usize, std::collections::HashSet<(usize, usize)>> =
        std::collections::HashMap::new();
    let wants_close: Vec<bool> = layout
        .parts
        .iter()
        .map(|p| {
            let on = |pick: &dyn Fn(&crate::layout::LayoutNet) -> bool| {
                p.pads.iter().filter_map(|q| q.net).any(|n| pick(&layout.nets[n]))
            };
            let decap = crate::place::is_capacitor(&p.reference, &p.footprint_name)
                && on(&|n| {
                    !crate::place::is_ground(&n.name)
                        && crate::place::is_power_net(board, &n.name, &n.class)
                });
            decap || on(&|n| crate::place::is_rf_class(board, &n.class))
        })
        .collect();
    for (pi, part) in layout.parts.iter().enumerate() {
        if is_bga(&part.footprint_name, part.pads.len()) {
            continue;
        }
        let mut body = Bounds::EMPTY;
        part.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| body.add(*q));
        let centre = body.center();
        for (qi, pad) in part.pads.iter().enumerate() {
            let Some(net) = pad.net else { continue };
            if !planes.contains(&net) || pad.drill.is_some() || pad.kind == PadKind::Npth {
                continue;
            }
            let Some(layer) = pad.copper.first().cloned() else { continue };
            let mut pb = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| pb.add(*q));
            let pc = pb.center();
            let reach = pb.size()[0].max(pb.size()[1]) / 2.0 + NEAR_VIA;
            let near_hole = layout.parts.iter().flat_map(|p| &p.pads).any(|q| {
                q.net == Some(net)
                    && q.drill.is_some()
                    && q.kind != PadKind::Npth
                    && crate::drc::rings_point_gap(&q.outlines, pc) <= reach
            });
            let joined = if wants_close[pi] {
                near_hole
            } else {
                reached
                    .entry(net)
                    .or_insert_with(|| reaches_plane(layout, net, &mut labels))
                    .contains(&(pi, qi))
            };
            if joined {
                out.already += 1;
                continue;
            }
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
            let (vr, dr) = (spec.diameter.to_mm() / 2.0, spec.drill.to_mm() / 2.0);
            let width = layout.nets[net].width.min(pb.size()[0].min(pb.size()[1]));
            let stub = |c: P| crate::layout::Track {
                source: usize::MAX,
                net,
                layer: layer.clone(),
                width,
                points: vec![pc, c],
            };
            let follows_rules = |c: P| {
                let via = crate::layout::Via::of(spec, net, c, copper);
                let plan = crate::rules::Planned::after(
                    &base,
                    &kept,
                    crate::rules::Plan {
                        tracks: vec![stub(c)],
                        vias: vec![via],
                        ..Default::default()
                    },
                );
                crate::rules::legal(&plan).is_ok()
            };
            let off_own = |c: P| {
                crate::drc::rings_point_gap(&pad.outlines, c) >= (vr + PAD_GAP).max(dr + hole_smd)
            };
            let vlayers = spec.copper_layers(copper);
            let in_a_pad = |c: P| {
                layout.parts.iter().flat_map(|p| &p.pads).any(|q| {
                    crate::drc::is_smd(q)
                        && q.copper.iter().any(|l| vlayers.contains(l))
                        && crate::drc::rings_point_gap(&q.outlines, c) < vr + PAD_GAP
                })
            };
            let legal = |c: P| -> bool { !in_a_pad(c) && follows_rules(c) };
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
            kept.tracks.push(stub(c));
            kept.vias.push(crate::layout::Via::of(spec, net, c, copper));
            placed.push((net, c));
            out.tied += 1;
        }
    }
    Ok(out)
}

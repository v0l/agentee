use crate::flow::Flow;
use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::footprint::PadKind;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::place::{self, Role};
use serde::Serialize;

pub struct Escape;

#[derive(Clone, Debug, Serialize)]
pub struct EscapeTrack {
    pub net: usize,
    pub layer: String,
    pub width: f64,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EscapeVia {
    pub net: usize,
    pub at: P,
    pub via: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EscapePlan {
    pub tracks: Vec<EscapeTrack>,
    pub vias: Vec<EscapeVia>,
    pub per_part: Vec<PartEscape>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PartEscape {
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub pitch: f64,
    pub balls: usize,
    pub signals: usize,
    pub plane: usize,
    pub per_layer: Vec<(String, usize)>,
    pub failed: Vec<String>,
}

struct Ball {
    pad: usize,
    net: usize,
    cell: (i64, i64),
    at: P,
    plane: bool,
}

impl Phase for Escape {
    fn name(&self) -> &'static str {
        "escape"
    }

    fn run(
        &self,
        model: &mut Model,
        cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "escape".into(), ..Default::default() };
        let layers: Vec<String> = cfg
            .escape
            .as_ref()
            .map(|e| e.layers.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| signal_layers(model));
        let mut plan = EscapePlan::default();
        let prefer: std::collections::HashMap<usize, String> = model
            .layers
            .as_ref()
            .map(|lp| {
                lp.groups
                    .iter()
                    .flat_map(|g| {
                        g.nets.iter().filter_map(|n| {
                            model
                                .layout
                                .nets
                                .iter()
                                .position(|x| x.name == *n)
                                .map(|i| (i, g.layer.clone()))
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        for i in 0..model.layout.parts.len() {
            let p = &model.layout.parts[i];
            if place::role_of(&p.reference, &p.footprint_name, &p.footprint) != Role::Chip {
                continue;
            }
            if !is_bga(p) {
                continue;
            }
            let in_pad = cfg.escape.as_ref().and_then(|e| e.via_in_pad).unwrap_or(false);
            let signals = cfg.escape.as_ref().and_then(|e| e.signals).unwrap_or(false);
            let pe = escape_part(model, i, &layers, in_pad, signals, &prefer, &mut plan);
            if !pe.failed.is_empty() {
                report.failed.extend(pe.failed.iter().map(|f| format!("{}: {f}", pe.reference)));
            }
            let how = if signals {
                format!("{} signal balls escaped per layer {:?}", pe.signals, pe.per_layer)
            } else {
                format!("{} signal balls left to routing", pe.signals)
            };
            report.notes.push(format!(
                "{}: pitch {:.2}, {} plane balls fanned out, {how}{}",
                pe.reference,
                pe.pitch,
                pe.plane,
                pe.note.as_ref().map(|n| format!("; {n}")).unwrap_or_default()
            ));
            plan.per_part.push(pe);
        }
        report.changed = !plan.tracks.is_empty() || !plan.vias.is_empty();
        model.escape = Some(plan);
        report
    }
}

fn signal_layers(model: &Model) -> Vec<String> {
    let copper = &model.layout.copper;
    let plane: Vec<String> = model
        .file
        .zones
        .iter()
        .filter(|z| z.outline.as_ref().is_none_or(|o| o.is_empty()))
        .filter(|z| {
            model
                .layout
                .nets
                .iter()
                .find(|n| n.name == z.net)
                .is_some_and(|n| place::is_power_net(model.board, &n.name, &n.class))
        })
        .flat_map(|z| z.layers.clone())
        .collect();
    copper
        .iter()
        .enumerate()
        .filter(|(i, l)| *i == 0 || *i + 1 == copper.len() || !plane.contains(l))
        .map(|(_, l)| l.clone())
        .collect()
}

fn escape_part(
    model: &Model,
    part: usize,
    layers: &[String],
    in_pad: bool,
    signals: bool,
    prefer: &std::collections::HashMap<usize, String>,
    plan: &mut EscapePlan,
) -> PartEscape {
    let l = &model.layout;
    let p = &l.parts[part];
    let mut pe = PartEscape { reference: p.reference.clone(), ..Default::default() };
    let centres: Vec<(usize, P, f64)> = p
        .pads
        .iter()
        .enumerate()
        .filter(|(_, q)| q.kind == PadKind::Smd)
        .map(|(i, q)| {
            let mut b = Bounds::EMPTY;
            q.outlines.iter().flatten().for_each(|r| b.add(*r));
            let [w, h] = b.size();
            (i, b.center(), w.min(h))
        })
        .collect();
    if centres.len() < 4 {
        return pe;
    }
    let pitch = pitch_of(p);
    pe.pitch = pitch;
    let pad = centres.iter().map(|c| c.2).fold(0.0, f64::max);
    let mut gb = Bounds::EMPTY;
    centres.iter().for_each(|c| gb.add(c.1));
    let half = pitch / 2.0;
    let margin = 3i64;
    let origin = [gb.min[0] - margin as f64 * half, gb.min[1] - margin as f64 * half];
    let nx = ((gb.max[0] - gb.min[0]) / half).round() as i64 + 2 * margin + 1;
    let ny = ((gb.max[1] - gb.min[1]) / half).round() as i64 + 2 * margin + 1;
    let cell_of = |q: P| {
        (((q[0] - origin[0]) / half).round() as i64, ((q[1] - origin[1]) / half).round() as i64)
    };
    let at_of = |c: (i64, i64)| [origin[0] + c.0 as f64 * half, origin[1] + c.1 as f64 * half];

    let mut balls: Vec<Ball> = Vec::new();
    for (i, c, _) in &centres {
        let Some(net) = p.pads[*i].net else { continue };
        let n = &l.nets[net];
        let plane = place::is_power_net(model.board, &n.name, &n.class);
        balls.push(Ball { pad: *i, net, cell: cell_of(*c), at: *c, plane });
    }
    pe.balls = balls.len();
    pe.plane = balls.iter().filter(|b| b.plane).count();
    pe.signals = balls.len() - pe.plane;

    let via_name = model
        .board
        .vias
        .iter()
        .min_by(|a, b| a.diameter.to_mm().partial_cmp(&b.diameter.to_mm()).unwrap())
        .map(|v| v.name.clone())
        .unwrap_or_else(|| "std".into());
    let via_d = model
        .board
        .vias
        .iter()
        .find(|v| v.name == via_name)
        .map(|v| v.diameter.to_mm())
        .unwrap_or(0.35);

    let min_w = model.board.rules.min_track_width.to_mm();
    let min_c = model.board.rules.min_clearance.to_mm();
    let class_allows = |net: usize, layer: &str| -> bool {
        let n = &l.nets[net];
        model
            .board
            .netclasses
            .iter()
            .find(|c| c.name == n.class)
            .is_none_or(|c| c.layers.is_empty() || c.layers.iter().any(|x| x == layer))
    };
    let neck_of = |net: usize| -> (f64, f64) {
        let n = &l.nets[net];
        (n.width.min(min_w), n.clearance.min(min_c))
    };
    let all_cells: Vec<(i64, i64)> = balls.iter().map(|b| b.cell).collect();
    let pad_cells: Vec<(i64, i64)> = centres.iter().map(|c| cell_of(c.1)).collect();
    let stub_room = half * (2.0f64).sqrt() / 2.0 - pad / 2.0;
    let via_room = pitch * (2.0f64).sqrt() / 2.0 - pad / 2.0 - via_d / 2.0;
    let dog_bone_fits = stub_room >= min_w / 2.0 + min_c && via_room >= min_c;
    let in_pad = in_pad || !dog_bone_fits;
    if !dog_bone_fits {
        pe.note = Some(format!(
            "dog-bone does not fit at {pitch:.2} mm pitch ({:.2} mm stub room, {:.2} mm via room), via in pad",
            stub_room, via_room
        ));
    }
    let mut via_cells: Vec<(i64, i64)> = Vec::new();
    let mut stub_cells: Vec<(i64, i64)> = Vec::new();
    let site_of = |cell: (i64, i64),
                   taken: &[&Vec<(i64, i64)>],
                   top_taken: &[&Vec<(i64, i64)>]|
     -> Option<(i64, i64)> {
        if in_pad {
            return Some(cell);
        }
        [(1, 1), (-1, 1), (1, -1), (-1, -1)].iter().map(|(dx, dy)| (cell.0 + dx, cell.1 + dy)).find(
            |c| {
                let stub = [(cell.0, c.1), (c.0, cell.1)];
                c.0 >= 0
                    && c.1 >= 0
                    && c.0 < nx
                    && c.1 < ny
                    && !taken.iter().any(|t| t.contains(c))
                    && !pad_cells.contains(c)
                    && !top_taken
                        .iter()
                        .any(|t| t.contains(c) || stub.iter().any(|q| t.contains(q)))
            },
        )
    };
    let place_at = |b: &Ball,
                    site: (i64, i64),
                    via_cells: &mut Vec<(i64, i64)>,
                    stub_cells: &mut Vec<(i64, i64)>,
                    plan: &mut EscapePlan| {
        let at = if site == b.cell { b.at } else { at_of(site) };
        if !l.vias.iter().any(|v| geom::dist(v.at, at) < 1e-3)
            && !plan.vias.iter().any(|v| geom::dist(v.at, at) < 1e-3)
        {
            plan.vias.push(EscapeVia { net: b.net, at, via: via_name.clone() });
        }
        if site != b.cell {
            let (w, _) = neck_of(b.net);
            plan.tracks.push(EscapeTrack {
                net: b.net,
                layer: l.copper[0].clone(),
                width: w,
                points: vec![b.at, at],
            });
            stub_cells.push(site);
            stub_cells.push((b.cell.0, site.1));
            stub_cells.push((site.0, b.cell.1));
        }
        if !via_cells.contains(&site) {
            via_cells.push(site);
        }
    };
    let mut top_used: Vec<(i64, i64)> = Vec::new();
    let mut no_site = Vec::new();
    let signal_cells: Vec<(i64, i64)> = balls.iter().filter(|b| !b.plane).map(|b| b.cell).collect();
    let mut plane_vias: Vec<((i64, i64), usize)> = Vec::new();
    let mut plane_balls: Vec<&Ball> = balls.iter().filter(|b| b.plane).collect();
    plane_balls.sort_by_key(|b| {
        let e = (b.cell.0 - margin)
            .min(nx - 1 - margin - b.cell.0)
            .min(b.cell.1 - margin)
            .min(ny - 1 - margin - b.cell.1);
        std::cmp::Reverse(e)
    });
    for b in plane_balls {
        if in_pad {
            place_at(b, b.cell, &mut via_cells, &mut stub_cells, plan);
            continue;
        }
        let mut best: Option<((i64, i64), usize)> = None;
        for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let c = (b.cell.0 + dx, b.cell.1 + dy);
            if c.0 < 0 || c.1 < 0 || c.0 >= nx || c.1 >= ny || pad_cells.contains(&c) {
                continue;
            }
            let shared = plane_vias.iter().any(|&(q, n)| q == c && n == b.net);
            if !shared && (via_cells.contains(&c) || stub_cells.contains(&c)) {
                continue;
            }
            let crowd =
                [(c.0 - 1, c.1 - 1), (c.0 + 1, c.1 - 1), (c.0 - 1, c.1 + 1), (c.0 + 1, c.1 + 1)]
                    .iter()
                    .filter(|q| signal_cells.contains(q))
                    .count();
            let rank = if shared { 0 } else { 1 + crowd };
            if best.is_none_or(|(_, k)| rank < k) {
                best = Some((c, rank));
            }
        }
        match best {
            Some((site, _)) => {
                place_at(b, site, &mut via_cells, &mut stub_cells, plan);
                plane_vias.push((site, b.net));
            }
            None => no_site.push(b.pad),
        }
    }
    if !signals {
        for pad in no_site {
            pe.failed.push(format!(
                "{} ({}): no free spot for a dog-bone via",
                p.pads[pad].number,
                l.nets[p.pads[pad].net.unwrap_or(0)].name
            ));
        }
        return pe;
    }
    let mut pre_site: std::collections::HashMap<usize, (i64, i64)> =
        std::collections::HashMap::new();
    if !in_pad && layers.first() == l.copper.first() {
        for (bi, b) in balls.iter().enumerate() {
            let inner = prefer.get(&b.net).is_some_and(|ly| ly != &layers[0]);
            if b.plane || (class_allows(b.net, &layers[0]) && !inner) {
                continue;
            }
            if let Some(c) = site_of(b.cell, &[&via_cells], &[&stub_cells]) {
                place_at(b, c, &mut via_cells, &mut stub_cells, plan);
                pre_site.insert(bi, c);
            }
        }
    }
    let mut pending: Vec<usize> = (0..balls.len()).filter(|&i| !balls[i].plane).collect();
    let target_side: Vec<Option<usize>> = balls
        .iter()
        .map(|b| {
            let others: Vec<P> = l
                .parts
                .iter()
                .enumerate()
                .filter(|(pi, _)| *pi != part)
                .flat_map(|(_, q)| q.pads.iter().filter(|pd| pd.net == Some(b.net)))
                .map(|pd| {
                    let mut bb = Bounds::EMPTY;
                    pd.outlines.iter().flatten().for_each(|r| bb.add(*r));
                    bb.center()
                })
                .collect();
            if others.is_empty() {
                return None;
            }
            let n = others.len() as f64;
            let cx = others.iter().map(|o| o[0]).sum::<f64>() / n;
            let cy = others.iter().map(|o| o[1]).sum::<f64>() / n;
            let (dx, dy) = (cx - p.at.to_mm()[0], cy - p.at.to_mm()[1]);
            Some(if dx.abs() > dy.abs() {
                if dx > 0.0 { 1 } else { 3 }
            } else if dy > 0.0 {
                2
            } else {
                0
            })
        })
        .collect();
    let idx = |x: i64, y: i64| (y * nx + x) as usize;
    let n_cells = (nx * ny) as usize;
    let src = 2 * n_cells;
    let sink = src + 1;
    let side_of = |x: i64, y: i64| -> Option<usize> {
        if y == 0 {
            Some(0)
        } else if x == nx - 1 {
            Some(1)
        } else if y == ny - 1 {
            Some(2)
        } else if x == 0 {
            Some(3)
        } else {
            None
        }
    };
    let mut used_of: Vec<Vec<(i64, i64)>> = vec![Vec::new(); layers.len()];
    let mut count_of: Vec<usize> = vec![0; layers.len()];
    let layer_index = |ly: &str| layers.iter().position(|x| x == ly);
    for round in 0..2 {
        for (li, layer) in layers.iter().enumerate() {
            if pending.is_empty() {
                break;
            }
            let waiting = |net: usize| {
                round == 0
                    && prefer.get(&net).and_then(|ly| layer_index(ly)).is_some_and(|k| k > li)
            };
            let top = li == 0 && layer == &l.copper[0];
            let obstacle = if top { pad } else { via_d };
            let gap = pitch - obstacle;
            let fits = |net: usize| -> bool {
                let (w, c) = neck_of(net);
                gap >= w + 2.0 * c
            };
            let widest = pending
                .iter()
                .map(|&i| {
                    let (w, c) = neck_of(balls[i].net);
                    w / 2.0 + c
                })
                .fold(0.0, f64::max);
            let diagonals = half * (5.0f64).sqrt() / 2.0 - obstacle / 2.0 >= widest;
            let on_edge = |c: (i64, i64)| {
                c.0 == margin || c.1 == margin || c.0 == nx - 1 - margin || c.1 == ny - 1 - margin
            };
            let mut layer_count = 0usize;
            let mut used: Vec<(i64, i64)> = std::mem::take(&mut used_of[li]);
            let sides = [(0u8, Some(0)), (0, Some(1)), (0, Some(2)), (0, Some(3))];
            let plain: Vec<Vec<(u8, Option<usize>)>> = vec![
                sides.iter().copied().chain([(0, None)]).collect(),
                [(1, None)].into_iter().chain(sides.iter().copied()).chain([(0, None)]).collect(),
                vec![(0, None)],
            ];
            let attempts: Vec<Vec<(u8, Option<usize>)>> = plain
                .iter()
                .cloned()
                .chain(
                    plain
                        .iter()
                        .map(|a| [(2, None)].into_iter().chain(a.iter().copied()).collect()),
                )
                .collect();
            let last_chance = |net: usize| layers[li + 1..].iter().all(|ly| !class_allows(net, ly));
            let pre = (
                used.clone(),
                plan.tracks.len(),
                plan.vias.len(),
                via_cells.clone(),
                stub_cells.clone(),
                pending.clone(),
            );
            let mut best: Option<(i64, usize, _)> = None;
            for (ai, passes) in attempts.iter().enumerate() {
                if ai > 0 {
                    used = pre.0.clone();
                    plan.tracks.truncate(pre.1);
                    plan.vias.truncate(pre.2);
                    via_cells = pre.3.clone();
                    stub_cells = pre.4.clone();
                    pending = pre.5.clone();
                    layer_count = 0;
                }
                for &(kind, pass) in passes {
                    let here: Vec<usize> = pending
                        .iter()
                        .copied()
                        .filter(|&i| fits(balls[i].net) || on_edge(balls[i].cell))
                        .filter(|&i| kind != 1 || on_edge(balls[i].cell))
                        .filter(|&i| kind != 2 || last_chance(balls[i].net))
                        .filter(|&i| kind != 0 || pass.is_none() || target_side[i] == pass)
                        .filter(|&i| !waiting(balls[i].net))
                        .filter(|&i| class_allows(balls[i].net, layer))
                        .collect();
                    if here.is_empty() {
                        continue;
                    }
                    let mut sites: Vec<(usize, (i64, i64))> = Vec::new();
                    let here: Vec<usize> = if top {
                        here
                    } else {
                        let mut reserved = via_cells.clone();
                        here.into_iter()
                            .filter(|&bi| {
                                if let Some(&c) = pre_site.get(&bi) {
                                    sites.push((bi, c));
                                    return true;
                                }
                                let s = site_of(
                                    balls[bi].cell,
                                    &[&reserved, &used],
                                    &[&stub_cells, &top_used],
                                );
                                if let Some(c) = s {
                                    sites.push((bi, c));
                                    reserved.push(c);
                                }
                                s.is_some()
                            })
                            .collect()
                    };
                    let mut blocked: Vec<(i64, i64)> =
                        if top { pad_cells.clone() } else { via_cells.clone() };
                    if top {
                        blocked.extend(stub_cells.iter().copied());
                    } else {
                        blocked.extend(sites.iter().map(|(_, c)| *c));
                    }
                    if here.is_empty() {
                        continue;
                    }
                    let src_cell = |bi: usize| -> (i64, i64) {
                        sites
                            .iter()
                            .find(|(b, _)| *b == bi)
                            .map(|(_, c)| *c)
                            .unwrap_or(balls[bi].cell)
                    };
                    let mut f = Flow::new(sink + 1);
                    for y in 0..ny {
                        for x in 0..nx {
                            let c = (x, y);
                            let i = idx(x, y);
                            let side = side_of(x, y);
                            let cap = if blocked.contains(&c)
                                || ((top || in_pad) && all_cells.contains(&c))
                                || used.contains(&c)
                            {
                                0
                            } else {
                                1
                            };
                            f.add(i, i + n_cells, cap);
                            if let Some(sd) = side {
                                let cost = match pass {
                                    None => 0,
                                    Some(want) if want == sd => 0,
                                    Some(want) if (want + 2) % 4 == sd => 4 * (nx + ny),
                                    _ => nx + ny,
                                };
                                f.add_cost(i + n_cells, sink, 100, cost);
                            }
                            let crossing =
                                |c: (i64, i64)| (c.0 - margin) % 2 != 0 && (c.1 - margin) % 2 != 0;
                            let gap_cell = |c: (i64, i64)| {
                                ((c.0 - margin) % 2 != 0) != ((c.1 - margin) % 2 != 0)
                            };
                            for (dx, dy) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                                let (x2, y2) = (x + dx, y + dy);
                                if x2 < 0 || y2 < 0 || x2 >= nx || y2 >= ny {
                                    continue;
                                }
                                let d = (x2, y2);
                                if dx != 0 && dy != 0 {
                                    let ok = diagonals
                                        && ((crossing(c) && gap_cell(d))
                                            || (gap_cell(c) && crossing(d)));
                                    if !ok {
                                        continue;
                                    }
                                }
                                let j = idx(x2, y2);
                                f.add_cost(i + n_cells, j, 1, 1);
                                f.add_cost(j + n_cells, i, 1, 1);
                            }
                        }
                    }
                    for &bi in &here {
                        let (x, y) = src_cell(bi);
                        f.add(src, idx(x, y) + n_cells, 1);
                    }
                    f.min_cost_flow(src, sink);
                    let mut escaped = Vec::new();
                    for &bi in &here {
                        let (x, y) = src_cell(bi);
                        let Some(path) = f.take_path(idx(x, y) + n_cells, sink) else { continue };
                        let cells: Vec<usize> = path
                            .iter()
                            .filter(|&&n| n < 2 * n_cells)
                            .map(|&n| n % n_cells)
                            .collect();
                        for &n in &cells {
                            used.push(((n as i64) % nx, (n as i64) / nx));
                        }
                        let mut pts: Vec<P> = cells
                            .iter()
                            .copied()
                            .scan(usize::MAX, |last, n| {
                                if *last == n {
                                    Some(None)
                                } else {
                                    *last = n;
                                    Some(Some(n))
                                }
                            })
                            .flatten()
                            .map(|n| at_of(((n as i64) % nx, (n as i64) / nx)))
                            .collect();
                        if top {
                            if let Some(first) = pts.first_mut() {
                                *first = balls[bi].at;
                            }
                        } else if in_pad {
                            if let Some(first) = pts.first_mut() {
                                *first = balls[bi].at;
                            }
                        }
                        let pts = straighten(pts);
                        let (w, _) = neck_of(balls[bi].net);
                        plan.tracks.push(EscapeTrack {
                            net: balls[bi].net,
                            layer: layer.clone(),
                            width: w,
                            points: pts,
                        });
                        if !top {
                            if !pre_site.contains_key(&bi) {
                                place_at(
                                    &balls[bi],
                                    src_cell(bi),
                                    &mut via_cells,
                                    &mut stub_cells,
                                    plan,
                                );
                            }
                        }
                        escaped.push(bi);
                    }
                    layer_count += escaped.len();
                    pending.retain(|b| !escaped.contains(b));
                }
                let stuck = pending
                    .iter()
                    .filter(|&&i| class_allows(balls[i].net, layer) && last_chance(balls[i].net))
                    .count();
                let merit = layer_count as i64 - 4 * stuck as i64;
                if best.as_ref().is_none_or(|b| merit > b.0) {
                    best = Some((
                        merit,
                        ai,
                        (
                            used.clone(),
                            plan.tracks[pre.1..].to_vec(),
                            plan.vias[pre.2..].to_vec(),
                            via_cells.clone(),
                            stub_cells.clone(),
                            pending.clone(),
                        ),
                    ));
                }
            }
            if let Some((_, _, st)) = best {
                layer_count = pre.5.len() - st.5.len();
                used = st.0;
                plan.tracks.truncate(pre.1);
                plan.tracks.extend(st.1);
                plan.vias.truncate(pre.2);
                plan.vias.extend(st.2);
                via_cells = st.3;
                stub_cells = st.4;
                pending = st.5;
            }
            if top {
                top_used = used.clone();
            }
            used_of[li] = used;
            count_of[li] += layer_count;
        }
    }
    for (li, layer) in layers.iter().enumerate() {
        pe.per_layer.push((layer.clone(), count_of[li]));
    }
    for pad in no_site {
        pe.failed.push(format!(
            "{} ({}): no free spot for a dog-bone via",
            p.pads[pad].number,
            l.nets[p.pads[pad].net.unwrap_or(0)].name
        ));
    }
    for &bi in &pending {
        let (w, c) = neck_of(balls[bi].net);
        pe.failed.push(format!(
            "{} ({}): no escape on {}, {:.2} mm track with {:.2} mm clearance",
            p.pads[balls[bi].pad].number,
            l.nets[balls[bi].net].name,
            layers.join("/"),
            w,
            c
        ));
    }
    pe
}

pub fn is_bga(p: &agentee_core::layout::Placed) -> bool {
    let smd: Vec<f64> = p
        .pads
        .iter()
        .filter(|q| q.kind == PadKind::Smd && q.net.is_some())
        .map(|q| {
            let mut b = Bounds::EMPTY;
            q.outlines.iter().flatten().for_each(|r| b.add(*r));
            let [w, h] = b.size();
            (w - h).abs()
        })
        .collect();
    smd.len() >= 36 && smd.iter().all(|d| *d < 0.05)
}

pub fn pitch_of(p: &agentee_core::layout::Placed) -> f64 {
    let centres: Vec<P> = p
        .pads
        .iter()
        .filter(|q| q.kind == PadKind::Smd)
        .map(|q| {
            let mut b = Bounds::EMPTY;
            q.outlines.iter().flatten().for_each(|r| b.add(*r));
            b.center()
        })
        .collect();
    let mut pitch = f64::MAX;
    for i in 0..centres.len() {
        for j in i + 1..centres.len() {
            let d = geom::dist(centres[i], centres[j]);
            if d > 1e-3 && d < pitch {
                pitch = d;
            }
        }
    }
    pitch
}

fn straighten(pts: Vec<P>) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for q in pts {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b[0] - a[0]) * (q[1] - b[1]) - (b[1] - a[1]) * (q[0] - b[0]);
            let dot = (b[0] - a[0]) * (q[0] - b[0]) + (b[1] - a[1]) * (q[1] - b[1]);
            if cross.abs() < 1e-9 && dot > 0.0 {
                *out.last_mut().unwrap() = q;
                continue;
            }
        }
        out.push(q);
    }
    out
}

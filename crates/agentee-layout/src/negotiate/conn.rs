use super::grid::{Shape, touch};
use agentee_core::geom::{self, P};
use agentee_core::layout::{Layout, ViaSource, ZoneFill};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum Item {
    Pad { shapes: Vec<Shape>, layers: Vec<usize>, centre: P, pitch: Option<f64> },
    Seg { layer: usize, shape: Shape },
    Via { at: P, r: f64, layers: Vec<usize> },
    Island { zone: usize, label: u32, layer: usize },
}

pub struct Islands {
    pub labels: HashMap<usize, Vec<u32>>,
}

impl Islands {
    pub fn new(layout: &Layout, nets: &[usize]) -> Islands {
        let mut labels = HashMap::new();
        for (zi, z) in layout.zones.iter().enumerate() {
            if nets.contains(&z.net) {
                labels.insert(zi, label(z));
            }
        }
        Islands { labels }
    }

    pub fn at(&self, layout: &Layout, zone: usize, p: P) -> Option<u32> {
        let z = &layout.zones[zone];
        let x = ((p[0] - z.origin[0]) / z.cell).floor();
        let y = ((p[1] - z.origin[1]) / z.cell).floor();
        if x < 0.0 || y < 0.0 || x as usize >= z.width || y as usize >= z.height {
            return None;
        }
        let v = self.labels.get(&zone)?[y as usize * z.width + x as usize];
        (v != 0).then_some(v)
    }
}

fn label(z: &ZoneFill) -> Vec<u32> {
    let (w, h) = (z.width, z.height);
    let mut out = vec![0u32; w * h];
    if z.mask.len() < w * h {
        return out;
    }
    let mut next = 0;
    let mut stack = Vec::new();
    for s in 0..w * h {
        if z.mask[s] == 0 || out[s] != 0 {
            continue;
        }
        next += 1;
        out[s] = next;
        stack.push(s);
        while let Some(c) = stack.pop() {
            let (x, y) = (c % w, c / w);
            let mut go = |n: usize| {
                if z.mask[n] != 0 && out[n] == 0 {
                    out[n] = next;
                    stack.push(n);
                }
            };
            if x > 0 {
                go(c - 1);
            }
            if x + 1 < w {
                go(c + 1);
            }
            if y > 0 {
                go(c - w);
            }
            if y + 1 < h {
                go(c + w);
            }
        }
    }
    out
}

pub struct NetCopper {
    pub items: Vec<Item>,
    pub group: Vec<usize>,
    pub groups: usize,
}

impl NetCopper {
    pub fn points(&self, g: usize) -> Vec<P> {
        let mut out = Vec::new();
        for (i, it) in self.items.iter().enumerate().filter(|(i, _)| self.group[*i] == g) {
            let _ = i;
            match it {
                Item::Pad { centre, .. } => out.push(*centre),
                Item::Via { at, .. } => out.push(*at),
                Item::Seg { shape: Shape::Seg(a, b, _), .. } => {
                    out.push(*a);
                    out.push(*b);
                }
                _ => {}
            }
        }
        out
    }

    pub fn has_pad(&self, g: usize) -> bool {
        self.items
            .iter()
            .zip(&self.group)
            .any(|(it, &k)| k == g && matches!(it, Item::Pad { .. }))
    }

    pub fn islands(&self, g: usize) -> Vec<(usize, u32)> {
        self.items
            .iter()
            .zip(&self.group)
            .filter(|(_, k)| **k == g)
            .filter_map(|(it, _)| match it {
                Item::Island { zone, label, .. } => Some((*zone, *label)),
                _ => None,
            })
            .collect()
    }
}

fn find(p: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while p[r] != r {
        r = p[r];
    }
    let mut c = i;
    while p[c] != r {
        let n = p[c];
        p[c] = r;
        c = n;
    }
    r
}

pub fn net_copper(layout: &Layout, islands: &Islands, net: usize) -> NetCopper {
    let layer_of = |n: &str| layout.copper.iter().position(|c| c == n);
    let mut items: Vec<Item> = Vec::new();
    let mut part_of: Vec<Option<usize>> = Vec::new();
    for (pi, part) in layout.parts.iter().enumerate() {
        if !part.pads.iter().any(|q| q.net == Some(net)) {
            continue;
        }
        let pitch = crate::escape::is_bga(part).then(|| crate::escape::pitch_of(part));
        for pad in part.pads.iter().filter(|q| q.net == Some(net)) {
            let layers: Vec<usize> = pad.copper.iter().filter_map(|c| layer_of(c)).collect();
            if layers.is_empty() || pad.outlines.is_empty() {
                continue;
            }
            let shapes: Vec<Shape> = pad.outlines.iter().map(|o| Shape::Poly(o.clone())).collect();
            let mut lo = [f64::MAX; 2];
            let mut hi = [f64::MIN; 2];
            for q in pad.outlines.iter().flatten() {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
            let centre = pad
                .drill
                .map(|d| d.0)
                .unwrap_or([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]);
            items.push(Item::Pad { shapes, layers, centre, pitch });
            part_of.push(Some(pi));
        }
    }
    for t in layout.tracks.iter().filter(|t| t.net == net) {
        let Some(l) = layer_of(&t.layer) else { continue };
        for w in t.points.windows(2) {
            items.push(Item::Seg { layer: l, shape: Shape::Seg(w[0], w[1], t.width / 2.0) });
            part_of.push(None);
        }
    }
    for v in layout.vias.iter().filter(|v| v.net == net && !matches!(v.source, ViaSource::Stitch(_))) {
        let layers: Vec<usize> = v.layers.iter().filter_map(|c| layer_of(c)).collect();
        items.push(Item::Via { at: v.at, r: v.diameter / 2.0, layers });
        part_of.push(None);
    }
    let solid = items.len();
    let mut parent: Vec<usize> = (0..solid).collect();
    let shapes_of = |it: &Item| -> Vec<(usize, Shape)> {
        match it {
            Item::Pad { shapes, layers, .. } => layers
                .iter()
                .flat_map(|&l| shapes.iter().map(move |s| (l, s.clone())))
                .collect(),
            Item::Seg { layer, shape } => vec![(*layer, shape.clone())],
            Item::Via { at, r, layers } => {
                layers.iter().map(|&l| (l, Shape::Circle(*at, *r))).collect()
            }
            Item::Island { .. } => Vec::new(),
        }
    };
    let all: Vec<Vec<(usize, Shape)>> = items.iter().map(shapes_of).collect();
    for i in 0..solid {
        for j in i + 1..solid {
            if let (Some(a), Some(b)) = (part_of[i], part_of[j])
                && a == b
                && !all[i].iter().any(|(l, s)| all[j].iter().any(|(m, t)| l == m && touch(s, t)))
            {
                continue;
            }
            let joined =
                all[i].iter().any(|(l, s)| all[j].iter().any(|(m, t)| l == m && touch(s, t)));
            if joined {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut island_items: Vec<(usize, u32, usize, Vec<usize>)> = Vec::new();
    for (zi, z) in layout.zones.iter().enumerate().filter(|(_, z)| z.net == net) {
        let Some(layer) = layer_of(&z.layer) else { continue };
        if !islands.labels.contains_key(&zi) {
            continue;
        }
        let reach = z.cell * 1.5;
        for (i, shapes) in all.iter().enumerate() {
            for (l, s) in shapes.iter().filter(|(l, _)| *l == layer) {
                let _ = l;
                let (lo, hi) = s.bounds();
                let x0 = (((lo[0] - reach) - z.origin[0]) / z.cell).floor().max(0.0) as usize;
                let y0 = (((lo[1] - reach) - z.origin[1]) / z.cell).floor().max(0.0) as usize;
                let x1 = ((((hi[0] + reach) - z.origin[0]) / z.cell).ceil().max(0.0) as usize)
                    .min(z.width.saturating_sub(1));
                let y1 = ((((hi[1] + reach) - z.origin[1]) / z.cell).ceil().max(0.0) as usize)
                    .min(z.height.saturating_sub(1));
                let labels = &islands.labels[&zi];
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let v = labels[y * z.width + x];
                        if v == 0 {
                            continue;
                        }
                        let c = [
                            z.origin[0] + (x as f64 + 0.5) * z.cell,
                            z.origin[1] + (y as f64 + 0.5) * z.cell,
                        ];
                        if s.dist(c) <= reach {
                            match island_items.iter_mut().find(|e| e.0 == zi && e.1 == v) {
                                Some(e) => {
                                    if !e.3.contains(&i) {
                                        e.3.push(i);
                                    }
                                }
                                None => island_items.push((zi, v, layer, vec![i])),
                            }
                        }
                    }
                }
            }
        }
    }
    for (zi, label, layer, touched) in island_items {
        items.push(Item::Island { zone: zi, label, layer });
        parent.push(parent.len());
        let me = parent.len() - 1;
        for t in touched {
            let (a, b) = (find(&mut parent, t), find(&mut parent, me));
            parent[a] = b;
        }
    }
    let mut ids: HashMap<usize, usize> = HashMap::new();
    let mut group = Vec::with_capacity(items.len());
    for i in 0..items.len() {
        let r = find(&mut parent, i);
        let n = ids.len();
        group.push(*ids.entry(r).or_insert(n));
    }
    let groups = ids.len();
    NetCopper { items, group, groups }
}

pub fn group_distance(
    layout: &Layout,
    islands: &Islands,
    nc: &NetCopper,
    from: &[P],
    to_points: &[P],
    to_islands: &[(usize, u32)],
) -> (f64, P, P) {
    let mut best = (f64::MAX, [0.0; 2], [0.0; 2]);
    for &p in from {
        for &q in to_points {
            let d = geom::dist(p, q);
            if d < best.0 {
                best = (d, p, q);
            }
        }
        for &(z, label) in to_islands {
            if islands.at(layout, z, p) == Some(label) {
                return (0.0, p, p);
            }
        }
    }
    let _ = nc;
    best
}

use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::route::{RoutedTrack, RoutedVia, Unrouted};
use serde::Serialize;
use spade::{DelaunayTriangulation, HasPosition, Point2, Triangulation};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

pub struct Topo;

#[derive(Clone, Debug, Default, Serialize)]
pub struct TopoPlan {
    pub connections: usize,
    pub routed: usize,
    pub tracks: Vec<RoutedTrack>,
    pub vias: Vec<RoutedVia>,
    pub failed: Vec<Unrouted>,
}

impl TopoPlan {
    pub fn as_detail(&self) -> crate::detail::DetailPlan {
        crate::detail::DetailPlan {
            connections: self.connections,
            routed: self.routed,
            tracks: self.tracks.clone(),
            vias: self.vias.clone(),
            failed: self.failed.clone(),
            ..Default::default()
        }
    }
}

const NONE: u32 = u32::MAX;
const SAMPLE: f64 = 0.2;
const MARGIN: f64 = 0.01;

#[derive(Clone)]
struct Obs {
    net: Option<usize>,
    poly: Vec<P>,
    clear: f64,
}

#[derive(Clone, Copy)]
enum Owner {
    Obs(u32),
    Site(u32),
}

struct Vx {
    at: P,
}

impl HasPosition for Vx {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        Point2::new(self.at[0], self.at[1])
    }
}

struct Site {
    at: P,
    room: f64,
    only: Option<usize>,
    used: Option<usize>,
    radius: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum End {
    Cross(u32),
    Vertex(u32),
}

struct Layer {
    name: String,
    pts: Vec<P>,
    owner: Vec<Owner>,
    tris: Vec<[u32; 3]>,
    solid: Vec<bool>,
    tri_portals: Vec<[u32; 3]>,
    portals: Vec<Portal>,
    vertex_tris: Vec<Vec<u32>>,
    site_vertex: Vec<u32>,
    chords: Vec<Vec<(usize, End, End)>>,
}

struct Portal {
    a: u32,
    b: u32,
    tris: [u32; 2],
    wires: Vec<usize>,
}

impl Layer {
    fn portal_of(&self, t: u32, k: usize) -> u32 {
        self.tri_portals[t as usize][k]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Node {
    Slot(u8, u32, u32),
    Vert(u8, u32),
}

#[derive(PartialEq)]
struct Q(f64, Node);
impl Eq for Q {}
impl PartialOrd for Q {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Q {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

#[derive(Clone)]
struct Step {
    layer: u8,
    tri: u32,
    from: End,
    to: End,
    slot: u32,
}

struct Wire {
    width: f64,
    clear: f64,
    steps: Vec<Step>,
    sites: Vec<u32>,
    start_site: Option<u32>,
    end_site: Option<u32>,
}

struct Conn {
    net: usize,
    a: P,
    b: P,
    width: f64,
    clear: f64,
    layers: Vec<u8>,
    via_radius: f64,
    via_name: String,
}

fn sample(poly: &[P], step: f64) -> Vec<P> {
    let mut out = Vec::new();
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let k = (geom::dist(a, b) / step).ceil().max(1.0) as usize;
        for j in 0..k {
            let t = j as f64 / k as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

fn capsule(a: P, b: P, r: f64) -> Vec<P> {
    let d = geom::dist(a, b).max(1e-9);
    let u = [(b[0] - a[0]) / d, (b[1] - a[1]) / d];
    let mut out = Vec::new();
    for k in 0..=4 {
        let ang = std::f64::consts::PI * (0.5 + k as f64 / 4.0);
        let (c, s) = (ang.cos(), ang.sin());
        out.push([a[0] + (u[0] * c - u[1] * s) * r, a[1] + (u[1] * c + u[0] * s) * r]);
    }
    for k in 0..=4 {
        let ang = std::f64::consts::PI * (1.5 + k as f64 / 4.0);
        let (c, s) = (ang.cos(), ang.sin());
        out.push([b[0] + (u[0] * c - u[1] * s) * r, b[1] + (u[1] * c + u[0] * s) * r]);
    }
    out
}

fn disc(c: P, r: f64) -> Vec<P> {
    (0..8)
        .map(|k| {
            let a = k as f64 * std::f64::consts::TAU / 8.0;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

fn poly_dist(q: P, poly: &[P]) -> f64 {
    if geom::point_in_polygon(q, poly) {
        return 0.0;
    }
    let n = poly.len();
    (0..n)
        .map(|i| geom::point_segment_distance(q, poly[i], poly[(i + 1) % n]))
        .fold(f64::MAX, f64::min)
}

struct Grid {
    origin: P,
    cell: f64,
    nx: usize,
    ny: usize,
    items: Vec<Vec<u32>>,
}

impl Grid {
    fn new(b: Bounds, cell: f64) -> Grid {
        let nx = ((b.max[0] - b.min[0]) / cell).ceil() as usize + 1;
        let ny = ((b.max[1] - b.min[1]) / cell).ceil() as usize + 1;
        Grid { origin: b.min, cell, nx, ny, items: vec![Vec::new(); nx * ny] }
    }

    fn range(&self, b: &Bounds) -> (usize, usize, usize, usize) {
        let c =
            |v: f64, o: f64, n: usize| (((v - o) / self.cell).floor().max(0.0) as usize).min(n - 1);
        (
            c(b.min[0], self.origin[0], self.nx),
            c(b.min[1], self.origin[1], self.ny),
            c(b.max[0], self.origin[0], self.nx),
            c(b.max[1], self.origin[1], self.ny),
        )
    }

    fn insert(&mut self, id: u32, b: &Bounds) {
        let (x0, y0, x1, y1) = self.range(b);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.items[y * self.nx + x].push(id);
            }
        }
    }

    fn near(&self, q: P, r: f64) -> Vec<u32> {
        let mut b = Bounds::EMPTY;
        b.add_circle(q, r);
        let (x0, y0, x1, y1) = self.range(&b);
        let mut out = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                out.extend(&self.items[y * self.nx + x]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

struct Router {
    layers: Vec<Layer>,
    obs: Vec<Vec<Obs>>,
    sites: Vec<Site>,
    site_grid: Grid,
    wires: Vec<Option<Wire>>,
}

impl Router {
    fn site_ok(&self, s: u32, net: usize, radius: f64, clear: f64) -> bool {
        let st = &self.sites[s as usize];
        if st.used.is_some_and(|u| u != net) || st.room < radius {
            return false;
        }
        if st.used == Some(net) {
            return true;
        }
        for o in self.site_grid.near(st.at, radius * 2.0 + clear + 0.6) {
            if o == s {
                continue;
            }
            let other = &self.sites[o as usize];
            if other.used.is_some() && geom::dist(other.at, st.at) < radius + other.radius + clear {
                return false;
            }
        }
        for (l, ly) in self.layers.iter().enumerate() {
            let v = ly.site_vertex[s as usize];
            if v == NONE {
                continue;
            }
            for &t in &ly.vertex_tris[v as usize] {
                for k in 0..3 {
                    let p = ly.tri_portals[t as usize][k];
                    let portal = &ly.portals[p as usize];
                    if (portal.a == v || portal.b == v)
                        && !portal.wires.is_empty()
                        && self.room(l, p) < radius + clear
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn boundary_pos(
        &self,
        l: usize,
        t: u32,
        e: End,
        wire: Option<usize>,
        slot: Option<u32>,
    ) -> f64 {
        let ly = &self.layers[l];
        let tri = ly.tris[t as usize];
        match e {
            End::Vertex(v) => {
                let k = tri.iter().position(|&x| x == v).unwrap_or(0);
                k as f64 * 1e6
            }
            End::Cross(p) => {
                let k = (0..3).find(|&k| ly.tri_portals[t as usize][k] == p).unwrap_or(0);
                let portal = &ly.portals[p as usize];
                let forward = portal.a == tri[k];
                let n = portal.wires.len() as f64;
                let idx = match (wire, slot) {
                    (Some(w), _) => {
                        portal.wires.iter().position(|&x| x == w).unwrap_or(0) as f64 + 1.0
                    }
                    (None, Some(s)) => s as f64 + 0.5,
                    _ => 0.0,
                };
                let along = if forward { idx } else { n + 1.0 - idx };
                k as f64 * 1e6 + along
            }
        }
    }

    fn crosses(
        &self,
        l: usize,
        t: u32,
        from: (End, Option<u32>),
        to: (End, Option<u32>),
    ) -> Vec<usize> {
        let x = self.boundary_pos(l, t, from.0, None, from.1);
        let y = self.boundary_pos(l, t, to.0, None, to.1);
        let (lo, hi) = if x < y { (x, y) } else { (y, x) };
        let mut hit = Vec::new();
        for &(w, a, b) in &self.layers[l].chords[t as usize] {
            let p = self.boundary_pos(l, t, a, Some(w), None);
            let q = self.boundary_pos(l, t, b, Some(w), None);
            let inside = |v: f64| v > lo + 1e-9 && v < hi - 1e-9;
            let shared = |v: f64| (v - lo).abs() < 1e-9 || (v - hi).abs() < 1e-9;
            if shared(p) || shared(q) {
                continue;
            }
            if inside(p) != inside(q) {
                hit.push(w);
            }
        }
        hit
    }

    fn room(&self, l: usize, p: u32) -> f64 {
        let ly = &self.layers[l];
        let portal = &ly.portals[p as usize];
        let (a, b) = (ly.pts[portal.a as usize], ly.pts[portal.b as usize]);
        let end = |v: u32| match ly.owner[v as usize] {
            Owner::Obs(o) => self.obs[l][o as usize].clear,
            Owner::Site(s) => {
                let site = &self.sites[s as usize];
                if site.used.is_some() { site.radius + 0.1 } else { 0.0 }
            }
        };
        let used: f64 = portal
            .wires
            .iter()
            .filter_map(|&w| self.wires[w].as_ref())
            .map(|w| w.width + w.clear)
            .sum();
        geom::dist(a, b) - end(portal.a) - end(portal.b) - used
    }

    fn slot_point(&self, l: usize, p: u32, s: u32) -> P {
        let ly = &self.layers[l];
        let portal = &ly.portals[p as usize];
        let (a, b) = (ly.pts[portal.a as usize], ly.pts[portal.b as usize]);
        let n = portal.wires.len() as f64;
        let t = (s as f64 + 1.0) / (n + 2.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

fn build_layer(name: &str, obs: &[Obs], sites: &[Site], outline: &[P]) -> Layer {
    let mut tri: DelaunayTriangulation<Vx> = DelaunayTriangulation::new();
    let mut handles: Vec<(P, Owner)> = Vec::new();
    for (i, o) in obs.iter().enumerate() {
        for q in sample(&o.poly, SAMPLE) {
            handles.push((q, Owner::Obs(i as u32)));
        }
    }
    for (i, s) in sites.iter().enumerate() {
        handles.push((s.at, Owner::Site(i as u32)));
    }
    let edge_obs = obs.len() as u32;
    let mut ring = outline.to_vec();
    if geom::signed_area(&ring) < 0.0 {
        ring.reverse();
    }
    for q in sample(&ring, SAMPLE * 2.0) {
        handles.push((q, Owner::Obs(edge_obs)));
    }
    let mut pts = Vec::new();
    let mut owner = Vec::new();
    let mut index: HashMap<usize, u32> = HashMap::new();
    for (q, o) in handles {
        if let Ok(h) = tri.insert(Vx { at: q }) {
            let k = h.index();
            index.entry(k).or_insert_with(|| {
                pts.push(q);
                owner.push(o);
                (pts.len() - 1) as u32
            });
        }
    }
    let vid = |h: spade::handles::FixedVertexHandle| index[&h.index()];
    let mut tris = Vec::new();
    let mut face_id: HashMap<usize, u32> = HashMap::new();
    for f in tri.inner_faces() {
        let v = f.vertices();
        face_id.insert(f.fix().index(), tris.len() as u32);
        tris.push([vid(v[0].fix()), vid(v[1].fix()), vid(v[2].fix())]);
    }
    let mut solid = vec![false; tris.len()];
    for (t, tv) in tris.iter().enumerate() {
        let c = [
            (pts[tv[0] as usize][0] + pts[tv[1] as usize][0] + pts[tv[2] as usize][0]) / 3.0,
            (pts[tv[0] as usize][1] + pts[tv[1] as usize][1] + pts[tv[2] as usize][1]) / 3.0,
        ];
        if !geom::point_in_polygon(c, outline) {
            solid[t] = true;
            continue;
        }
        for &v in tv {
            if let Owner::Obs(o) = owner[v as usize]
                && (o as usize) < obs.len()
                && geom::point_in_polygon(c, &obs[o as usize].poly)
            {
                solid[t] = true;
            }
        }
    }
    let mut portals: Vec<Portal> = Vec::new();
    let mut key: HashMap<(u32, u32), u32> = HashMap::new();
    let mut tri_portals = vec![[NONE; 3]; tris.len()];
    for (t, tv) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (tv[k], tv[(k + 1) % 3]);
            let kk = (a.min(b), a.max(b));
            let id = *key.entry(kk).or_insert_with(|| {
                portals.push(Portal { a, b, tris: [NONE, NONE], wires: Vec::new() });
                (portals.len() - 1) as u32
            });
            let p = &mut portals[id as usize];
            if p.tris[0] == NONE {
                p.tris[0] = t as u32
            } else {
                p.tris[1] = t as u32
            }
            tri_portals[t][k] = id;
        }
    }
    let mut vertex_tris = vec![Vec::new(); pts.len()];
    for (t, tv) in tris.iter().enumerate() {
        if solid[t] {
            continue;
        }
        for &v in tv {
            vertex_tris[v as usize].push(t as u32);
        }
    }
    let mut site_vertex = vec![NONE; sites.len()];
    for (v, o) in owner.iter().enumerate() {
        if let Owner::Site(s) = o {
            site_vertex[*s as usize] = v as u32;
        }
    }
    let n = tris.len();
    Layer {
        name: name.to_string(),
        pts,
        owner,
        tris,
        solid,
        tri_portals,
        portals,
        vertex_tris,
        site_vertex,
        chords: vec![Vec::new(); n],
    }
}

impl Phase for Topo {
    fn name(&self) -> &'static str {
        "topo"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "topo".into(), ..Default::default() };
        let l = &model.layout;
        let b = model.board;
        let full: Vec<(&str, &String)> = model
            .file
            .zones
            .iter()
            .filter(|z| z.outline.as_ref().is_none_or(|o| o.is_empty()))
            .flat_map(|z| z.layers.iter().map(move |ly| (z.net.as_str(), ly)))
            .collect();
        let outer = |ly: &str| {
            l.copper.first().is_some_and(|c| c == ly) || l.copper.last().is_some_and(|c| c == ly)
        };
        let names: Vec<String> = l
            .copper
            .iter()
            .filter(|c| outer(c) || !full.iter().any(|(_, z)| *z == *c))
            .cloned()
            .collect();
        let edge = b.rules.min_copper_to_edge.to_mm();
        let in_pad = cfg.access.as_ref().and_then(|a| a.via_in_pad).unwrap_or(false);

        let mut obs: Vec<Vec<Obs>> = vec![Vec::new(); names.len()];
        for p in &l.parts {
            for pad in &p.pads {
                let clear = pad.net.map(|n| l.nets[n].clearance).unwrap_or(0.1);
                for (li, ly) in names.iter().enumerate() {
                    if !pad.copper.contains(ly) {
                        continue;
                    }
                    for o in &pad.outlines {
                        obs[li].push(Obs { net: pad.net, poly: o.clone(), clear });
                    }
                }
            }
        }
        for v in &l.vias {
            for (li, ly) in names.iter().enumerate() {
                if v.layers.contains(ly) {
                    obs[li].push(Obs {
                        net: Some(v.net),
                        poly: disc(v.at, v.diameter / 2.0),
                        clear: l.nets[v.net].clearance,
                    });
                }
            }
        }
        for t in &l.tracks {
            let Some(li) = names.iter().position(|n| *n == t.layer) else { continue };
            for w in t.points.windows(2) {
                obs[li].push(Obs {
                    net: Some(t.net),
                    poly: capsule(w[0], w[1], t.width / 2.0),
                    clear: l.nets[t.net].clearance,
                });
            }
        }

        let mut bounds = Bounds::EMPTY;
        l.outline.iter().for_each(|q| bounds.add(*q));
        let smallest = b
            .vias
            .iter()
            .min_by(|x, y| x.diameter.to_mm().total_cmp(&y.diameter.to_mm()))
            .map(|v| v.diameter.to_mm() / 2.0)
            .unwrap_or(0.2);
        let mut grids: Vec<Grid> = Vec::new();
        for layer_obs in &obs {
            let mut g = Grid::new(bounds, 1.0);
            for (i, o) in layer_obs.iter().enumerate() {
                let mut bb = Bounds::EMPTY;
                o.poly.iter().for_each(|q| bb.add(*q));
                g.insert(i as u32, &bb);
            }
            grids.push(g);
        }
        let all_copper: Vec<(P, f64, Option<usize>, f64)> = l
            .vias
            .iter()
            .map(|v| (v.at, v.diameter / 2.0, Some(v.net), l.nets[v.net].clearance))
            .collect();
        let step = cfg.topo.as_ref().and_then(|t| t.site_pitch).map(|v| v.to_mm()).unwrap_or(0.4);
        let mut sites: Vec<Site> = Vec::new();
        let mut taken: Grid = Grid::new(bounds, 1.0);
        let mut y = bounds.min[1] + step / 2.0;
        while y < bounds.max[1] {
            let mut x = bounds.min[0] + step / 2.0;
            while x < bounds.max[0] {
                let q = [x, y];
                x += step;
                if !geom::point_in_polygon(q, &l.outline) {
                    continue;
                }
                let edge_room = l
                    .outline
                    .windows(2)
                    .map(|w| geom::point_segment_distance(q, w[0], w[1]))
                    .fold(f64::MAX, f64::min)
                    - edge;
                let mut room = edge_room;
                for li in 0..names.len() {
                    for i in grids[li].near(q, 1.0) {
                        let o = &obs[li][i as usize];
                        room = room.min(poly_dist(q, &o.poly) - o.clear);
                    }
                }
                for (c, r, _, cl) in &all_copper {
                    room = room.min(geom::dist(q, *c) - r - cl);
                }
                if room < smallest {
                    continue;
                }
                if taken
                    .near(q, step)
                    .iter()
                    .any(|&s| geom::dist(sites[s as usize].at, q) < step - 1e-9)
                {
                    continue;
                }
                let mut bb = Bounds::EMPTY;
                bb.add(q);
                taken.insert(sites.len() as u32, &bb);
                sites.push(Site { at: q, room, only: None, used: None, radius: 0.0 });
            }
            y += step;
        }
        if in_pad {
            for p in &l.parts {
                if !crate::escape::is_bga(p) {
                    continue;
                }
                for pad in &p.pads {
                    let Some(n) = pad.net else { continue };
                    if l.vias.iter().any(|v| {
                        v.net == n && pad.outlines.iter().any(|o| geom::point_in_polygon(v.at, o))
                    }) {
                        continue;
                    }
                    let mut bb = Bounds::EMPTY;
                    pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
                    sites.push(Site {
                        at: bb.center(),
                        room: smallest,
                        only: Some(n),
                        used: None,
                        radius: 0.0,
                    });
                }
            }
        }

        let layers: Vec<Layer> = names
            .iter()
            .enumerate()
            .map(|(li, n)| build_layer(n, &obs[li], &sites, &l.outline))
            .collect();
        let mut site_grid = Grid::new(bounds, 1.0);
        for (i, st) in sites.iter().enumerate() {
            let mut bb = Bounds::EMPTY;
            bb.add(st.at);
            site_grid.insert(i as u32, &bb);
        }
        let mut router = Router { layers, obs, sites, site_grid, wires: Vec::new() };
        for layer_obs in router.obs.iter_mut() {
            layer_obs.push(Obs { net: None, poly: Vec::new(), clear: edge });
        }

        let class_order = class_order(model);
        let rank = |c: &str| class_order.iter().position(|k| k == c).unwrap_or(usize::MAX);
        let mut conns: Vec<Conn> = Vec::new();
        for &(a, c, n) in &l.ratsnest {
            let net = &l.nets[n];
            let class = b.netclasses.iter().find(|k| k.name == net.class);
            let allowed: Vec<u8> = names
                .iter()
                .enumerate()
                .filter(|(_, ly)| {
                    class.is_none_or(|k| k.layers.is_empty() || k.layers.contains(ly))
                })
                .map(|(i, _)| i as u8)
                .collect();
            if allowed.is_empty() {
                continue;
            }
            let via = b.via_for(None, class, &[]);
            conns.push(Conn {
                net: n,
                a,
                b: c,
                width: net.width,
                clear: net.clearance,
                layers: allowed,
                via_radius: via.map(|v| v.diameter.to_mm() / 2.0).unwrap_or(0.25),
                via_name: via.map(|v| v.name.clone()).unwrap_or_default(),
            });
        }
        conns.sort_by(|x, y| {
            rank(&l.nets[x.net].class)
                .cmp(&rank(&l.nets[y.net].class))
                .then(geom::dist(x.a, x.b).total_cmp(&geom::dist(y.a, y.b)))
        });
        let via_cost = cfg
            .topo
            .as_ref()
            .and_then(|t| t.via_cost)
            .or(cfg.detail.as_ref().and_then(|d| d.via_cost))
            .map(|v| v.to_mm())
            .unwrap_or(3.0);
        let rips = cfg.topo.as_ref().and_then(|t| t.rip_limit).unwrap_or(4);

        let mut queue: std::collections::VecDeque<usize> = (0..conns.len()).collect();
        let mut owner_of: Vec<Option<usize>> = vec![None; conns.len()];
        let mut ripped = vec![0usize; conns.len()];
        let mut failed: Vec<usize> = Vec::new();
        let mut budget = conns.len() * 6;
        while let Some(ci) = queue.pop_front() {
            let c = &conns[ci];
            match route_one(&router, c, via_cost, false) {
                Some(path) => {
                    let w = commit(&mut router, c, &path);
                    owner_of[ci] = Some(w);
                }
                None => {
                    if budget == 0 || ripped[ci] >= rips {
                        failed.push(ci);
                        continue;
                    }
                    budget -= 1;
                    match route_one(&router, c, via_cost, true) {
                        Some(path) => {
                            let mut hit: Vec<usize> = path.hits.clone();
                            hit.sort_unstable();
                            hit.dedup();
                            for w in hit {
                                if let Some(k) = owner_of.iter().position(|o| *o == Some(w)) {
                                    rip(&mut router, w);
                                    owner_of[k] = None;
                                    ripped[k] += 1;
                                    queue.push_back(k);
                                }
                            }
                            match route_one(&router, c, via_cost, false) {
                                Some(p) => {
                                    let w = commit(&mut router, c, &p);
                                    owner_of[ci] = Some(w);
                                }
                                None => {
                                    ripped[ci] += 1;
                                    queue.push_back(ci);
                                }
                            }
                        }
                        None => failed.push(ci),
                    }
                }
            }
        }

        let mut plan = TopoPlan { connections: conns.len(), ..Default::default() };
        for (ci, o) in owner_of.iter().enumerate() {
            let Some(w) = o else { continue };
            plan.routed += 1;
            let c = &conns[ci];
            let (tracks, vias) = geometry(&router, *w, c, l);
            plan.tracks.extend(tracks);
            plan.vias.extend(vias);
        }
        for &ci in &failed {
            let c = &conns[ci];
            plan.failed.push(Unrouted {
                net: l.nets[c.net].name.clone(),
                from: c.a,
                to: c.b,
                reason: "no topological route".into(),
            });
        }
        report.notes.push(format!(
            "{} of {} connections, {} tracks, {} vias, {} via sites",
            plan.routed,
            plan.connections,
            plan.tracks.len(),
            plan.vias.len(),
            router.sites.len()
        ));
        report.failed = plan.failed.iter().map(|f| format!("{}: {}", f.net, f.reason)).collect();
        report.changed = !plan.tracks.is_empty();
        model.topo = Some(plan);
        report
    }
}

fn class_order(model: &Model) -> Vec<String> {
    let b = model.board;
    let l = &model.layout;
    let mut classes: Vec<(u8, i64, usize, String)> = b
        .netclasses
        .iter()
        .map(|c| {
            let nets: Vec<&agentee_core::layout::LayoutNet> =
                l.nets.iter().filter(|n| n.class == c.name).collect();
            let supply = !nets.is_empty()
                && nets.iter().all(|n| agentee_core::place::is_power_net(b, &n.name, &n.class));
            let rank = if supply || c.current.is_some() {
                3
            } else if c.layers.len() == 1 || c.impedance.is_some() || c.diff_gap.is_some() {
                0
            } else {
                1
            };
            let clearance = (c.clearance.to_mm() * 1000.0) as i64;
            (rank, -clearance, nets.len(), c.name.clone())
        })
        .collect();
    classes.sort();
    classes.into_iter().map(|c| c.3).collect()
}

struct Term {
    layer: u8,
    vertex: u32,
    cost: f64,
    site: Option<u32>,
}

fn terminals(router: &Router, c: &Conn, q: P, via_cost: f64) -> Vec<Term> {
    let mut out = Vec::new();
    let mut pads: Vec<Vec<P>> = Vec::new();
    for &li in &c.layers {
        let ly = &router.layers[li as usize];
        let obs = &router.obs[li as usize];
        for (v, o) in ly.owner.iter().enumerate() {
            let Owner::Obs(oi) = o else { continue };
            let Some(ob) = obs.get(*oi as usize) else { continue };
            if ob.net != Some(c.net) || ob.poly.is_empty() || poly_dist(q, &ob.poly) > 0.05 {
                continue;
            }
            if !pads.contains(&ob.poly) {
                pads.push(ob.poly.clone());
            }
            if !ly.vertex_tris[v].is_empty() {
                out.push(Term { layer: li, vertex: v as u32, cost: 0.0, site: None });
            }
        }
    }
    for (s, site) in router.sites.iter().enumerate() {
        if site.only != Some(c.net) || !pads.iter().any(|p| geom::point_in_polygon(site.at, p)) {
            continue;
        }
        if site.used.is_some_and(|u| u != c.net) {
            continue;
        }
        for &li in &c.layers {
            let v = router.layers[li as usize].site_vertex[s];
            if v != NONE && !router.layers[li as usize].vertex_tris[v as usize].is_empty() {
                out.push(Term { layer: li, vertex: v, cost: via_cost, site: Some(s as u32) });
            }
        }
    }
    out
}

struct Path {
    steps: Vec<Step>,
    sites: Vec<u32>,
    hits: Vec<usize>,
    start_site: Option<u32>,
    end_site: Option<u32>,
}

type Prev = HashMap<Node, (Node, Option<Step>, Vec<usize>)>;

fn search(router: &Router, c: &Conn, via_cost: f64, soft: bool, blocked: &[u32]) -> Option<Path> {
    let starts = terminals(router, c, c.a, via_cost);
    let goals = terminals(router, c, c.b, via_cost);
    if starts.is_empty() || goals.is_empty() {
        return None;
    }
    let goal_of: HashMap<(u8, u32), (f64, Option<u32>)> =
        goals.iter().map(|t| ((t.layer, t.vertex), (t.cost, t.site))).collect();
    let start_of: HashMap<(u8, u32), Option<u32>> =
        starts.iter().map(|t| ((t.layer, t.vertex), t.site)).collect();
    let need = c.width + c.clear;
    let mut dist: HashMap<Node, f64> = HashMap::new();
    let mut prev: Prev = HashMap::new();
    let mut heap = BinaryHeap::new();
    let pos = |n: Node| -> P {
        match n {
            Node::Slot(l, p, s) => router.slot_point(l as usize, p, s),
            Node::Vert(l, v) => router.layers[l as usize].pts[v as usize],
        }
    };
    let h = |q: P| geom::dist(q, c.b);
    for t in &starts {
        let n = Node::Vert(t.layer, t.vertex);
        if dist.get(&n).is_none_or(|&o| t.cost < o) {
            dist.insert(n, t.cost);
            heap.push(Q(t.cost + h(pos(n)), n));
        }
    }
    let mut found = None;
    let mut expanded = 0usize;
    while let Some(Q(f, n)) = heap.pop() {
        let d = dist[&n];
        if f > d + h(pos(n)) + 1e-9 {
            continue;
        }
        expanded += 1;
        if expanded > 300_000 {
            break;
        }
        let arrived_by_step = prev.get(&n).is_some_and(|p| p.1.is_some());
        if let Node::Vert(l, v) = n
            && arrived_by_step
            && goal_of.contains_key(&(l, v))
        {
            found = Some(n);
            break;
        }
        let here = pos(n);
        let mut relax =
            |m: Node, nd: f64, step: Option<Step>, hit: Vec<usize>, heap: &mut BinaryHeap<Q>| {
                if dist.get(&m).is_none_or(|&o| nd < o - 1e-12) {
                    dist.insert(m, nd);
                    prev.insert(m, (n, step, hit));
                    heap.push(Q(nd + h(pos(m)), m));
                }
            };
        if let Node::Vert(l, v) = n
            && let Owner::Site(s) = router.layers[l as usize].owner[v as usize]
            && arrived_by_step
        {
            if blocked.contains(&s) || !router.site_ok(s, c.net, c.via_radius, c.clear) {
                continue;
            }
            for &l2 in &c.layers {
                if l2 == l {
                    continue;
                }
                let v2 = router.layers[l2 as usize].site_vertex[s as usize];
                if v2 == NONE || router.layers[l2 as usize].vertex_tris[v2 as usize].is_empty() {
                    continue;
                }
                relax(Node::Vert(l2, v2), d + via_cost, None, Vec::new(), &mut heap);
            }
            continue;
        }
        let (layer, tris_from): (u8, Vec<(u32, End, Option<u32>)>) = match n {
            Node::Vert(l, v) => (
                l,
                router.layers[l as usize].vertex_tris[v as usize]
                    .iter()
                    .map(|&t| (t, End::Vertex(v), None))
                    .collect(),
            ),
            Node::Slot(l, p, s) => {
                let ly = &router.layers[l as usize];
                let portal = &ly.portals[p as usize];
                (
                    l,
                    portal
                        .tris
                        .iter()
                        .filter(|&&t| t != NONE && !ly.solid[t as usize])
                        .map(|&t| (t, End::Cross(p), Some(s)))
                        .collect(),
                )
            }
        };
        let ly = &router.layers[layer as usize];
        for (t, from, fslot) in tris_from {
            let tv = ly.tris[t as usize];
            for k in 0..3 {
                let p = ly.portal_of(t, k);
                if End::Cross(p) == from {
                    continue;
                }
                let portal = &ly.portals[p as usize];
                let other = if portal.tris[0] == t { portal.tris[1] } else { portal.tris[0] };
                if other == NONE || ly.solid[other as usize] {
                    continue;
                }
                if let End::Vertex(v) = from
                    && (portal.a == v || portal.b == v)
                {
                    continue;
                }
                if router.room(layer as usize, p) < need {
                    continue;
                }
                let count = portal.wires.len() as u32;
                for s in 0..=count {
                    let hit =
                        router.crosses(layer as usize, t, (from, fslot), (End::Cross(p), Some(s)));
                    if !hit.is_empty() && !soft {
                        continue;
                    }
                    let cost = geom::dist(here, router.slot_point(layer as usize, p, s))
                        + hit.len() as f64 * 20.0;
                    let step = Step { layer, tri: t, from, to: End::Cross(p), slot: s };
                    relax(Node::Slot(layer, p, s), d + cost, Some(step), hit, &mut heap);
                }
            }
            for &v in &tv {
                if End::Vertex(v) == from {
                    continue;
                }
                let goal = goal_of.contains_key(&(layer, v));
                let usable_site = match ly.owner[v as usize] {
                    Owner::Site(s) => {
                        router.sites[s as usize].only.is_none()
                            && !blocked.contains(&s)
                            && router.site_ok(s, c.net, c.via_radius, c.clear)
                    }
                    _ => false,
                };
                if !goal && !usable_site {
                    continue;
                }
                let hit = router.crosses(layer as usize, t, (from, fslot), (End::Vertex(v), None));
                if !hit.is_empty() && !soft {
                    continue;
                }
                let extra = goal_of.get(&(layer, v)).map(|g| g.0).unwrap_or(0.0);
                let cost = geom::dist(here, ly.pts[v as usize]) + hit.len() as f64 * 20.0 + extra;
                let step = Step { layer, tri: t, from, to: End::Vertex(v), slot: 0 };
                relax(Node::Vert(layer, v), d + cost, Some(step), hit, &mut heap);
            }
        }
    }
    let end = found?;
    let mut steps = Vec::new();
    let mut sites = Vec::new();
    let mut hits = Vec::new();
    let end_site = match end {
        Node::Vert(l, v) => goal_of.get(&(l, v)).and_then(|g| g.1),
        _ => None,
    };
    let mut cur = end;
    while let Some((p, step, hit)) = prev.get(&cur).cloned() {
        hits.extend(hit);
        match step {
            Some(st) => steps.push(st),
            None => {
                if let Node::Vert(l1, v1) = p
                    && let Owner::Site(s) = router.layers[l1 as usize].owner[v1 as usize]
                {
                    sites.push(s);
                }
            }
        }
        cur = p;
    }
    let start_site = match cur {
        Node::Vert(l, v) => start_of.get(&(l, v)).copied().flatten(),
        _ => None,
    };
    steps.reverse();
    Some(Path { steps, sites, hits, start_site, end_site })
}

fn insert_crossing(router: &mut Router, l: usize, p: u32, w: usize, slot: u32) {
    let portal = &mut router.layers[l].portals[p as usize];
    let s = (slot as usize).min(portal.wires.len());
    portal.wires.insert(s, w);
}

fn commit(router: &mut Router, c: &Conn, path: &Path) -> usize {
    let w = router.wires.len();
    router.wires.push(Some(Wire {
        width: c.width,
        clear: c.clear,
        steps: path.steps.clone(),
        sites: path.sites.clone(),
        start_site: path.start_site,
        end_site: path.end_site,
    }));
    for st in &path.steps {
        if let End::Cross(p) = st.to {
            insert_crossing(router, st.layer as usize, p, w, st.slot);
        }
        router.layers[st.layer as usize].chords[st.tri as usize].push((w, st.from, st.to));
    }
    for &s in path.sites.iter().chain(path.start_site.iter()).chain(path.end_site.iter()) {
        router.sites[s as usize].used = Some(c.net);
        router.sites[s as usize].radius = c.via_radius;
    }
    w
}

fn rip(router: &mut Router, w: usize) {
    let Some(wire) = router.wires[w].take() else { return };
    for st in &wire.steps {
        let ly = &mut router.layers[st.layer as usize];
        if let End::Cross(p) = st.to {
            ly.portals[p as usize].wires.retain(|&x| x != w);
        }
        ly.chords[st.tri as usize].retain(|c| c.0 != w);
    }
    for s in wire.sites.iter().chain(wire.start_site.iter()).chain(wire.end_site.iter()).copied() {
        let still = router
            .wires
            .iter()
            .flatten()
            .any(|o| o.sites.contains(&s) || o.start_site == Some(s) || o.end_site == Some(s));
        if !still {
            router.sites[s as usize].used = None;
            router.sites[s as usize].radius = 0.0;
        }
    }
}

fn interval(router: &Router, l: usize, p: u32, w: usize) -> (P, P) {
    let ly = &router.layers[l];
    let portal = &ly.portals[p as usize];
    let (a, b) = (ly.pts[portal.a as usize], ly.pts[portal.b as usize]);
    let len = geom::dist(a, b).max(1e-9);
    let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
    let end = |v: u32| match ly.owner[v as usize] {
        Owner::Obs(o) => router.obs[l][o as usize].clear,
        Owner::Site(s) => {
            let site = &router.sites[s as usize];
            if site.used.is_some() { site.radius + 0.1 } else { 0.0 }
        }
    };
    let ws: Vec<&Wire> = portal.wires.iter().filter_map(|&x| router.wires[x].as_ref()).collect();
    let k = portal.wires.iter().position(|&x| x == w).unwrap_or(0);
    let me = router.wires[w].as_ref().unwrap();
    let before: f64 = ws[..k].iter().map(|x| x.width + x.clear).sum();
    let after: f64 = ws[k + 1..].iter().map(|x| x.width + x.clear).sum();
    let lo = end(portal.a).max(me.clear) + MARGIN + before + me.width / 2.0;
    let hi = len - end(portal.b).max(me.clear) - MARGIN - after - me.width / 2.0;
    let (lo, hi) = if lo <= hi {
        (lo, hi)
    } else {
        let m = (lo + hi) / 2.0;
        (m, m)
    };
    ([a[0] + u[0] * lo, a[1] + u[1] * lo], [a[0] + u[0] * hi, a[1] + u[1] * hi])
}

fn tri2(a: P, b: P, c: P) -> f64 {
    -orient(a, b, c)
}

fn funnel(start: P, portals: &[(P, P)], end: P) -> Vec<P> {
    let mut path = vec![start];
    let mut ps: Vec<(P, P)> = vec![(start, start)];
    ps.extend_from_slice(portals);
    ps.push((end, end));
    let same = |a: P, b: P| geom::dist(a, b) < 1e-9;
    let (mut apex, mut pl, mut pr) = (start, start, start);
    let (mut li, mut ri) = (0usize, 0usize);
    let mut ai: usize;
    let mut i = 1;
    let mut guard = 0;
    while i < ps.len() {
        guard += 1;
        if guard > 8 * ps.len() + 16 {
            let mut plain = vec![start];
            plain.extend(portals.iter().map(|(l, r)| [(l[0] + r[0]) / 2.0, (l[1] + r[1]) / 2.0]));
            plain.push(end);
            return plain;
        }
        let (left, right) = ps[i];
        if tri2(apex, pr, right) <= 0.0 {
            if same(apex, pr) || tri2(apex, pl, right) > 0.0 {
                pr = right;
                ri = i;
            } else {
                path.push(pl);
                apex = pl;
                ai = li;
                pl = apex;
                pr = apex;
                li = ai;
                ri = ai;
                i = ai + 1;
                continue;
            }
        }
        if tri2(apex, pl, left) >= 0.0 {
            if same(apex, pl) || tri2(apex, pr, left) < 0.0 {
                pl = left;
                li = i;
            } else {
                path.push(pr);
                apex = pr;
                ai = ri;
                pl = apex;
                pr = apex;
                li = ai;
                ri = ai;
                i = ai + 1;
                continue;
            }
        }
        i += 1;
    }
    if path.last().is_none_or(|q| !same(*q, end)) {
        path.push(end);
    }
    path
}

fn geometry(
    router: &Router,
    w: usize,
    c: &Conn,
    l: &agentee_core::layout::Layout,
) -> (Vec<RoutedTrack>, Vec<RoutedVia>) {
    let Some(wire) = router.wires[w].as_ref() else { return (Vec::new(), Vec::new()) };
    let net = l.nets[c.net].name.clone();
    let mut tracks = Vec::new();
    let mut vias = Vec::new();
    let site_at = |s: u32| router.sites[s as usize].at;
    let mut lead: Option<P> = None;
    let mut start = match wire.start_site {
        Some(s) => {
            vias.push(RoutedVia { net: net.clone(), at: site_at(s), via: c.via_name.clone() });
            site_at(s)
        }
        None => match wire.steps.first().map(|st| (st.layer, st.from)) {
            Some((l, End::Vertex(v))) => {
                lead = Some(c.a);
                router.layers[l as usize].pts[v as usize]
            }
            _ => c.a,
        },
    };
    if let Some(s) = wire.end_site {
        vias.push(RoutedVia { net: net.clone(), at: site_at(s), via: c.via_name.clone() });
    }
    let steps = &wire.steps;
    let mut i = 0;
    while i < steps.len() {
        let layer = steps[i].layer;
        let ly = &router.layers[layer as usize];
        let mut j = i;
        while j < steps.len() && steps[j].layer == layer {
            j += 1;
        }
        let mut portals = Vec::new();
        let mut pivots: Vec<(P, P)> = Vec::new();
        for st in &steps[i..j] {
            if let End::Cross(p) = st.to {
                let (lo, hi) = interval(router, layer as usize, p, w);
                let portal = &ly.portals[p as usize];
                let (pa, pb) = (ly.pts[portal.a as usize], ly.pts[portal.b as usize]);
                let tv = ly.tris[st.tri as usize];
                let third = tv
                    .iter()
                    .copied()
                    .find(|&v| v != portal.a && v != portal.b)
                    .unwrap_or(portal.a);
                let side = orient(pa, pb, ly.pts[third as usize]);
                if side > 0.0 {
                    portals.push((hi, lo));
                    pivots.push((pb, pa));
                } else {
                    portals.push((lo, hi));
                    pivots.push((pa, pb));
                }
            }
        }
        let last = j == steps.len();
        let end = match steps[j - 1].to {
            End::Vertex(v) => match ly.owner[v as usize] {
                Owner::Site(s) => {
                    if !last {
                        vias.push(RoutedVia {
                            net: net.clone(),
                            at: site_at(s),
                            via: c.via_name.clone(),
                        });
                    }
                    site_at(s)
                }
                _ if last => match wire.end_site {
                    Some(s) => site_at(s),
                    None => ly.pts[v as usize],
                },
                _ => ly.pts[v as usize],
            },
            End::Cross(_) => c.b,
        };
        let mut pts: Vec<P> = Vec::new();
        for q in round_corners(&funnel(start, &portals, end), &portals, &pivots) {
            if q[0].is_finite()
                && q[1].is_finite()
                && pts.last().is_none_or(|l| geom::dist(*l, q) > 1e-4)
            {
                pts.push(q);
            }
        }
        if pts.len() < 2 {
            pts = vec![start, end];
        }
        let pts = simplify(&pts);
        let neck = |q: P| -> f64 {
            let narrow = router.obs[layer as usize]
                .iter()
                .filter(|o| {
                    o.net == Some(c.net) && !o.poly.is_empty() && geom::point_in_polygon(q, &o.poly)
                })
                .map(|o| {
                    let mut bb = Bounds::EMPTY;
                    o.poly.iter().for_each(|v| bb.add(*v));
                    bb.size()[0].min(bb.size()[1])
                })
                .fold(f64::MAX, f64::min);
            c.width.min(narrow)
        };
        if let Some(q) = lead.take() {
            tracks.push(RoutedTrack {
                net: net.clone(),
                layer: ly.name.clone(),
                width: Some(neck(q)),
                points: vec![q, pts[0]],
            });
        }
        if last && wire.end_site.is_none() {
            tracks.push(RoutedTrack {
                net: net.clone(),
                layer: ly.name.clone(),
                width: Some(neck(c.b)),
                points: vec![*pts.last().unwrap(), c.b],
            });
        }
        tracks.push(RoutedTrack {
            net: net.clone(),
            layer: ly.name.clone(),
            width: None,
            points: pts,
        });
        start = end;
        i = j;
    }
    (tracks, vias)
}

fn round_corners(path: &[P], portals: &[(P, P)], pivots: &[(P, P)]) -> Vec<P> {
    let pivot_of = |q: P| -> Option<P> {
        portals.iter().zip(pivots).find_map(|((l, r), (pl, pr))| {
            if geom::dist(*l, q) < 1e-9 {
                Some(*pl)
            } else if geom::dist(*r, q) < 1e-9 {
                Some(*pr)
            } else {
                None
            }
        })
    };
    let mut out: Vec<P> = vec![path[0]];
    let mut k = 1;
    while k + 1 < path.len() {
        let Some(c) = pivot_of(path[k]) else {
            out.push(path[k]);
            k += 1;
            continue;
        };
        let mut last = k;
        let mut r = geom::dist(path[k], c);
        while last + 2 < path.len()
            && pivot_of(path[last + 1]).is_some_and(|c2| geom::dist(c2, c) < 1e-9)
        {
            last += 1;
            r = r.max(geom::dist(path[last], c));
        }
        let (p0, p2) = (*out.last().unwrap(), path[last + 1]);
        let turn = orient(p0, path[k], p2);
        let bend = {
            let (u, v) = (
                [path[k][0] - p0[0], path[k][1] - p0[1]],
                [p2[0] - path[last][0], p2[1] - path[last][1]],
            );
            let cos =
                (u[0] * v[0] + u[1] * v[1]) / ((u[0].hypot(u[1]) * v[0].hypot(v[1])).max(1e-12));
            cos.clamp(-1.0, 1.0).acos()
        };
        if bend < 0.2 {
            out.extend_from_slice(&path[k..=last]);
            k = last + 1;
            continue;
        }
        match (tangent(p0, c, r, turn, true), tangent(p2, c, r, turn, false)) {
            (Some(t1), Some(t2)) => {
                let a1 = (t1[1] - c[1]).atan2(t1[0] - c[0]);
                let mut a2 = (t2[1] - c[1]).atan2(t2[0] - c[0]);
                if turn > 0.0 {
                    while a2 < a1 {
                        a2 += std::f64::consts::TAU;
                    }
                } else {
                    while a2 > a1 {
                        a2 -= std::f64::consts::TAU;
                    }
                }
                if (a2 - a1).abs() > std::f64::consts::PI {
                    out.extend_from_slice(&path[k..=last]);
                    k = last + 1;
                    continue;
                }
                let n = (((a2 - a1).abs() / (std::f64::consts::PI / 6.0)).ceil() as usize).max(2);
                let half = ((a2 - a1).abs() / n as f64) / 2.0;
                let outer = r / half.cos();
                for m in 0..=n {
                    let a = a1 + (a2 - a1) * m as f64 / n as f64;
                    out.push([c[0] + outer * a.cos(), c[1] + outer * a.sin()]);
                }
            }
            _ => out.extend_from_slice(&path[k..=last]),
        }
        k = last + 1;
    }
    out.push(*path.last().unwrap());
    out
}

fn tangent(p: P, c: P, r: f64, turn: f64, incoming: bool) -> Option<P> {
    let d = geom::dist(p, c);
    if d <= r + 1e-9 {
        return None;
    }
    let base = (p[1] - c[1]).atan2(p[0] - c[0]);
    let th = (r / d).acos();
    let sign = if (turn > 0.0) == incoming { 1.0 } else { -1.0 };
    let a = base + sign * th;
    Some([c[0] + r * a.cos(), c[1] + r * a.sin()])
}

fn simplify(pts: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for &q in pts {
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let off = geom::point_segment_distance(b, a, q);
            if off < 0.003 {
                out.pop();
            } else {
                break;
            }
        }
        out.push(q);
    }
    out
}

fn route_one(router: &Router, c: &Conn, via_cost: f64, soft: bool) -> Option<Path> {
    let mut blocked: Vec<u32> = Vec::new();
    for _ in 0..8 {
        let path = search(router, c, via_cost, soft, &blocked)?;
        let used: Vec<u32> = path
            .sites
            .iter()
            .chain(path.start_site.iter())
            .chain(path.end_site.iter())
            .copied()
            .collect();
        let gap = 2.0 * c.via_radius + c.clear;
        let clash = used.iter().enumerate().find_map(|(i, &a)| {
            used[i + 1..].iter().find(|&&b| {
                a != b && geom::dist(router.sites[a as usize].at, router.sites[b as usize].at) < gap
            })
        });
        match clash {
            Some(&b) => blocked.push(b),
            None => return Some(path),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn funnel_bends_round_a_portal_above_the_line() {
        let path = funnel([0.0, 0.0], &[([5.0, 3.0], [5.0, 1.0])], [10.0, 0.0]);
        assert_eq!(path, vec![[0.0, 0.0], [5.0, 1.0], [10.0, 0.0]]);
    }

    #[test]
    fn corners_round_off_around_their_pivot() {
        let path = vec![[0.0, 0.0], [5.0, 1.0], [10.0, 0.0]];
        let portals = vec![([5.0, 3.0], [5.0, 1.0])];
        let pivots = vec![([5.0, 4.0], [5.0, 0.0])];
        let out = round_corners(&path, &portals, &pivots);
        assert!(out.len() > 3);
        for w in out.windows(2) {
            let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            assert!(geom::dist(mid, [5.0, 0.0]) > 0.99, "{mid:?}");
        }
        assert_eq!(out[0], [0.0, 0.0]);
        assert_eq!(*out.last().unwrap(), [10.0, 0.0]);
    }

    #[test]
    fn left_turns_round_off_too() {
        let path = vec![[0.0, 0.0], [5.0, -1.0], [10.0, 0.0]];
        let portals = vec![([5.0, -1.0], [5.0, -3.0])];
        let pivots = vec![([5.0, 0.0], [5.0, -4.0])];
        let out = round_corners(&path, &portals, &pivots);
        assert!(out.len() > 3);
        for w in out.windows(2) {
            let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            assert!(geom::dist(mid, [5.0, 0.0]) > 0.99, "{mid:?}");
        }
    }

    #[test]
    fn funnel_goes_straight_when_portals_allow() {
        let path = funnel([0.0, 0.0], &[([5.0, 1.0], [5.0, -1.0])], [10.0, 0.0]);
        assert_eq!(path, vec![[0.0, 0.0], [10.0, 0.0]]);
    }
}

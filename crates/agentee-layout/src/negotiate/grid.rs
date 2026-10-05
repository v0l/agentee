use super::rules::{Need, Rules, um};
use agentee_core::footprint::PadKind;
use agentee_core::geom::{self, P};
use agentee_core::layout::{Layout, ViaSource};

pub const NONE: u16 = u16::MAX - 1;
const EMPTY: u16 = u16::MAX;

#[derive(Clone, Copy)]
pub struct Near {
    a: u16,
    b: u16,
    net: u16,
}

impl Near {
    const FAR: Near = Near { a: u16::MAX, b: u16::MAX, net: EMPTY };
    const WALL: Near = Near { a: 0, b: 0, net: NONE };

    fn put(&mut self, v: u16, net: u16) {
        if net == self.net && net != NONE {
            self.a = self.a.min(v);
        } else if v < self.a {
            self.b = self.a;
            self.a = v;
            self.net = net;
        } else {
            self.b = self.b.min(v);
        }
    }

    #[inline]
    pub fn foreign(&self, net: u16) -> u16 {
        if self.net == net { self.b } else { self.a }
    }
}

#[derive(Clone, Debug)]
pub enum Shape {
    Poly(Vec<P>),
    Seg(P, P, f64),
    Circle(P, f64),
}

impl Shape {
    pub fn dist(&self, p: P) -> f64 {
        match self {
            Shape::Poly(v) => {
                if geom::point_in_polygon(p, v) {
                    return 0.0;
                }
                edge_dist(v, p)
            }
            Shape::Seg(a, b, r) => (geom::point_segment_distance(p, *a, *b) - r).max(0.0),
            Shape::Circle(c, r) => (geom::dist(p, *c) - r).max(0.0),
        }
    }

    pub fn bounds(&self) -> (P, P) {
        match self {
            Shape::Poly(v) => {
                let mut lo = [f64::MAX; 2];
                let mut hi = [f64::MIN; 2];
                for q in v {
                    lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                    hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                }
                (lo, hi)
            }
            Shape::Seg(a, b, r) => {
                ([a[0].min(b[0]) - r, a[1].min(b[1]) - r], [a[0].max(b[0]) + r, a[1].max(b[1]) + r])
            }
            Shape::Circle(c, r) => ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]),
        }
    }
}

fn ball_phase(layout: &Layout, g: f64) -> P {
    let wrap = |v: f64| v.rem_euclid(g);
    layout
        .parts
        .iter()
        .filter(|p| crate::escape::is_bga(p))
        .max_by(|a, b| {
            let pa = crate::escape::pitch_of(a);
            let pb = crate::escape::pitch_of(b);
            pb.total_cmp(&pa).then(a.pads.len().cmp(&b.pads.len()))
        })
        .and_then(|p| {
            let mut b = agentee_core::graphic::Bounds::EMPTY;
            p.pads.first()?.outlines.iter().flatten().for_each(|q| b.add(*q));
            let c = b.center();
            Some([wrap(c[0]), wrap(c[1])])
        })
        .unwrap_or([g / 2.0, g / 2.0])
}

pub fn edge_dist(v: &[P], p: P) -> f64 {
    (0..v.len())
        .map(|i| geom::point_segment_distance(p, v[i], v[(i + 1) % v.len()]))
        .fold(f64::MAX, f64::min)
}

pub fn poly_centre(v: &[P]) -> P {
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    for q in v {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
}

pub fn pad_gap(pad: &[P], p: P) -> f64 {
    let e = edge_dist(pad, p);
    if geom::point_in_polygon(p, pad) { -e } else { e }
}

pub fn touch(a: &Shape, b: &Shape) -> bool {
    let edges = |v: &Vec<P>| -> Vec<Shape> {
        (0..v.len()).map(|i| Shape::Seg(v[i], v[(i + 1) % v.len()], 0.0)).collect()
    };
    let point = |s: &Shape| match s {
        Shape::Poly(v) => v[0],
        Shape::Seg(a, _, _) => *a,
        Shape::Circle(c, _) => *c,
    };
    match (a, b) {
        (Shape::Poly(v), other) | (other, Shape::Poly(v)) => {
            geom::point_in_polygon(point(other), v)
                || matches!(other, Shape::Poly(w) if geom::point_in_polygon(w[0], v))
                || edges(v).iter().any(|e| touch(e, other))
        }
        (Shape::Circle(c, r), Shape::Circle(d, q)) => geom::dist(*c, *d) <= r + q + 1e-6,
        (Shape::Seg(a0, a1, r), Shape::Circle(c, q))
        | (Shape::Circle(c, q), Shape::Seg(a0, a1, r)) => {
            geom::point_segment_distance(*c, *a0, *a1) <= r + q + 1e-6
        }
        (Shape::Seg(a0, a1, r), Shape::Seg(b0, b1, q)) => {
            geom::segment_segment_distance(*a0, *a1, *b0, *b1) <= r + q + 1e-6
        }
    }
}

pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub g: f64,
    pub w: usize,
    pub h: usize,
    pub nl: usize,
    d: Vec<Near>,
    q: Vec<Near>,
    hole: Vec<u16>,
    via_block: Vec<u32>,
    fence: Vec<u16>,
    fence_nets: Vec<Vec<u16>>,
    bin: f64,
    bw: usize,
    bh: usize,
    bins: Vec<Vec<u32>>,
    obstacles: Vec<Obstacle>,
}

struct Obstacle {
    shape: Shape,
    layers: u64,
    net: u16,
    clr: f64,
    edge: bool,
}

pub struct Fence {
    pub nets: Vec<usize>,
    pub lo: P,
    pub hi: P,
}

impl Grid {
    pub fn plane(&self) -> usize {
        self.w * self.h
    }

    #[inline]
    pub fn idx(&self, l: usize, x: usize, y: usize) -> usize {
        (l * self.h + y) * self.w + x
    }

    pub fn cell(&self, p: P) -> (i64, i64) {
        (((p[0] - self.x0) / self.g).floor() as i64, ((p[1] - self.y0) / self.g).floor() as i64)
    }

    pub fn inside(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    #[inline]
    pub fn center(&self, x: usize, y: usize) -> P {
        [self.x0 + (x as f64 + 0.5) * self.g, self.y0 + (y as f64 + 0.5) * self.g]
    }

    pub fn span(&self, lo: P, hi: P, reach: f64) -> (usize, usize, usize, usize) {
        let (x0, y0) = self.cell([lo[0] - reach, lo[1] - reach]);
        let (x1, y1) = self.cell([hi[0] + reach, hi[1] + reach]);
        (
            x0.max(0) as usize,
            y0.max(0) as usize,
            x1.min(self.w as i64 - 1).max(-1) as usize,
            y1.min(self.h as i64 - 1).max(-1) as usize,
        )
    }

    pub fn near(&self, shape: &Shape, reach: f64, mut f: impl FnMut(usize, usize, f64)) {
        let (lo, hi) = shape.bounds();
        let (x0, y0, x1, y1) = self.span(lo, hi, reach);
        if x1 == usize::MAX || y1 == usize::MAX {
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = shape.dist(self.center(x, y));
                if d <= reach {
                    f(x, y, d);
                }
            }
        }
    }

    fn record(&mut self, shape: &Shape, layers: u64, net: u16, clr: f64, edge: bool, reach: f64) {
        let (lo, hi) = shape.bounds();
        let k = self.obstacles.len() as u32;
        let b =
            |v: f64, o: f64, n: usize| (((v - o) / self.bin).floor().max(0.0) as usize).min(n - 1);
        let (x0, x1) = (b(lo[0] - reach, self.x0, self.bw), b(hi[0] + reach, self.x0, self.bw));
        let (y0, y1) = (b(lo[1] - reach, self.y0, self.bh), b(hi[1] + reach, self.y0, self.bh));
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.bins[y * self.bw + x].push(k);
            }
        }
        self.obstacles.push(Obstacle { shape: shape.clone(), layers, net, clr, edge });
    }

    pub fn exact(&self, l: usize, p: P, net: u16) -> (f64, f64) {
        let (cx, cy) = self.cell(p);
        if !self.inside(cx, cy) {
            return (0.0, 0.0);
        }
        let i = self.idx(l, cx as usize, cy as usize);
        if self.d[i].a == 0 && self.d[i].net == NONE && self.q[i].a == 0 {
            return (0.0, 0.0);
        }
        let bx = (((p[0] - self.x0) / self.bin) as usize).min(self.bw - 1);
        let by = (((p[1] - self.y0) / self.bin) as usize).min(self.bh - 1);
        let (mut d, mut q) = (1e3f64, 1e3f64);
        for &k in &self.bins[by * self.bw + bx] {
            let o = &self.obstacles[k as usize];
            if o.layers & (1 << l) == 0 || (o.net == net && !o.edge) {
                continue;
            }
            let v = o.shape.dist(p);
            q = q.min(v - o.clr);
            if !o.edge {
                d = d.min(v);
            }
        }
        (d, q.max(0.0))
    }

    fn stamp(&mut self, shape: &Shape, layers: &[usize], net: u16, clr: f64, reach: f64) {
        let mask = layers.iter().fold(0u64, |m, &l| m | 1 << l);
        self.record(shape, mask, net, clr, false, reach);
        let plane = self.plane();
        let mut hits = Vec::new();
        self.near(shape, reach, |x, y, d| hits.push((y * self.w + x, d)));
        for (c, d) in hits {
            for &l in layers {
                let i = l * plane + c;
                self.d[i].put(um(d), net);
                self.q[i].put(um((d - clr).max(0.0)), net);
            }
        }
    }

    fn stamp_edge(&mut self, shape: &Shape, clr: f64, reach: f64) {
        self.record(shape, u64::MAX, NONE, clr, true, reach);
        let plane = self.plane();
        let mut hits = Vec::new();
        self.near(shape, reach, |x, y, d| hits.push((y * self.w + x, d)));
        for (c, d) in hits {
            for l in 0..self.nl {
                self.q[l * plane + c].put(um((d - clr).max(0.0)), NONE);
            }
        }
    }

    fn stamp_hole(&mut self, c: P, dr: f64, reach: f64) {
        let mut hits = Vec::new();
        self.near(&Shape::Circle(c, dr), reach, |x, y, d| hits.push((y * self.w + x, d)));
        for (k, d) in hits {
            self.hole[k] = self.hole[k].min(um(d));
        }
    }

    pub fn build(layout: &Layout, rules: &Rules, g: f64, fences: &[Fence]) -> Grid {
        let nl = layout.copper.len();
        let b = layout.bounds();
        let snap = |v: f64| ((v / g).floor() - 2.0) * g;
        let phase = ball_phase(layout, g);
        let (x0, y0) = (snap(b.min[0]) + phase[0] - g / 2.0, snap(b.min[1]) + phase[1] - g / 2.0);
        let w = ((b.max[0] - x0) / g).ceil() as usize + 3;
        let h = ((b.max[1] - y0) / g).ceil() as usize + 3;
        let mut grid = Grid {
            x0,
            y0,
            g,
            w,
            h,
            nl,
            d: vec![Near::FAR; nl * w * h],
            q: vec![Near::FAR; nl * w * h],
            hole: vec![u16::MAX; w * h],
            via_block: vec![0; w * h],
            fence: vec![0; w * h],
            fence_nets: Vec::new(),
            bin: rules.reach.max(0.5),
            bw: ((w as f64 * g) / rules.reach.max(0.5)).ceil() as usize + 1,
            bh: ((h as f64 * g) / rules.reach.max(0.5)).ceil() as usize + 1,
            bins: Vec::new(),
            obstacles: Vec::new(),
        };
        grid.bins = vec![Vec::new(); grid.bw * grid.bh];
        let layer_of = |n: &str| layout.copper.iter().position(|c| c == n);
        let reach = rules.reach;
        let board = layout.edge();
        let edges: Vec<(P, P)> =
            if board.is_closed() { board.segments().collect() } else { Vec::new() };
        if !edges.is_empty() {
            for y in 0..h {
                let cy = grid.center(0, y)[1];
                let mut xs: Vec<f64> = edges
                    .iter()
                    .filter(|(a, b)| (a[1] <= cy) != (b[1] <= cy))
                    .map(|&(a, b)| a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
                    .collect();
                xs.sort_by(f64::total_cmp);
                for x in 0..w {
                    let cx = grid.center(x, y)[0];
                    if xs.iter().filter(|&&e| e < cx).count() % 2 == 0 {
                        for l in 0..nl {
                            let i = grid.idx(l, x, y);
                            grid.d[i] = Near::WALL;
                            grid.q[i] = Near::WALL;
                        }
                        grid.via_block[y * w + x] = u32::MAX;
                    }
                }
            }
            for &(a, b) in &edges {
                grid.stamp_edge(&Shape::Seg(a, b, 0.0), rules.edge, reach);
            }
        }
        let net_id = |n: Option<usize>| n.map(|n| n as u16).unwrap_or(NONE);
        let hole_reach =
            rules.vias.iter().map(|o| o.dr).fold(0.0, f64::max) + rules.hole_gap + rules.slack + g;
        for part in &layout.parts {
            for pad in &part.pads {
                let layers: Vec<usize> = pad.copper.iter().filter_map(|c| layer_of(c)).collect();
                let clr = pad.net.map(|n| layout.nets[n].clearance).unwrap_or(0.0);
                if !layers.is_empty() {
                    for o in &pad.outlines {
                        grid.stamp(&Shape::Poly(o.clone()), &layers, net_id(pad.net), clr, reach);
                    }
                }
                if let Some((c, s, _)) = pad.drill {
                    let hole = Shape::Circle(c, s[0].max(s[1]) / 2.0);
                    if pad.kind == PadKind::Npth || layers.is_empty() {
                        let all: Vec<usize> = (0..nl).collect();
                        grid.stamp(&hole, &all, NONE, rules.npth, reach);
                    } else {
                        let outer = [0, nl - 1];
                        let inner: Vec<usize> = (1..nl.saturating_sub(1)).collect();
                        grid.stamp(&hole, &outer, net_id(pad.net), rules.pth_cu, reach);
                        grid.stamp(&hole, &inner, net_id(pad.net), rules.inner_pth_cu, reach);
                    }
                    grid.stamp_hole(c, s[0].min(s[1]) / 2.0, hole_reach);
                }
            }
        }
        for t in &layout.tracks {
            let Some(l) = layer_of(&t.layer) else { continue };
            let clr = layout.nets[t.net].clearance;
            for s in t.points.windows(2) {
                grid.stamp(&Shape::Seg(s[0], s[1], t.width / 2.0), &[l], t.net as u16, clr, reach);
            }
        }
        for v in layout.vias.iter().filter(|v| !matches!(v.source, ViaSource::Stitch(_))) {
            let layers: Vec<usize> = v.layers.iter().filter_map(|c| layer_of(c)).collect();
            let clr =
                layout.nets[v.net].clearance.max(v.drill / 2.0 + rules.hole_cu - v.diameter / 2.0);
            grid.stamp(&Shape::Circle(v.at, v.diameter / 2.0), &layers, v.net as u16, clr, reach);
            grid.stamp_hole(v.at, v.drill / 2.0, hole_reach);
        }
        for z in layout.zones.iter().filter(|_| std::env::var("AGENTEE_POUR").is_ok()) {
            let Some(l) = layer_of(&z.layer) else { continue };
            let clr = layout.nets[z.net].clearance;
            for r in z.rings.iter().filter(|r| r.len() >= 3) {
                for k in 0..r.len() {
                    grid.stamp(
                        &Shape::Seg(r[k], r[(k + 1) % r.len()], 0.0),
                        &[l],
                        NONE,
                        clr,
                        reach,
                    );
                }
            }
        }
        grid.block_smd(layout, rules);
        grid.block_silk(layout, rules);
        for f in fences {
            grid.fence_nets.push(f.nets.iter().map(|&n| n as u16).collect());
            let id = grid.fence_nets.len() as u16;
            let (x0, y0, x1, y1) = grid.span(f.lo, f.hi, 0.0);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let c = grid.center(x, y);
                    if c[0] >= f.lo[0] && c[0] <= f.hi[0] && c[1] >= f.lo[1] && c[1] <= f.hi[1] {
                        grid.fence[y * w + x] = id;
                    }
                }
            }
        }
        grid
    }

    fn block_smd(&mut self, layout: &Layout, rules: &Rules) {
        let layer_of = |n: &str| layout.copper.iter().position(|c| c == n);
        for (k, via) in rules.vias.iter().enumerate() {
            let reach = via.r.max(via.dr + rules.hole_smd) + 1e-3 + rules.slack;
            for part in &layout.parts {
                for pad in part.pads.iter().filter(|p| p.drill.is_none()) {
                    if !pad
                        .copper
                        .iter()
                        .filter_map(|c| layer_of(c))
                        .any(|l| via.layers.contains(&l))
                    {
                        continue;
                    }
                    for o in &pad.outlines {
                        let centre = poly_centre(o);
                        let inscribed = if geom::point_in_polygon(centre, o) {
                            edge_dist(o, centre)
                        } else {
                            0.0
                        };
                        let site =
                            (via.in_pad && inscribed >= via.r + 1e-3).then(|| self.cell(centre));
                        let mut hits = Vec::new();
                        self.near(&Shape::Poly(o.clone()), reach, |x, y, _| hits.push((x, y)));
                        for (x, y) in hits {
                            if site == Some((x as i64, y as i64)) {
                                continue;
                            }
                            let gap = pad_gap(o, self.center(x, y));
                            let fully_in = gap <= -(via.r + 1e-3);
                            if (fully_in && !via.in_pad) || (!fully_in && gap < reach) {
                                self.via_block[y * self.w + x] |= 1 << k;
                            }
                        }
                    }
                }
            }
        }
    }

    fn block_silk(&mut self, layout: &Layout, rules: &Rules) {
        let bottom = self.nl - 1;
        for (k, via) in rules.vias.iter().enumerate() {
            let reach = via.r + self.g / 2.0;
            for b in layout.silk.iter().filter(|b| b.outline.len() >= 3) {
                let outer = if b.layer.starts_with("B.") { bottom } else { 0 };
                if !via.layers.contains(&outer) {
                    continue;
                }
                let mut hits = Vec::new();
                self.near(&Shape::Poly(b.outline.clone()), reach, |x, y, _| hits.push((x, y)));
                for (x, y) in hits {
                    self.via_block[y * self.w + x] |= 1 << k;
                }
            }
        }
    }

    #[inline]
    pub fn fence_ok(&self, c2: usize, net: u16) -> bool {
        let f = self.fence[c2];
        f == 0 || self.fence_nets[f as usize - 1].contains(&net)
    }

    #[inline]
    pub fn track_ok(&self, i: usize, net: u16, need: Need) -> bool {
        self.d[i].foreign(net) > need.d
            && self.q[i].foreign(net) > need.q
            && self.fence_ok(i % self.plane(), net)
    }

    #[inline]
    pub fn via_ok(&self, c2: usize, layers: &[usize], k: usize, net: u16, need: Need) -> bool {
        if self.via_block[c2] & (1 << k) != 0
            || self.hole[c2] <= need.hole
            || !self.fence_ok(c2, net)
        {
            return false;
        }
        let plane = self.plane();
        layers.iter().all(|&l| {
            let i = l * plane + c2;
            self.d[i].foreign(net) > need.d && self.q[i].foreign(net) > need.q
        })
    }

    #[inline]
    pub fn open(&self, i: usize, need: Need) -> bool {
        self.d[i].a > need.d && self.q[i].a > need.q
    }

    pub fn via_open(&self, c2: usize, layers: &[usize], k: usize, need: Need) -> bool {
        if self.via_block[c2] & (1 << k) != 0 || self.hole[c2] <= need.hole {
            return false;
        }
        let plane = self.plane();
        layers.iter().all(|&l| self.open(l * plane + c2, need))
    }

    pub fn clearance_at(&self, i: usize, net: u16) -> (f64, f64) {
        (self.d[i].foreign(net) as f64 / 1000.0, self.q[i].foreign(net) as f64 / 1000.0)
    }
}

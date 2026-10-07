use crate::board::Board;
use crate::diag::{Diags, Severity};
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::graphic::Graphic;
use crate::interface::Interface;
use crate::layout::{Layout, LayoutNet, MatchGroup, Pair, Placed, PlacedPad, Track, Via, ZoneFill};
use serde::Serialize;
use std::cell::OnceCell;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Copper,
    Drill,
    Mask,
    Silk,
    Assembly,
    Zone,
    Signal,
    Test,
    Placement,
}

pub struct Rule {
    pub id: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub summary: &'static str,
    pub when: &'static str,
    pub applies: fn(&Setup) -> bool,
    pub check: fn(&Ctx, &mut Report),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Setup {
    pub fab: String,
    pub copper_layers: usize,
    pub thickness_mm: f64,
    pub finish: String,
    pub outer_oz: f64,
    pub inner_oz: Option<f64>,
    pub blind_buried: bool,
    pub impedance: bool,
    pub layout: bool,
    pub via_in_pad: bool,
    pub parts: usize,
    pub bottom_parts: bool,
    pub zones: bool,
    pub npth: bool,
    pub slots: bool,
    pub bga: bool,
    pub pairs: bool,
    pub match_groups: bool,
    pub interfaces: bool,
    pub mlcc: bool,
    pub small_chips: bool,
    pub tall_parts: bool,
    pub test_points: bool,
    pub domains: bool,
    pub barrier_clearance: bool,
    pub creepage: bool,
    pub spark_gaps: bool,
    pub title: bool,
}

impl Setup {
    pub fn of_board(board: &Board) -> Setup {
        let copper = board.stackup.copper_names();
        let inner: Vec<f64> =
            board.stackup.copper().map(|(_, l)| l.thickness.to_mm() / 0.035).skip(1).collect();
        Setup {
            fab: board.fab.clone(),
            copper_layers: copper.len(),
            thickness_mm: board.stackup.thickness().to_mm(),
            finish: board.stackup.finish.clone(),
            outer_oz: board.stackup.outer_oz(),
            inner_oz: inner.split_last().and_then(|(_, v)| v.iter().copied().reduce(f64::max)),
            blind_buried: board.vias.iter().any(|v| v.kind != crate::board::ViaKind::Through),
            impedance: board.netclasses.iter().any(|n| n.impedance.is_some()),
            domains: board.domains.iter().any(|d| !d.implicit),
            barrier_clearance: board.barriers.iter().any(|b| b.clearance.is_some()),
            creepage: board.barriers.iter().any(|b| b.creepage.is_some()),
            ..Setup::default()
        }
    }

    pub fn of(cx: &Ctx) -> Setup {
        let pads = || cx.parts.iter().flat_map(|p| p.pads.iter());
        Setup {
            layout: true,
            via_in_pad: !vias_in_pads(cx.parts, cx.vias).is_empty(),
            parts: cx.parts.len(),
            bottom_parts: cx.parts.iter().any(|p| p.bottom),
            zones: !cx.zones.is_empty(),
            npth: pads().any(|q| q.kind == PadKind::Npth),
            slots: pads().any(|q| q.drill.is_some_and(|(_, s, _)| (s[0] - s[1]).abs() > 1e-6)),
            bga: cx.parts.iter().any(|p| assembly::bga_pitch(p).is_some()),
            pairs: !cx.pairs.is_empty(),
            match_groups: !cx.match_groups.is_empty(),
            interfaces: !cx.interfaces.is_empty(),
            mlcc: !mechanical::mlcc_chips(cx.parts).is_empty(),
            small_chips: mechanical::has_small_chips(cx.parts),
            tall_parts: mechanical::has_tall_parts(cx.parts),
            test_points: cx.parts.iter().any(crate::testpoint::is_test_point),
            spark_gaps: cx.parts.iter().any(|p| !p.footprint.spark_gaps.is_empty()),
            title: cx.title,
            ..Setup::of_board(cx.board)
        }
    }
}

pub struct Ctx<'a> {
    pub board: &'a Board,
    pub copper: &'a [String],
    pub outline: &'a [P],
    pub board_cutouts: &'a [Vec<P>],
    pub parts: &'a [Placed],
    pub tracks: &'a [Track],
    pub vias: &'a [Via],
    pub zones: &'a [ZoneFill],
    pub nets: &'a [LayoutNet],
    pub graphics: &'a [Graphic],
    pub pairs: &'a [Pair],
    pub match_groups: &'a [MatchGroup],
    pub interfaces: &'a [Interface],
    pub test: Option<&'a crate::testpoint::TestSpec>,
    pub heat: &'a [(String, f64)],
    pub title: bool,
    found: &'a [Finding],
    items: OnceCell<Vec<Cu>>,
    spacing: OnceCell<crate::rules::Spacings>,
    grid: OnceCell<HashMap<(i64, i64), Vec<usize>>>,
    fills: OnceCell<Vec<FillIndex>>,
}

const GRID: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoleOf {
    Via(usize),
    Pad(usize, usize),
}

pub struct Hole {
    pub of: HoleOf,
    pub a: P,
    pub b: P,
    pub r: f64,
    pub size: [f64; 2],
    pub plated: bool,
    pub net: Option<usize>,
    pub layers: Vec<String>,
}

impl Hole {
    pub fn slot(&self) -> bool {
        (self.size[0] - self.size[1]).abs() > 1e-6
    }

    pub fn gap_to(&self, c: &CuShape) -> f64 {
        match c {
            CuShape::Poly(v) => {
                let d = v
                    .iter()
                    .map(|o| geom::polyline_polygon_distance(&[self.a, self.b], o))
                    .fold(f64::MAX, f64::min);
                d - self.r
            }
            CuShape::Seg(p, q, hw) => {
                geom::segment_segment_distance(self.a, self.b, *p, *q) - hw - self.r
            }
            CuShape::Circle(o, ro) => {
                geom::point_segment_distance(*o, self.a, self.b) - ro - self.r
            }
        }
    }

    pub fn gap_to_fill(&self, f: &FillIndex, reach: f64) -> f64 {
        let mid = [(self.a[0] + self.b[0]) / 2.0, (self.a[1] + self.b[1]) / 2.0];
        [self.a, mid, self.b]
            .into_iter()
            .map(|c| f.circle_gap(c, self.r, reach))
            .fold(f64::MAX, f64::min)
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        b.add_circle(self.a, self.r);
        b.add_circle(self.b, self.r);
        b
    }
}

impl<'a> Ctx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        board: &'a Board,
        copper: &'a [String],
        outline: &'a [P],
        board_cutouts: &'a [Vec<P>],
        parts: &'a [Placed],
        tracks: &'a [Track],
        vias: &'a [Via],
        zones: &'a [ZoneFill],
        nets: &'a [LayoutNet],
    ) -> Ctx<'a> {
        Ctx {
            board,
            copper,
            outline,
            board_cutouts,
            parts,
            tracks,
            vias,
            zones,
            nets,
            graphics: &[],
            pairs: &[],
            match_groups: &[],
            interfaces: &[],
            test: None,
            heat: &[],
            title: false,
            found: &[],
            items: OnceCell::new(),
            spacing: OnceCell::new(),
            grid: OnceCell::new(),
            fills: OnceCell::new(),
        }
    }

    pub fn of_layout(board: &'a Board, l: &'a Layout) -> Ctx<'a> {
        Ctx::new(
            board,
            &l.copper,
            &l.outline,
            &l.board_cutouts,
            &l.parts,
            &l.tracks,
            &l.vias,
            &l.zones,
            &l.nets,
        )
        .with_signals(&l.graphics, &l.pairs, &l.match_groups, &l.interfaces)
        .with_test(&l.test)
        .with_title(l.title.is_some() || l.title_problem.is_some())
    }

    pub fn with_title(mut self, title: bool) -> Ctx<'a> {
        self.title = title;
        self
    }

    pub fn edge(&self) -> geom::BoardEdge<'a> {
        geom::BoardEdge::new(self.outline, self.board_cutouts)
    }

    pub fn with_test(mut self, test: &'a crate::testpoint::TestSpec) -> Ctx<'a> {
        self.test = Some(test);
        self
    }

    pub fn with_signals(
        mut self,
        graphics: &'a [Graphic],
        pairs: &'a [Pair],
        match_groups: &'a [MatchGroup],
        interfaces: &'a [Interface],
    ) -> Ctx<'a> {
        self.graphics = graphics;
        self.pairs = pairs;
        self.match_groups = match_groups;
        self.interfaces = interfaces;
        self
    }

    pub fn with_heat(mut self, heat: &'a [(String, f64)]) -> Ctx<'a> {
        self.heat = heat;
        self
    }

    pub fn with_found(mut self, found: &'a Findings) -> Ctx<'a> {
        self.found = &found.0;
        self
    }

    pub fn net_name(&self, n: Option<usize>) -> &str {
        n.and_then(|n| self.nets.get(n)).map(|n| n.name.as_str()).unwrap_or("no net")
    }

    pub fn pad_name(&self, part: usize, pad: usize) -> String {
        format!("{}.{}", self.parts[part].reference, self.parts[part].pads[pad].number)
    }

    pub fn spacing(&self) -> &crate::rules::Spacings {
        self.spacing
            .get_or_init(|| crate::rules::Spacings::new(self.board, self.nets, self.copper.len()))
    }

    pub fn copper_items(&self) -> &[Cu] {
        self.items.get_or_init(|| {
            let mut out = Vec::new();
            for (pi, p) in self.parts.iter().enumerate() {
                for (k, q) in p.pads.iter().enumerate() {
                    if q.copper.is_empty() {
                        continue;
                    }
                    out.push(Cu {
                        owner: Owner::Pad(pi, k),
                        net: q.net,
                        layers: q.copper.clone(),
                        bounds: rings_bounds(&q.outlines),
                        shape: CuShape::Poly(q.outlines.clone()),
                    });
                }
            }
            for (ti, t) in self.tracks.iter().enumerate() {
                for w in t.points.windows(2) {
                    let mut b = Bounds::EMPTY;
                    b.add_circle(w[0], t.width / 2.0);
                    b.add_circle(w[1], t.width / 2.0);
                    out.push(Cu {
                        owner: Owner::Track(ti),
                        net: Some(t.net),
                        layers: vec![t.layer.clone()],
                        bounds: b,
                        shape: CuShape::Seg(w[0], w[1], t.width / 2.0),
                    });
                }
            }
            for (vi, v) in self.vias.iter().enumerate() {
                let mut b = Bounds::EMPTY;
                b.add_circle(v.at, v.diameter / 2.0);
                out.push(Cu {
                    owner: Owner::Via(vi),
                    net: Some(v.net),
                    layers: v.layers.clone(),
                    bounds: b,
                    shape: CuShape::Circle(v.at, v.diameter / 2.0),
                });
            }
            out
        })
    }

    pub fn items_near(&self, b: &Bounds, reach: f64) -> Vec<usize> {
        let items = self.copper_items();
        let grid = self.grid.get_or_init(|| {
            let mut g: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
            for (i, c) in items.iter().enumerate() {
                for x in (c.bounds.min[0] / GRID).floor() as i64
                    ..=(c.bounds.max[0] / GRID).floor() as i64
                {
                    for y in (c.bounds.min[1] / GRID).floor() as i64
                        ..=(c.bounds.max[1] / GRID).floor() as i64
                    {
                        g.entry((x, y)).or_default().push(i);
                    }
                }
            }
            g
        });
        let mut out = Vec::new();
        for x in
            ((b.min[0] - reach) / GRID).floor() as i64..=((b.max[0] + reach) / GRID).floor() as i64
        {
            for y in ((b.min[1] - reach) / GRID).floor() as i64
                ..=((b.max[1] + reach) / GRID).floor() as i64
            {
                out.extend(grid.get(&(x, y)).into_iter().flatten().copied());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn holes(&self) -> Vec<Hole> {
        let mut out: Vec<Hole> = self
            .vias
            .iter()
            .enumerate()
            .map(|(vi, v)| Hole {
                of: HoleOf::Via(vi),
                a: v.at,
                b: v.at,
                r: v.drill / 2.0,
                size: [v.drill, v.drill],
                plated: true,
                net: Some(v.net),
                layers: v.hole.clone(),
            })
            .collect();
        for (pi, p) in self.parts.iter().enumerate() {
            let t = p.transform();
            for (k, q) in p.pads.iter().enumerate() {
                let Some((c, size, _)) = q.drill else { continue };
                let rot = p.footprint.pads.get(k).map(|f| f.rotation).unwrap_or(0.0);
                let long = if size[0] >= size[1] { [1.0, 0.0] } else { [0.0, 1.0] };
                let u = t.direction(geom::rotate(long, rot));
                let half = (size[0] - size[1]).abs() / 2.0;
                out.push(Hole {
                    of: HoleOf::Pad(pi, k),
                    a: [c[0] - u[0] * half, c[1] - u[1] * half],
                    b: [c[0] + u[0] * half, c[1] + u[1] * half],
                    r: size[0].min(size[1]) / 2.0,
                    size,
                    plated: q.kind != PadKind::Npth,
                    net: q.net,
                    layers: self.copper.to_vec(),
                });
            }
        }
        out
    }

    pub fn hole_name(&self, h: &Hole) -> String {
        match h.of {
            HoleOf::Via(v) => format!(
                "via at [{:.3}, {:.3}] ({})",
                self.vias[v].at[0], self.vias[v].at[1], self.nets[self.vias[v].net].name
            ),
            HoleOf::Pad(p, k) if self.parts[p].pads[k].number.is_empty() => {
                format!("{} hole at [{:.3}, {:.3}]", self.parts[p].reference, h.a[0], h.a[1])
            }
            HoleOf::Pad(p, k) => self.pad_name(p, k),
        }
    }

    pub fn fills(&self) -> &[FillIndex] {
        self.fills.get_or_init(|| self.zones.iter().map(|z| FillIndex::new(&z.rings)).collect())
    }

    pub fn describe(&self, c: &Cu) -> String {
        match c.owner {
            Owner::Pad(p, k) => self.pad_name(p, k),
            Owner::Track(t) => format!("track {} ({})", t, self.nets[self.tracks[t].net].name),
            Owner::Via(v) => format!(
                "via at [{:.3}, {:.3}] ({})",
                self.vias[v].at[0], self.vias[v].at[1], self.nets[self.vias[v].net].name
            ),
            Owner::Copper(k) => format!("copper {k}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Owner {
    Pad(usize, usize),
    Track(usize),
    Via(usize),
    Copper(usize),
}

pub struct Cu {
    pub owner: Owner,
    pub net: Option<usize>,
    pub layers: Vec<String>,
    pub bounds: Bounds,
    pub shape: CuShape,
}

pub enum CuShape {
    Poly(Vec<Vec<P>>),
    Seg(P, P, f64),
    Circle(P, f64),
}

impl CuShape {
    pub fn point_distance(&self, p: P) -> f64 {
        match self {
            CuShape::Poly(v) => v
                .iter()
                .map(|poly| {
                    if geom::point_in_polygon(p, poly) {
                        0.0
                    } else {
                        geom::polyline_polygon_distance(&[p, p], poly)
                    }
                })
                .fold(f64::MAX, f64::min),
            CuShape::Seg(a, b, hw) => geom::point_segment_distance(p, *a, *b) - hw,
            CuShape::Circle(c, r) => geom::dist(p, *c) - r,
        }
    }

    pub fn distance(&self, o: &CuShape) -> f64 {
        match (self, o) {
            (CuShape::Seg(a, b, h1), CuShape::Seg(c, d, h2)) => {
                geom::segment_segment_distance(*a, *b, *c, *d) - h1 - h2
            }
            (CuShape::Seg(a, b, h), CuShape::Circle(c, r))
            | (CuShape::Circle(c, r), CuShape::Seg(a, b, h)) => {
                geom::point_segment_distance(*c, *a, *b) - h - r
            }
            (CuShape::Circle(a, r1), CuShape::Circle(b, r2)) => geom::dist(*a, *b) - r1 - r2,
            (CuShape::Poly(v), CuShape::Seg(a, b, h))
            | (CuShape::Seg(a, b, h), CuShape::Poly(v)) => v
                .iter()
                .map(|poly| {
                    if geom::point_in_polygon(*a, poly) || geom::point_in_polygon(*b, poly) {
                        -h
                    } else {
                        geom::polyline_polygon_distance(&[*a, *b], poly) - h
                    }
                })
                .fold(f64::MAX, f64::min),
            (CuShape::Poly(v), CuShape::Circle(c, r))
            | (CuShape::Circle(c, r), CuShape::Poly(v)) => {
                v.iter()
                    .map(|poly| {
                        if geom::point_in_polygon(*c, poly) {
                            0.0
                        } else {
                            geom::polyline_polygon_distance(&[*c, *c], poly)
                        }
                    })
                    .fold(f64::MAX, f64::min)
                    - r
            }
            (CuShape::Poly(a), CuShape::Poly(b)) => a
                .iter()
                .flat_map(|p| b.iter().map(move |q| geom::polygon_distance(p, q)))
                .fold(f64::MAX, f64::min),
        }
    }

    pub fn circle_gap(&self, c: P, r: f64) -> f64 {
        match self {
            CuShape::Poly(v) => rings_point_gap(v, c) - r,
            CuShape::Seg(a, b, hw) => geom::point_segment_distance(c, *a, *b) - hw - r,
            CuShape::Circle(o, ro) => geom::dist(c, *o) - ro - r,
        }
    }
}

pub fn rings_bounds(rings: &[Vec<P>]) -> Bounds {
    let mut b = Bounds::EMPTY;
    rings.iter().flatten().for_each(|p| b.add(*p));
    b
}

pub fn edge_distance(poly: &[P], p: P) -> f64 {
    (0..poly.len())
        .map(|i| geom::point_segment_distance(p, poly[i], poly[(i + 1) % poly.len()]))
        .fold(f64::MAX, f64::min)
}

pub fn rings_point_gap(rings: &[Vec<P>], p: P) -> f64 {
    if rings.iter().any(|r| geom::point_in_polygon(p, r)) {
        return 0.0;
    }
    rings.iter().map(|r| edge_distance(r, p)).fold(f64::MAX, f64::min)
}

pub fn near(b: &Bounds, p: P, reach: f64) -> bool {
    !b.is_empty()
        && p[0] >= b.min[0] - reach
        && p[0] <= b.max[0] + reach
        && p[1] >= b.min[1] - reach
        && p[1] <= b.max[1] + reach
}

pub fn is_smd(q: &PlacedPad) -> bool {
    q.drill.is_none() && !q.copper.is_empty()
}

pub struct PadRef<'a> {
    pub part: usize,
    pub pad: usize,
    pub q: &'a PlacedPad,
    pub bounds: Bounds,
}

pub fn pads_where<'a>(parts: &'a [Placed], keep: impl Fn(&PlacedPad) -> bool) -> Vec<PadRef<'a>> {
    parts
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| p.pads.iter().enumerate().map(move |(k, q)| (pi, k, q)))
        .filter(|(_, _, q)| keep(q))
        .map(|(part, pad, q)| PadRef { part, pad, q, bounds: rings_bounds(&q.outlines) })
        .collect()
}

pub fn outline_distance(outline: &[P], p: P) -> f64 {
    edge_distance(outline, p)
}

pub struct FillIndex {
    pub(crate) edges: Vec<(P, P)>,
    pub(crate) cell: f64,
    y0: f64,
    rows: Vec<Vec<usize>>,
    pub(crate) bins: HashMap<(i64, i64), Vec<usize>>,
    pub bounds: Bounds,
}

impl FillIndex {
    pub fn new(rings: &[Vec<P>]) -> FillIndex {
        let cell = 0.5;
        let bounds = rings_bounds(rings);
        let mut edges = Vec::new();
        for r in rings {
            for i in 0..r.len() {
                edges.push((r[i], r[(i + 1) % r.len()]));
            }
        }
        let y0 = if bounds.is_empty() { 0.0 } else { bounds.min[1] };
        let n_rows =
            if bounds.is_empty() { 0 } else { ((bounds.max[1] - y0) / cell).floor() as usize + 1 };
        let mut rows = vec![Vec::new(); n_rows];
        let mut bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (k, (a, b)) in edges.iter().enumerate() {
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            let (r0, r1) =
                (((lo - y0) / cell).floor() as usize, ((hi - y0) / cell).floor() as usize);
            for row in rows.iter_mut().take(r1.min(n_rows.saturating_sub(1)) + 1).skip(r0) {
                row.push(k);
            }
            let (x0, x1) =
                ((a[0].min(b[0]) / cell).floor() as i64, (a[0].max(b[0]) / cell).floor() as i64);
            let (y0b, y1b) = ((lo / cell).floor() as i64, (hi / cell).floor() as i64);
            for x in x0..=x1 {
                for y in y0b..=y1b {
                    bins.entry((x, y)).or_default().push(k);
                }
            }
        }
        FillIndex { edges, cell, y0, rows, bins, bounds }
    }

    pub fn contains(&self, p: P) -> bool {
        if !near(&self.bounds, p, 0.0) {
            return false;
        }
        let row = ((p[1] - self.y0) / self.cell).floor();
        if row < 0.0 || row as usize >= self.rows.len() {
            return false;
        }
        let mut wind = 0;
        for &k in &self.rows[row as usize] {
            let (a, b) = self.edges[k];
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    wind += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
        wind != 0
    }

    pub fn edge_gap(&self, p: P, reach: f64) -> f64 {
        let (x0, x1) = (
            ((p[0] - reach) / self.cell).floor() as i64,
            ((p[0] + reach) / self.cell).floor() as i64,
        );
        let (y0, y1) = (
            ((p[1] - reach) / self.cell).floor() as i64,
            ((p[1] + reach) / self.cell).floor() as i64,
        );
        let mut best = f64::MAX;
        for x in x0..=x1 {
            for y in y0..=y1 {
                for &k in self.bins.get(&(x, y)).into_iter().flatten() {
                    let (a, b) = self.edges[k];
                    best = best.min(geom::point_segment_distance(p, a, b));
                }
            }
        }
        best
    }

    pub fn circle_gap(&self, c: P, r: f64, reach: f64) -> f64 {
        if self.contains(c) {
            return -r;
        }
        self.edge_gap(c, r + reach) - r
    }
}

pub struct Finding {
    pub rule: &'static str,
    pub at: String,
    pub message: String,
}

#[derive(Default)]
pub struct Findings(pub Vec<Finding>);

impl Findings {
    pub fn add(&mut self, rule: &'static str, at: impl Into<String>, message: impl Into<String>) {
        debug_assert!(find(rule).is_some(), "no DRC rule `{rule}`");
        self.0.push(Finding { rule, at: at.into(), message: message.into() });
    }
}

pub fn recorded(cx: &Ctx, r: &mut Report) {
    let rule = r.rule;
    for f in cx.found.iter().filter(|f| f.rule == rule) {
        r.emit(f.at.clone(), f.message.clone());
    }
}

pub struct Report<'a> {
    d: &'a mut Diags,
    rule: &'static str,
    severity: Severity,
}

impl Report<'_> {
    pub fn emit(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.d.push_rule(self.severity, self.rule, at, message);
    }
}

pub fn every(_: &Setup) -> bool {
    true
}

pub fn registry() -> impl Iterator<Item = &'static Rule> {
    via::RULES
        .iter()
        .chain(drill::RULES)
        .chain(copper::RULES)
        .chain(track::RULES)
        .chain(zone::RULES)
        .chain(courtyard::RULES)
        .chain(mask::RULES)
        .chain(silk::RULES)
        .chain(assembly::RULES)
        .chain(mechanical::RULES)
        .chain(signal::RULES)
        .chain(test::RULES)
        .chain(placement::RULES)
        .chain(isolation::RULES)
}

pub fn find(id: &str) -> Option<&'static Rule> {
    registry().find(|r| r.id == id)
}

pub fn severity_of(board: &Board, rule: &Rule) -> Severity {
    board.drc.severity.get(rule.id).copied().unwrap_or(rule.severity)
}

pub fn enabled(board: &Board, rule: &Rule) -> bool {
    id_enabled(board, rule.id)
}

pub fn id_enabled(board: &Board, id: &str) -> bool {
    !board.drc.disable.iter().any(|d| d == id)
}

pub fn messages(rule: &Rule, cx: &Ctx) -> Vec<crate::diag::Diagnostic> {
    if !(rule.applies)(&Setup::of(cx)) || !enabled(cx.board, rule) {
        return Vec::new();
    }
    let mut d = Diags::new("");
    let mut report = Report { d: &mut d, rule: rule.id, severity: severity_of(cx.board, rule) };
    (rule.check)(cx, &mut report);
    d.list
}

pub fn run(cx: &Ctx, d: &mut Diags) {
    let setup = Setup::of(cx);
    for rule in registry() {
        if !(rule.applies)(&setup) || !enabled(cx.board, rule) {
            continue;
        }
        let mut report = Report { d, rule: rule.id, severity: severity_of(cx.board, rule) };
        (rule.check)(cx, &mut report);
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleStatus {
    pub id: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub default_severity: Severity,
    pub summary: &'static str,
    pub when: &'static str,
    pub applies: bool,
    pub enabled: bool,
}

pub fn status(board: &Board, setup: &Setup) -> Vec<RuleStatus> {
    registry()
        .map(|r| RuleStatus {
            id: r.id,
            category: r.category,
            severity: severity_of(board, r),
            default_severity: r.severity,
            summary: r.summary,
            when: r.when,
            applies: (r.applies)(setup),
            enabled: enabled(board, r),
        })
        .collect()
}

pub fn vias_in_pads(parts: &[Placed], vias: &[Via]) -> Vec<(usize, usize, usize)> {
    let pads = pads_where(parts, is_smd);
    let mut out = Vec::new();
    for (vi, v) in vias.iter().enumerate() {
        for p in &pads {
            if p.q.net == Some(v.net)
                && near(&p.bounds, v.at, v.diameter / 2.0)
                && p.q.copper.iter().any(|l| v.layers.contains(l))
                && via::via_on_pad(v, p.q).is_some_and(via::ViaOnPad::in_pad)
            {
                out.push((vi, p.part, p.pad));
            }
        }
    }
    out
}

pub fn list(items: &[String]) -> String {
    let mut s = items.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > 6 {
        s += &format!(" and {} more", items.len() - 6);
    }
    s
}

mod assembly;
mod copper;
mod courtyard;
mod drill;
mod isolation;
mod mask;
mod mechanical;
mod placement;

pub(crate) use mechanical::case_of;
mod signal;
mod silk;
mod test;
mod track;
pub(crate) mod via;
mod zone;

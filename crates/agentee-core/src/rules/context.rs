use crate::board::Board;
use crate::drc::{Ctx, Cu, CuShape, FillIndex, Hole, HoleOf, Owner};
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::{LayoutNet, Track, Via, ZoneFill};
use std::collections::HashMap;

pub trait Context {
    fn board(&self) -> &Board;
    fn copper(&self) -> &[String];
    fn nets(&self) -> &[LayoutNet];
    fn parts(&self) -> &[crate::layout::Placed];
    fn edge(&self) -> geom::BoardEdge<'_>;
    fn zones(&self) -> &[ZoneFill];
    fn fills(&self) -> &[FillIndex];
    fn spacing(&self) -> &super::Spacings;

    fn item(&self, i: usize) -> &Cu;
    fn hole(&self, i: usize) -> &Hole;
    fn hole_count(&self) -> usize;
    fn via(&self, k: usize) -> &Via;
    fn via_count(&self) -> usize;
    fn via_subjects(&self) -> Vec<usize>;
    fn planned_via(&self, k: usize) -> bool;
    fn items_near(&self, b: &Bounds, reach: f64) -> Vec<usize>;
    fn holes_near(&self, b: &Bounds, reach: f64) -> Vec<usize>;

    fn item_subjects(&self, reach: f64) -> Vec<usize>;
    fn hole_subjects(&self, reach: f64) -> Vec<usize>;
    fn planned_item(&self, i: usize) -> bool;
    fn planned_hole(&self, i: usize) -> bool;

    fn describe(&self, i: usize) -> String;
    fn hole_name(&self, i: usize) -> String;
    fn part_of_hole(&self, i: usize) -> Option<usize>;

    fn counts(&self, a: bool, b: bool) -> bool;
}

pub struct Placed<'a> {
    pub cx: &'a Ctx<'a>,
    holes: Vec<Hole>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

const CELL: f64 = 1.0;

fn index(bounds: impl Iterator<Item = Bounds>) -> HashMap<(i64, i64), Vec<usize>> {
    let mut g: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, b) in bounds.enumerate() {
        for x in (b.min[0] / CELL).floor() as i64..=(b.max[0] / CELL).floor() as i64 {
            for y in (b.min[1] / CELL).floor() as i64..=(b.max[1] / CELL).floor() as i64 {
                g.entry((x, y)).or_default().push(i);
            }
        }
    }
    g
}

fn lookup(g: &HashMap<(i64, i64), Vec<usize>>, b: &Bounds, reach: f64) -> Vec<usize> {
    let mut out = Vec::new();
    for x in ((b.min[0] - reach) / CELL).floor() as i64..=((b.max[0] + reach) / CELL).floor() as i64
    {
        for y in
            ((b.min[1] - reach) / CELL).floor() as i64..=((b.max[1] + reach) / CELL).floor() as i64
        {
            out.extend(g.get(&(x, y)).into_iter().flatten().copied());
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

impl<'a> Placed<'a> {
    pub fn new(cx: &'a Ctx<'a>) -> Placed<'a> {
        let holes = cx.holes();
        let grid = index(holes.iter().map(Hole::bounds));
        Placed { cx, holes, grid }
    }

    fn hole_count(&self) -> usize {
        self.holes.len()
    }
}

impl Context for Placed<'_> {
    fn via(&self, k: usize) -> &Via {
        &self.cx.vias[k]
    }

    fn via_count(&self) -> usize {
        self.cx.vias.len()
    }

    fn via_subjects(&self) -> Vec<usize> {
        (0..self.cx.vias.len()).collect()
    }

    fn planned_via(&self, _: usize) -> bool {
        false
    }

    fn board(&self) -> &Board {
        self.cx.board
    }

    fn copper(&self) -> &[String] {
        self.cx.copper
    }

    fn nets(&self) -> &[LayoutNet] {
        self.cx.nets
    }

    fn parts(&self) -> &[crate::layout::Placed] {
        self.cx.parts
    }

    fn edge(&self) -> geom::BoardEdge<'_> {
        self.cx.edge()
    }

    fn zones(&self) -> &[ZoneFill] {
        self.cx.zones
    }

    fn fills(&self) -> &[FillIndex] {
        self.cx.fills()
    }

    fn spacing(&self) -> &super::Spacings {
        self.cx.spacing()
    }

    fn item(&self, i: usize) -> &Cu {
        &self.cx.copper_items()[i]
    }

    fn hole(&self, i: usize) -> &Hole {
        &self.holes[i]
    }

    fn hole_count(&self) -> usize {
        self.holes.len()
    }

    fn items_near(&self, b: &Bounds, reach: f64) -> Vec<usize> {
        self.cx.items_near(b, reach)
    }

    fn holes_near(&self, b: &Bounds, reach: f64) -> Vec<usize> {
        lookup(&self.grid, b, reach)
    }

    fn item_subjects(&self, _: f64) -> Vec<usize> {
        (0..self.cx.copper_items().len()).collect()
    }

    fn hole_subjects(&self, _: f64) -> Vec<usize> {
        (0..self.hole_count()).collect()
    }

    fn planned_item(&self, _: usize) -> bool {
        false
    }

    fn planned_hole(&self, _: usize) -> bool {
        false
    }

    fn describe(&self, i: usize) -> String {
        self.cx.describe(&self.cx.copper_items()[i])
    }

    fn hole_name(&self, i: usize) -> String {
        self.cx.hole_name(&self.holes[i])
    }

    fn part_of_hole(&self, i: usize) -> Option<usize> {
        match self.holes[i].of {
            HoleOf::Pad(p, _) => Some(p),
            HoleOf::Via(_) => None,
        }
    }

    fn counts(&self, _: bool, _: bool) -> bool {
        true
    }
}

pub struct Planned<'a> {
    pub base: &'a Placed<'a>,
    pub tracks: Vec<Track>,
    pub vias: Vec<Via>,
    items: Vec<Cu>,
    holes: Vec<Hole>,
    first_item: usize,
    first_hole: usize,
}

impl<'a> Planned<'a> {
    pub fn new(base: &'a Placed<'a>, tracks: Vec<Track>, vias: Vec<Via>) -> Planned<'a> {
        Planned::after(base, &[], &[], tracks, vias)
    }

    pub fn after(
        base: &'a Placed<'a>,
        kept_tracks: &[Track],
        kept_vias: &[Via],
        tracks: Vec<Track>,
        vias: Vec<Via>,
    ) -> Planned<'a> {
        let (mut items, mut holes) = Planned::copper(base, kept_tracks, kept_vias, 0, 0);
        let (first_item, first_hole) = (items.len(), holes.len());
        let (more, more_holes) =
            Planned::copper(base, &tracks, &vias, kept_tracks.len(), kept_vias.len());
        items.extend(more);
        holes.extend(more_holes);
        let mut all_tracks = kept_tracks.to_vec();
        all_tracks.extend(tracks);
        let mut all_vias = kept_vias.to_vec();
        all_vias.extend(vias);
        Planned { base, tracks: all_tracks, vias: all_vias, items, holes, first_item, first_hole }
    }

    fn copper(
        base: &Placed,
        tracks: &[Track],
        vias: &[Via],
        track_from: usize,
        via_from: usize,
    ) -> (Vec<Cu>, Vec<Hole>) {
        let (bt, bv) = (base.cx.tracks.len() + track_from, base.cx.vias.len() + via_from);
        let mut items = Vec::new();
        let mut holes = Vec::new();
        for (k, t) in tracks.iter().enumerate() {
            for w in t.points.windows(2) {
                let mut b = Bounds::EMPTY;
                b.add_circle(w[0], t.width / 2.0);
                b.add_circle(w[1], t.width / 2.0);
                items.push(Cu {
                    owner: Owner::Track(bt + k),
                    net: Some(t.net),
                    layers: vec![t.layer.clone()],
                    bounds: b,
                    shape: CuShape::Seg(w[0], w[1], t.width / 2.0),
                });
            }
        }
        for (k, v) in vias.iter().enumerate() {
            let mut b = Bounds::EMPTY;
            b.add_circle(v.at, v.diameter / 2.0);
            items.push(Cu {
                owner: Owner::Via(bv + k),
                net: Some(v.net),
                layers: v.layers.clone(),
                bounds: b,
                shape: CuShape::Circle(v.at, v.diameter / 2.0),
            });
            holes.push(Hole {
                of: HoleOf::Via(bv + k),
                a: v.at,
                b: v.at,
                r: v.drill / 2.0,
                size: [v.drill, v.drill],
                plated: true,
                net: Some(v.net),
                layers: v.hole.clone(),
            });
        }
        (items, holes)
    }

    fn base_items(&self) -> usize {
        self.base.cx.copper_items().len()
    }

    fn base_holes(&self) -> usize {
        self.base.hole_count()
    }

    fn via_name(&self, at: P) -> String {
        format!("planned via at [{:.3}, {:.3}]", at[0], at[1])
    }
}

impl Context for Planned<'_> {
    fn board(&self) -> &Board {
        self.base.board()
    }

    fn copper(&self) -> &[String] {
        self.base.copper()
    }

    fn nets(&self) -> &[LayoutNet] {
        self.base.nets()
    }

    fn parts(&self) -> &[crate::layout::Placed] {
        self.base.parts()
    }

    fn edge(&self) -> geom::BoardEdge<'_> {
        self.base.edge()
    }

    fn zones(&self) -> &[ZoneFill] {
        self.base.zones()
    }

    fn fills(&self) -> &[FillIndex] {
        self.base.fills()
    }

    fn spacing(&self) -> &super::Spacings {
        self.base.spacing()
    }

    fn item(&self, i: usize) -> &Cu {
        let n = self.base_items();
        if i < n { self.base.item(i) } else { &self.items[i - n] }
    }

    fn hole(&self, i: usize) -> &Hole {
        let n = self.base_holes();
        if i < n { self.base.hole(i) } else { &self.holes[i - n] }
    }

    fn hole_count(&self) -> usize {
        self.base_holes() + self.holes.len()
    }

    fn via(&self, k: usize) -> &Via {
        let n = self.base.via_count();
        if k < n { self.base.via(k) } else { &self.vias[k - n] }
    }

    fn via_count(&self) -> usize {
        self.base.via_count() + self.vias.len()
    }

    fn via_subjects(&self) -> Vec<usize> {
        let n = self.base.via_count();
        (n + self.first_hole..n + self.vias.len()).collect()
    }

    fn planned_via(&self, k: usize) -> bool {
        k >= self.base.via_count() + self.first_hole
    }

    fn items_near(&self, b: &Bounds, reach: f64) -> Vec<usize> {
        let n = self.base_items();
        let mut out = self.base.items_near(b, reach);
        for (k, c) in self.items.iter().enumerate() {
            if overlaps(&c.bounds, b, reach) {
                out.push(n + k);
            }
        }
        out
    }

    fn holes_near(&self, b: &Bounds, reach: f64) -> Vec<usize> {
        let n = self.base_holes();
        let mut out = self.base.holes_near(b, reach);
        for (k, h) in self.holes.iter().enumerate() {
            if overlaps(&h.bounds(), b, reach) {
                out.push(n + k);
            }
        }
        out
    }

    fn item_subjects(&self, reach: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for c in &self.items[self.first_item..] {
            out.extend(self.items_near(&c.bounds, reach));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn hole_subjects(&self, reach: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for c in &self.items[self.first_item..] {
            out.extend(self.holes_near(&c.bounds, reach));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn planned_item(&self, i: usize) -> bool {
        i >= self.base_items() + self.first_item
    }

    fn planned_hole(&self, i: usize) -> bool {
        i >= self.base_holes() + self.first_hole
    }

    fn describe(&self, i: usize) -> String {
        let n = self.base_items();
        if i < n {
            return self.base.describe(i);
        }
        let c = &self.items[i - n];
        let net = c.net.map(|n| self.nets()[n].name.as_str()).unwrap_or("");
        match c.shape {
            CuShape::Circle(at, _) => format!("{} ({net})", self.via_name(at)),
            CuShape::Seg(a, b, _) => format!(
                "planned track [{:.3}, {:.3}] to [{:.3}, {:.3}] ({net})",
                a[0], a[1], b[0], b[1]
            ),
            CuShape::Poly(_) => format!("planned copper ({net})"),
        }
    }

    fn hole_name(&self, i: usize) -> String {
        let n = self.base_holes();
        if i < n { self.base.hole_name(i) } else { self.via_name(self.holes[i - n].a) }
    }

    fn part_of_hole(&self, i: usize) -> Option<usize> {
        if i < self.base_holes() { self.base.part_of_hole(i) } else { None }
    }

    fn counts(&self, a: bool, b: bool) -> bool {
        a || b
    }
}

fn overlaps(a: &Bounds, b: &Bounds, reach: f64) -> bool {
    !a.is_empty()
        && a.min[0] <= b.max[0] + reach
        && b.min[0] - reach <= a.max[0]
        && a.min[1] <= b.max[1] + reach
        && b.min[1] - reach <= a.max[1]
}

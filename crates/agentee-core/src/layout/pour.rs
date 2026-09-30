use super::{
    Item, LayoutNet, Owner, Shape, ZoneFill, check_zones, clip_overlay, fill_clip, fill_zone,
    keep_connected, open_to_width, probe_shapes, rasterize, vector_fill,
};
use crate::geom::P;
use crate::graphic::Bounds;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

const FILL_VERSION: u64 = 1;
const STORE_GRID: f64 = 1e4;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillFile {
    pub zone: usize,
    pub layer: String,
    pub hash: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub islands_removed: usize,
    pub rings: Vec<Vec<P>>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Clone, Debug, Serialize)]
pub struct FillKey {
    pub zone: usize,
    pub layer: String,
    pub hash: u64,
    pub stored: bool,
    pub stale: bool,
}

impl FillKey {
    pub fn hex(&self) -> String {
        format!("{:016x}", self.hash)
    }
}

pub fn to_file(key: &FillKey, fill: &ZoneFill) -> FillFile {
    FillFile {
        zone: key.zone,
        layer: key.layer.clone(),
        hash: key.hex(),
        islands_removed: fill.islands_removed,
        rings: fill.rings.clone(),
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        s.bytes().for_each(|b| self.u64(b as u64));
    }
    fn points(&mut self, pts: &[P]) {
        self.u64(pts.len() as u64);
        pts.iter().flatten().for_each(|v| self.f64(*v));
    }
    fn shape(&mut self, s: &Shape) {
        match s {
            Shape::Poly(rings) => {
                self.u64(0);
                rings.iter().for_each(|r| self.points(r));
            }
            Shape::Seg(a, b, hw) => {
                self.u64(1);
                self.points(&[*a, *b]);
                self.f64(*hw);
            }
            Shape::Circle(c, r) => {
                self.u64(2);
                self.points(&[*c]);
                self.f64(*r);
            }
        }
    }
}

pub(super) struct FillSpec<'a> {
    pub net: usize,
    pub net_name: &'a str,
    pub layer: &'a str,
    pub poly: &'a [P],
    pub board: crate::geom::BoardEdge<'a>,
    pub edge_clear: f64,
    pub clearance: f64,
    pub cutouts: &'a [&'a Vec<P>],
    pub min_width: f64,
    pub min_island_area: f64,
}

impl FillSpec<'_> {
    fn reach(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        self.poly.iter().for_each(|p| b.add(*p));
        b
    }

    pub fn hash(
        &self,
        items: &[Item],
        clearance_of: &dyn Fn(Option<usize>) -> f64,
        blockers: &[u64],
    ) -> u64 {
        let mut h = Fnv::new();
        h.u64(FILL_VERSION);
        h.str(self.net_name);
        h.str(self.layer);
        h.points(self.poly);
        h.points(self.board.outline);
        if !self.board.cutouts.is_empty() {
            h.str("board cutouts");
            self.board.cutouts.iter().for_each(|c| h.points(c));
        }
        for v in [self.edge_clear, self.clearance, self.min_width, self.min_island_area] {
            h.f64(v);
        }
        self.cutouts.iter().for_each(|c| h.points(c));
        blockers.iter().for_each(|b| h.u64(*b));
        let zone = self.reach();
        for it in items.iter().filter(|it| it.layers.iter().any(|l| l == self.layer)) {
            let own = it.net == Some(self.net);
            let gap = self.clearance.max(clearance_of(it.net)).max(it.pour_gap);
            let grow = gap + self.min_width + 0.5;
            if it.bounds.max[0] + grow < zone.min[0]
                || it.bounds.min[0] - grow > zone.max[0]
                || it.bounds.max[1] + grow < zone.min[1]
                || it.bounds.min[1] - grow > zone.max[1]
            {
                continue;
            }
            h.u64(own as u64);
            h.u64((it.owner == Owner::Hole) as u64);
            h.f64(gap);
            h.shape(&it.shape);
        }
        h.0
    }

    pub fn stored(&self, items: &[Item], file: &FillFile) -> (ZoneFill, Vec<Vec<usize>>) {
        let (origin, cell, width, height) = raster_grid(self.poly);
        let mut fill = ZoneFill {
            net: self.net,
            layer: self.layer.to_string(),
            origin,
            cell,
            width,
            height,
            mask: vec![0; width * height],
            islands_removed: file.islands_removed,
            min_width: self.min_width,
            rings: file.rings.clone(),
            triangles: Vec::new(),
        };
        let (_, touched, _) =
            keep_connected(&fill.rings, self.net, self.layer, items, self.min_island_area);
        rasterize(&mut fill);
        fill.triangles = crate::contour::triangles(&fill.rings);
        (fill, touched)
    }
}

pub(super) fn raster_grid(poly: &[P]) -> (P, f64, usize, usize) {
    let mut b = Bounds::EMPTY;
    poly.iter().for_each(|p| b.add(*p));
    let [sw, sh] = b.size();
    let cell = (sw.max(sh) / 1600.0).max(0.02);
    (b.min, cell, (sw / cell).ceil() as usize + 1, (sh / cell).ceil() as usize + 1)
}

pub(super) fn snap(rings: &mut Vec<Vec<P>>) {
    for r in rings.iter_mut() {
        for p in r.iter_mut() {
            *p = p.map(|v| (v * STORE_GRID).round() / STORE_GRID);
        }
        r.dedup();
        while r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
    }
    rings.retain(|r| r.len() >= 3);
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<FillCase>>> = const { RefCell::new(None) };
    static ZONES: RefCell<Vec<ZonesCase>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

pub(super) fn capture(case: impl FnOnce() -> FillCase) {
    CAPTURE.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(case());
        }
    });
}

pub(super) fn capture_zones(case: impl FnOnce() -> ZonesCase) {
    if capturing() {
        let case = case();
        ZONES.with(|z| z.borrow_mut().push(case));
    }
}

#[doc(hidden)]
pub fn take_zones_cases() -> Vec<ZonesCase> {
    ZONES.with(|z| std::mem::take(&mut *z.borrow_mut()))
}

#[doc(hidden)]
pub struct ZonesCase {
    pub(super) zones: Vec<ZoneFill>,
    pub(super) items: Vec<Item>,
    pub(super) nets: Vec<LayoutNet>,
    pub(super) default_clearance: f64,
}

impl ZonesCase {
    pub fn check(&self) -> usize {
        let mut found = crate::drc::Findings::default();
        let clearance_of =
            |n: Option<usize>| n.map(|n| self.nets[n].clearance).unwrap_or(self.default_clearance);
        check_zones(&self.zones, &self.items, &self.nets, &clearance_of, &mut found);
        std::hint::black_box(&found);
        self.zones.len()
    }
}

#[doc(hidden)]
pub fn capture_fill_cases() {
    ZONES.with(|z| z.borrow_mut().clear());
    CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
}

#[doc(hidden)]
pub fn take_fill_cases() -> Vec<FillCase> {
    CAPTURE.with(|c| c.borrow_mut().take().unwrap_or_default())
}

#[doc(hidden)]
pub struct FillCase {
    pub name: String,
    pub(super) net: usize,
    pub(super) layer: String,
    pub(super) poly: Vec<P>,
    pub(super) board: Vec<P>,
    pub(super) board_cutouts: Vec<Vec<P>>,
    pub(super) edge_clear: f64,
    pub(super) clearance: f64,
    pub(super) items: Vec<Item>,
    pub(super) net_clearance: Vec<f64>,
    pub(super) default_clearance: f64,
    pub(super) cutouts: Vec<Vec<P>>,
    pub(super) blockers: Vec<ZoneFill>,
    pub(super) min_width: f64,
    pub(super) min_island_area: f64,
}

impl FillCase {
    fn clearance_of(&self, n: Option<usize>) -> f64 {
        n.and_then(|n| self.net_clearance.get(n).copied()).unwrap_or(self.default_clearance)
    }

    pub fn items(&self) -> usize {
        self.items.len()
    }

    pub fn blockers(&self) -> usize {
        self.blockers.len()
    }

    pub fn fill(&self) -> ZoneFill {
        let cutouts: Vec<&Vec<P>> = self.cutouts.iter().collect();
        let blockers: Vec<&ZoneFill> = self.blockers.iter().collect();
        fill_zone(
            self.net,
            &self.layer,
            &self.poly,
            crate::geom::BoardEdge::new(&self.board, &self.board_cutouts),
            self.edge_clear,
            self.clearance,
            &self.items,
            &|n| self.clearance_of(n),
            &cutouts,
            &blockers,
            self.min_width,
            self.min_island_area,
        )
        .0
    }

    pub fn vector(&self, raster: &ZoneFill) -> Vec<Vec<P>> {
        let cutouts: Vec<&Vec<P>> = self.cutouts.iter().collect();
        let blockers: Vec<&ZoneFill> = self.blockers.iter().collect();
        vector_fill(
            raster,
            &self.poly,
            crate::geom::BoardEdge::new(&self.board, &self.board_cutouts),
            self.edge_clear,
            self.clearance,
            &self.items,
            &|n| self.clearance_of(n),
            &cutouts,
            &blockers,
            self.min_width,
        )
    }

    pub fn clip(&self) -> (Vec<Vec<P>>, Vec<Vec<P>>) {
        let cutouts: Vec<&Vec<P>> = self.cutouts.iter().collect();
        let blockers: Vec<&ZoneFill> = self.blockers.iter().collect();
        fill_clip(
            &self.layer,
            self.net,
            &self.poly,
            crate::geom::BoardEdge::new(&self.board, &self.board_cutouts),
            self.edge_clear,
            self.clearance,
            &self.items,
            &|n| self.clearance_of(n),
            &cutouts,
            &blockers,
            self.min_width,
        )
    }

    pub fn overlay(&self, subject: &Vec<Vec<P>>, clip: &Vec<Vec<P>>) -> Vec<Vec<Vec<P>>> {
        clip_overlay(subject, clip)
    }

    pub fn open(&self, shapes: Vec<Vec<Vec<P>>>) -> Vec<Vec<Vec<P>>> {
        open_to_width(shapes, self.min_width)
    }

    pub fn probe(&self, shapes: Vec<Vec<Vec<P>>>, raster: &ZoneFill) -> Vec<Vec<P>> {
        probe_shapes(shapes, raster)
    }

    pub fn keep_connected(&self, rings: &[Vec<P>]) -> usize {
        keep_connected(rings, self.net, &self.layer, &self.items, self.min_island_area).1.len()
    }

    pub fn rasterize(&self, fill: &mut ZoneFill) {
        rasterize(fill);
    }
}

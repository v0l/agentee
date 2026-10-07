use super::Context;
use crate::drc::{CuShape, HoleOf, Owner};
use crate::geom::{self, P};
use crate::graphic::Bounds;

#[derive(Clone, Debug)]
pub enum Kind {
    Track { half: f64 },
    Via { r: f64, drill: f64, hole: Vec<String>, fill: Option<crate::board::ViaFill> },
    Pad { r: f64 },
}

#[derive(Clone, Debug)]
pub struct Template {
    pub net: Option<usize>,
    pub also: Vec<usize>,
    pub layers: Vec<String>,
    pub kind: Kind,
}

impl Template {
    pub fn track(net: usize, layer: &str, width: f64) -> Template {
        Template {
            net: Some(net),
            also: Vec::new(),
            layers: vec![layer.into()],
            kind: Kind::Track { half: width / 2.0 },
        }
    }

    pub fn via(via: &crate::layout::Via) -> Template {
        Template {
            net: Some(via.net),
            also: Vec::new(),
            layers: via.layers.clone(),
            kind: Kind::Via {
                r: via.diameter / 2.0,
                drill: via.drill,
                hole: via.hole.clone(),
                fill: via.fill,
            },
        }
    }

    pub fn pad(net: usize, layer: &str, r: f64) -> Template {
        Template { net: Some(net), also: Vec::new(), layers: vec![layer.into()], kind: Kind::Pad { r } }
    }

    pub fn half(&self) -> f64 {
        match self.kind {
            Kind::Track { half } => half,
            Kind::Via { r, .. } | Kind::Pad { r } => r,
        }
    }

    pub fn owns(&self, net: Option<usize>) -> bool {
        net.is_some() && (net == self.net || net.is_some_and(|n| self.also.contains(&n)))
    }

    pub fn owner(&self) -> Owner {
        Owner::Copper(usize::MAX)
    }
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub origin: P,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub layers: Vec<String>,
    ok: Vec<Vec<bool>>,
}

impl Zone {
    pub fn new(window: &Bounds, cell: f64, layers: &[String]) -> Zone {
        let w = ((window.max[0] - window.min[0]) / cell - 1e-9).ceil().max(1.0) as usize + 1;
        let h = ((window.max[1] - window.min[1]) / cell - 1e-9).ceil().max(1.0) as usize + 1;
        Zone {
            origin: window.min,
            cell,
            w,
            h,
            layers: layers.to_vec(),
            ok: layers.iter().map(|_| vec![true; w * h]).collect(),
        }
    }

    pub fn margin(&self) -> f64 {
        self.cell * std::f64::consts::FRAC_1_SQRT_2
    }

    pub fn centre(&self, i: usize, j: usize) -> P {
        [self.origin[0] + i as f64 * self.cell, self.origin[1] + j as f64 * self.cell]
    }

    fn cell_of(&self, p: P) -> Option<(usize, usize)> {
        let i = ((p[0] - self.origin[0]) / self.cell).round();
        let j = ((p[1] - self.origin[1]) / self.cell).round();
        (i >= 0.0 && j >= 0.0 && (i as usize) < self.w && (j as usize) < self.h)
            .then_some((i as usize, j as usize))
    }

    fn range(&self, b: &Bounds, d: f64) -> (usize, usize, usize, usize) {
        let lo = |v: f64, o: f64| (((v - d - o) / self.cell).floor().max(0.0)) as usize;
        let hi = |v: f64, o: f64, n: usize| {
            ((((v + d - o) / self.cell).ceil()).max(0.0) as usize).min(n.saturating_sub(1))
        };
        (
            lo(b.min[0], self.origin[0]),
            hi(b.max[0], self.origin[0], self.w),
            lo(b.min[1], self.origin[1]),
            hi(b.max[1], self.origin[1], self.h),
        )
    }

    fn layer_ids(&self, layers: Option<&[String]>) -> Vec<usize> {
        (0..self.layers.len())
            .filter(|&k| layers.is_none_or(|ls| ls.contains(&self.layers[k])))
            .collect()
    }

    pub fn forbid(&mut self, layers: Option<&[String]>, shape: &CuShape, dist: f64) {
        let ids = self.layer_ids(layers);
        if ids.is_empty() {
            return;
        }
        let d = dist + self.margin();
        let b = bounds_of(shape);
        let (x0, x1, y0, y1) = self.range(&b, d);
        for j in y0..=y1 {
            for i in x0..=x1 {
                let c = self.centre(i, j);
                if shape.point_distance(c) < d {
                    for &k in &ids {
                        self.ok[k][j * self.w + i] = false;
                    }
                }
            }
        }
    }

    pub fn forbid_where(
        &mut self,
        layers: Option<&[String]>,
        near: &Bounds,
        reach: f64,
        f: impl Fn(P, f64) -> bool,
    ) {
        let ids = self.layer_ids(layers);
        if ids.is_empty() {
            return;
        }
        let m = self.margin();
        let (x0, x1, y0, y1) = self.range(near, reach + m);
        for j in y0..=y1 {
            for i in x0..=x1 {
                if f(self.centre(i, j), m) {
                    for &k in &ids {
                        self.ok[k][j * self.w + i] = false;
                    }
                }
            }
        }
    }

    pub fn allows_on(&self, layer: &str, p: P) -> bool {
        let Some(k) = self.layers.iter().position(|l| l == layer) else { return false };
        self.cell_of(p).is_some_and(|(i, j)| self.ok[k][j * self.w + i])
    }

    pub fn allows(&self, p: P) -> bool {
        self.cell_of(p).is_some_and(|(i, j)| self.ok.iter().all(|l| l[j * self.w + i]))
    }

    pub fn allows_segment(&self, a: P, b: P) -> bool {
        let n = ((geom::dist(a, b) / (self.cell / 2.0)).ceil() as usize).max(1);
        (0..=n).all(|k| {
            let t = k as f64 / n as f64;
            self.allows([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
        })
    }

    pub fn allows_cell(&self, i: usize, j: usize) -> bool {
        i < self.w && j < self.h && self.ok.iter().all(|l| l[j * self.w + i])
    }

    pub fn spots(&self) -> Vec<P> {
        let mut out = Vec::new();
        for j in 0..self.h {
            for i in 0..self.w {
                if self.ok.iter().all(|l| l[j * self.w + i]) {
                    out.push(self.centre(i, j));
                }
            }
        }
        out
    }

    pub fn window(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        b.add(self.origin);
        b.add(self.centre(self.w - 1, self.h - 1));
        b
    }

    pub fn and(&mut self, other: &Zone) {
        for (k, layer) in self.layers.clone().iter().enumerate() {
            for j in 0..self.h {
                for i in 0..self.w {
                    let c = self.centre(i, j);
                    if other.window().contains(&point_bounds(c)) && !other.allows_on(layer, c) {
                        self.ok[k][j * self.w + i] = false;
                    }
                }
            }
        }
    }
}

fn point_bounds(p: P) -> Bounds {
    let mut b = Bounds::EMPTY;
    b.add(p);
    b
}

pub fn bounds_of(shape: &CuShape) -> Bounds {
    let mut b = Bounds::EMPTY;
    match shape {
        CuShape::Poly(v) => v.iter().flatten().for_each(|p| b.add(*p)),
        CuShape::Seg(a, c, hw) => {
            b.add_circle(*a, *hw);
            b.add_circle(*c, *hw);
        }
        CuShape::Circle(c, r) => b.add_circle(*c, *r),
    }
    b
}

pub trait Constrains {
    fn constrain<C: Context>(&self, cx: &C, t: &Template, zone: &mut Zone);
}

pub fn green<C: Context>(cx: &C, t: &Template, window: &Bounds, cell: f64) -> Zone {
    let mut zone = Zone::new(window, cell, &t.layers);
    constrain_all(cx, t, &mut zone);
    zone
}

pub fn refresh<C: Context>(cx: &C, t: &Template, zone: &mut Zone, around: &Bounds) {
    let local = green(cx, t, around, zone.cell);
    zone.and(&local);
}

pub fn constrain_all<C: Context>(cx: &C, t: &Template, zone: &mut Zone) {
    super::NetClearance.constrain(cx, t, zone);
    super::IsolationClearance.constrain(cx, t, zone);
    super::Creepage.constrain(cx, t, zone);
    super::CopperToEdge.constrain(cx, t, zone);
    super::HoleToCopper(super::Which::Plated).constrain(cx, t, zone);
    super::HoleToCopper(super::Which::Inner).constrain(cx, t, zone);
    super::HoleToCopper(super::Which::Npth).constrain(cx, t, zone);
    super::HoleToHole.constrain(cx, t, zone);
    super::ViaCutsPad.constrain(cx, t, zone);
    super::ViaInPadFill.constrain(cx, t, zone);
    super::HoleToSmdPad.constrain(cx, t, zone);
    super::StackedVia.constrain(cx, t, zone);
}

pub fn hole_shape(h: &crate::drc::Hole) -> CuShape {
    CuShape::Seg(h.a, h.b, h.r)
}

pub fn own_hole(of: HoleOf, owner: Owner) -> bool {
    match (of, owner) {
        (HoleOf::Via(a), Owner::Via(b)) => a == b,
        (HoleOf::Pad(p, k), Owner::Pad(q, m)) => (p, k) == (q, m),
        _ => false,
    }
}

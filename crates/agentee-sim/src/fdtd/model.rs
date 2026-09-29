use super::edge;
use super::engine::{self, Edge, Element, Grid, Lumped, Materials, PortDef, Sim};
use crate::xsection::lines;
use agentee_core::board::{Board, LayerKind};
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::layout::{Layout, ZoneFill};
use agentee_core::sim::{Model, Sim as Spec};

#[derive(Clone, Debug)]
pub struct Sheet {
    pub name: String,
    pub z: f64,
    pub thickness: f64,
}

#[derive(Clone, Debug)]
pub struct Dielectric {
    pub z0: f64,
    pub z1: f64,
    pub er: f64,
    pub tan: f64,
    pub pinned: bool,
}

#[derive(Clone, Debug)]
pub enum Copper {
    Poly(Vec<P>),
    Seg(P, P, f64),
    Circle(P, f64),
    Fill(ZoneFill),
}

impl Copper {
    fn contains(&self, p: P) -> bool {
        match self {
            Copper::Poly(v) => geom::point_in_polygon(p, v),
            Copper::Seg(a, b, w) => geom::point_segment_distance(p, *a, *b) <= w / 2.0,
            Copper::Circle(c, r) => geom::dist(p, *c) <= *r,
            Copper::Fill(z) => z.filled(p),
        }
    }

    fn contains_grown(&self, p: P, grow: f64) -> bool {
        if self.contains(p) {
            return true;
        }
        if grow <= 0.0 {
            return false;
        }
        match self {
            Copper::Seg(a, b, w) => geom::point_segment_distance(p, *a, *b) <= w / 2.0 + grow,
            Copper::Circle(c, r) => geom::dist(p, *c) <= *r + grow,
            _ => [[grow, 0.0], [-grow, 0.0], [0.0, grow], [0.0, -grow]]
                .iter()
                .any(|d| self.contains([p[0] + d[0], p[1] + d[1]])),
        }
    }

    fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        match self {
            Copper::Poly(v) => v.iter().for_each(|q| b.add(*q)),
            Copper::Seg(a, c, w) => {
                b.add_circle(*a, w / 2.0);
                b.add_circle(*c, w / 2.0);
            }
            Copper::Circle(c, r) => b.add_circle(*c, *r),
            Copper::Fill(z) => {
                b.add(z.origin);
                b.add([
                    z.origin[0] + z.width as f64 * z.cell,
                    z.origin[1] + z.height as f64 * z.cell,
                ]);
            }
        }
        b
    }
}

#[derive(Clone, Debug)]
pub struct ModelPort {
    pub name: String,
    pub at: P,
    pub area: Vec<P>,
    pub sheet: usize,
    pub reference: usize,
    pub r: f64,
}

#[derive(Clone, Debug)]
pub struct ModelElement {
    pub name: String,
    pub a: P,
    pub b: P,
    pub sheet: usize,
    pub element: Element,
}

#[derive(Clone, Debug, Default)]
pub struct PcbModel {
    pub outline: Vec<P>,
    pub sheets: Vec<Sheet>,
    pub dielectrics: Vec<Dielectric>,
    pub copper: Vec<(usize, Copper)>,
    pub vias: Vec<(P, f64, usize, usize)>,
    pub ports: Vec<ModelPort>,
    pub elements: Vec<ModelElement>,
    pub features_x: Vec<f64>,
    pub features_y: Vec<f64>,
    pub region: Option<[f64; 4]>,
    pub roughness: crate::loss::Roughness,
}

pub struct Meshing {
    pub cell: f64,
    pub f_max: f64,
    pub margin: f64,
    pub pml: usize,
    pub f0: f64,
}

pub fn stack(board: &Board) -> (Vec<Sheet>, Vec<Dielectric>) {
    let layers: Vec<_> = board
        .stackup
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Copper || l.kind.is_dielectric())
        .collect();
    let n_cu = layers.iter().filter(|l| l.kind == LayerKind::Copper).count();
    let mut z = 0.0;
    let mut sheets = Vec::new();
    let mut diel = Vec::new();
    let mut seen = 0;
    let mut last_er = (4.4, 0.02);
    for (i, l) in layers.iter().enumerate() {
        let t = l.thickness.to_mm();
        let (top, bottom) = (z, z - t);
        if l.kind == LayerKind::Copper {
            let at = if seen == 0 {
                bottom
            } else if seen + 1 == n_cu {
                top
            } else {
                0.5 * (top + bottom)
            };
            if seen > 0 && seen + 1 < n_cu {
                let below = layers.get(i + 1).map(|b| (b.er, b.loss_tangent)).unwrap_or(last_er);
                let (er, tan) = if last_er.0 > 0.0 { last_er } else { below };
                diel.push(Dielectric { z0: bottom, z1: top, er, tan, pinned: false });
            }
            sheets.push(Sheet { name: l.name.clone(), z: at, thickness: t });
            seen += 1;
        } else {
            diel.push(Dielectric {
                z0: bottom,
                z1: top,
                er: l.er,
                tan: l.loss_tangent,
                pinned: true,
            });
            last_er = (l.er, l.loss_tangent);
        }
        z = bottom;
    }
    if let (Some(first), Some(last)) = (sheets.first().map(|s| s.z), sheets.last().map(|s| s.z)) {
        diel.retain(|d| d.z1 > last - 1e-12 && d.z0 < first + 1e-12);
        for d in &mut diel {
            d.z1 = d.z1.min(first);
            d.z0 = d.z0.max(last);
        }
    }
    (sheets, diel)
}

impl PcbModel {
    pub fn from_layout(layout: &Layout, board: &Board, spec: &Spec) -> Result<PcbModel, String> {
        let mut m = PcbModel::geometry(layout, board);
        m.region = spec.region;
        let sheets = m.sheets.clone();
        let sheet = |name: &str| sheets.iter().position(|s| s.name == name);
        for p in &spec.ports {
            let (Some(s), Some(r)) = (sheet(&p.layer), sheet(&p.reference)) else {
                return Err(format!("port {} is on a layer outside the stackup", p.name));
            };
            let area =
                layout.parts[p.part].pads[p.pad].outlines.first().cloned().unwrap_or_default();
            m.ports.push(ModelPort {
                name: p.name.clone(),
                at: p.at,
                area,
                sheet: s,
                reference: r,
                r: p.impedance,
            });
        }
        let inside = |p: P| {
            spec.region
                .is_none_or(|[x0, y0, x1, y1]| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1)
        };
        for e in &spec.elements {
            let Some(s) = sheet(&e.layer) else { continue };
            if !inside(e.a) && !inside(e.b) {
                continue;
            }
            let element = match e.model {
                Model::Capacitor(c) => Element::Capacitor(c),
                Model::Inductor(l) => Element::Inductor(l),
                Model::Resistor(r) => Element::Resistor(r),
                Model::Open => continue,
            };
            m.elements.push(ModelElement {
                name: e.reference.clone(),
                a: e.a,
                b: e.b,
                sheet: s,
                element,
            });
        }
        Ok(m)
    }

    pub fn geometry(layout: &Layout, board: &Board) -> PcbModel {
        let (sheets, dielectrics) = stack(board);
        let sheet = |name: &str| sheets.iter().position(|s| s.name == name);
        let mut m = PcbModel {
            outline: layout.outline.clone(),
            sheets: sheets.clone(),
            dielectrics,
            roughness: crate::loss::Roughness {
                rms_um: board.stackup.roughness_um,
                huray_radius_um: board.stackup.huray.map(|h| h.0),
                huray_ratio: board.stackup.huray.map(|h| h.1),
            },
            ..Default::default()
        };
        for part in &layout.parts {
            for pad in &part.pads {
                for l in &pad.copper {
                    if let Some(s) = sheet(l) {
                        for o in &pad.outlines {
                            m.copper.push((s, Copper::Poly(o.clone())));
                        }
                    }
                }
                if pad.copper.len() > 1
                    && let Some((c, size, _)) = pad.drill
                {
                    let r = size[0].min(size[1]) / 2.0;
                    let ks: Vec<usize> = pad.copper.iter().filter_map(|l| sheet(l)).collect();
                    if let (Some(a), Some(b)) = (ks.iter().min(), ks.iter().max()) {
                        m.vias.push((c, r, *a, *b));
                    }
                }
                let mut b = Bounds::EMPTY;
                pad.outlines.iter().flatten().for_each(|q| b.add(*q));
                if !b.is_empty() {
                    m.features_x.extend([b.min[0], b.max[0]]);
                    m.features_y.extend([b.min[1], b.max[1]]);
                }
            }
        }
        for t in &layout.tracks {
            let Some(s) = sheet(&t.layer) else { continue };
            for w in t.points.windows(2) {
                m.copper.push((s, Copper::Seg(w[0], w[1], t.width)));
                let hw = t.width / 2.0;
                if (w[0][1] - w[1][1]).abs() < 1e-9 {
                    m.features_y.extend([w[0][1] - hw, w[0][1] + hw]);
                    m.features_x.extend([w[0][0].min(w[1][0]) - hw, w[0][0].max(w[1][0]) + hw]);
                }
                if (w[0][0] - w[1][0]).abs() < 1e-9 {
                    m.features_x.extend([w[0][0] - hw, w[0][0] + hw]);
                    m.features_y.extend([w[0][1].min(w[1][1]) - hw, w[0][1].max(w[1][1]) + hw]);
                }
            }
        }
        for v in &layout.vias {
            let ks: Vec<usize> = v.layers.iter().filter_map(|l| sheet(l)).collect();
            for k in &ks {
                m.copper.push((*k, Copper::Circle(v.at, v.diameter / 2.0)));
            }
            if let (Some(a), Some(b)) = (ks.iter().min(), ks.iter().max()) {
                m.vias.push((v.at, v.drill / 2.0, *a, *b));
            }
        }
        for z in &layout.zones {
            if let Some(s) = sheet(&z.layer) {
                for ring in &z.rings {
                    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        if (a[1] - b[1]).abs() < 1e-9 && (a[0] - b[0]).abs() > 0.2 {
                            m.features_y.push(a[1]);
                        }
                        if (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() > 0.2 {
                            m.features_x.push(a[0]);
                        }
                    }
                }
                m.copper.push((s, Copper::Fill(z.clone())));
            }
        }
        m
    }

    fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        if let Some([x0, y0, x1, y1]) = self.region {
            b.add([x0, y0]);
            b.add([x1, y1]);
            return b;
        }
        self.outline.iter().for_each(|p| b.add(*p));
        if b.is_empty() {
            for (_, c) in &self.copper {
                b.union(&c.bounds());
            }
        }
        b
    }

    pub fn mesh(&self, opt: &Meshing) -> Grid {
        let b = self.bounds();
        let er_max = self.dielectrics.iter().map(|d| d.er).fold(1.0, f64::max);
        let coarse =
            (engine::C0 / opt.f_max / 20.0 / er_max.sqrt() * 1000.0).min(1.0).max(opt.cell);
        let ratio = 1.3;
        let axis_lines = |lo: f64, hi: f64, pinned: Vec<f64>, soft: &[f64]| {
            let mut fixed: Vec<f64> = vec![lo - opt.margin, hi + opt.margin];
            fixed.extend(
                pinned.iter().copied().filter(|v| *v > lo - opt.margin && *v < hi + opt.margin),
            );
            fixed.sort_by(f64::total_cmp);
            fixed.dedup_by(|a, b| (*a - *b).abs() < opt.cell * 0.3);
            for s in soft {
                if *s <= lo - opt.margin || *s >= hi + opt.margin {
                    continue;
                }
                if fixed.iter().all(|f| (f - s).abs() > opt.cell * 0.6) {
                    fixed.push(*s);
                }
            }
            fixed.sort_by(f64::total_cmp);
            let mut spaced: Vec<f64> = Vec::new();
            for v in fixed {
                let keep = pinned.iter().any(|p| (p - v).abs() < 1e-9);
                match spaced.last() {
                    Some(l) if v - l < opt.cell * 0.6 => {
                        if keep {
                            *spaced.last_mut().unwrap() = v;
                        }
                    }
                    _ => spaced.push(v),
                }
            }
            let fixed = spaced;
            let features: Vec<(f64, f64)> = fixed.iter().map(|v| (*v, opt.cell)).collect();
            let inner = lines(&fixed, &features, coarse, ratio);
            pad_pml(inner, opt.pml)
        };
        let pinned = |f: &dyn Fn(P) -> f64| -> Vec<f64> {
            let mut v: Vec<f64> = self.ports.iter().map(|p| f(p.at)).collect();
            v.extend(self.elements.iter().flat_map(|e| [f(e.a), f(e.b)]));
            v.extend(self.vias.iter().map(|x| f(x.0)));
            v
        };
        let top = self.sheets.iter().map(|s| s.z).fold(f64::MIN, f64::max);
        let bottom = self.sheets.iter().map(|s| s.z).fold(f64::MAX, f64::min);
        let mut fz: Vec<f64> = self.sheets.iter().map(|s| s.z).collect();
        let mut feat_z: Vec<(f64, f64)> = Vec::new();
        for d in self.dielectrics.iter().filter(|d| d.pinned) {
            fz.extend([d.z0, d.z1]);
            let size = ((d.z1 - d.z0) / 4.0).min(opt.cell * 2.0).max(1e-4);
            feat_z.extend([(d.z0, size), (d.z1, size)]);
        }
        for s in &self.sheets {
            feat_z.push((s.z, opt.cell.min(0.05)));
        }
        let air = opt.margin.max(2.0 * (top - bottom).max(0.5));
        fz.extend([top + air, bottom - air]);
        fz.sort_by(f64::total_cmp);
        let sheets_z: Vec<f64> = self.sheets.iter().map(|s| s.z).collect();
        let mut kept: Vec<f64> = Vec::new();
        for v in fz {
            match kept.last() {
                Some(l) if v - l < opt.cell * 0.4 => {
                    if sheets_z.iter().any(|s| (s - v).abs() < 1e-12) {
                        *kept.last_mut().unwrap() = v;
                    }
                }
                _ => kept.push(v),
            }
        }
        let fz = kept;
        let z = pad_pml(lines(&fz, &feat_z, coarse, ratio), opt.pml);
        let insets: Vec<f64> = self
            .sheets
            .iter()
            .map(|sh| {
                let k = z.partition_point(|v| *v < sh.z - 1e-9).clamp(1, z.len() - 2);
                edge::shift(opt.cell, z[k + 1] - z[k], z[k] - z[k - 1]) - self.thickness_growth(sh)
            })
            .collect();
        let edges = self.copper_edges();
        let moved = |axis: usize, list: &[f64]| -> Vec<f64> {
            list.iter()
                .map(|v| {
                    let key = (v * 1e6).round() as i64;
                    match edges[axis].get(&key) {
                        Some((dir, s)) if *dir != 0 => v + *dir as f64 * insets[*s],
                        _ => *v,
                    }
                })
                .collect()
        };
        let x = axis_lines(b.min[0], b.max[0], pinned(&|p| p[0]), &moved(0, &self.features_x));
        let y = axis_lines(b.min[1], b.max[1], pinned(&|p| p[1]), &moved(1, &self.features_y));
        Grid { x, y, z, pml: opt.pml }
    }

    fn thickness_growth(&self, sheet: &Sheet) -> f64 {
        let t = sheet.thickness;
        let h = self
            .sheets
            .iter()
            .map(|o| (o.z - sheet.z).abs())
            .filter(|d| *d > 1e-9)
            .fold(f64::MAX, f64::min);
        if t <= 0.0 || h == f64::MAX {
            return 0.0;
        }
        t / (2.0 * std::f64::consts::PI) * (1.0 + (4.0 * h.max(t) / t).ln())
    }

    pub fn grown(&self, opt: &Meshing, grid: &Grid) -> Vec<f64> {
        let z: Vec<f64> = grid.z.to_vec();
        self.sheets
            .iter()
            .map(|sh| {
                let k = z.partition_point(|v| *v < sh.z - 1e-9).clamp(1, z.len() - 2);
                (self.thickness_growth(sh)
                    - edge::shift(opt.cell, z[k + 1] - z[k], z[k] - z[k - 1]))
                .max(0.0)
                    + 1e-9
            })
            .collect()
    }

    fn copper_edges(&self) -> [std::collections::HashMap<i64, (i8, usize)>; 2] {
        let mut out: [std::collections::HashMap<i64, (i8, usize)>; 2] = Default::default();
        let mut note = |axis: usize, v: f64, dir: i8, s: usize| {
            let e = out[axis].entry((v * 1e6).round() as i64).or_insert((dir, s));
            if e.0 != dir {
                e.0 = 0;
            }
        };
        let eps = 1e-4;
        for (s, c) in &self.copper {
            let rings: Vec<&[P]> = match c {
                Copper::Poly(v) => vec![&v[..]],
                Copper::Fill(z) => z.rings.iter().map(|r| &r[..]).collect(),
                Copper::Seg(a, b, w) => {
                    let hw = w / 2.0;
                    if (a[1] - b[1]).abs() < 1e-9 {
                        note(1, a[1] - hw, 1, *s);
                        note(1, a[1] + hw, -1, *s);
                        note(0, a[0].min(b[0]) - hw, 1, *s);
                        note(0, a[0].max(b[0]) + hw, -1, *s);
                    } else if (a[0] - b[0]).abs() < 1e-9 {
                        note(0, a[0] - hw, 1, *s);
                        note(0, a[0] + hw, -1, *s);
                        note(1, a[1].min(b[1]) - hw, 1, *s);
                        note(1, a[1].max(b[1]) + hw, -1, *s);
                    }
                    continue;
                }
                Copper::Circle(..) => continue,
            };
            for ring in rings {
                for (p, q) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    for axis in 0..2 {
                        let other = 1 - axis;
                        if (p[axis] - q[axis]).abs() > 1e-9 || (p[other] - q[other]).abs() < 1e-6 {
                            continue;
                        }
                        let mut probe = [0.0; 2];
                        probe[other] = 0.5 * (p[other] + q[other]);
                        probe[axis] = p[axis] + eps;
                        let plus = c.contains(probe);
                        probe[axis] = p[axis] - eps;
                        let minus = c.contains(probe);
                        if plus != minus {
                            note(axis, p[axis], if plus { 1 } else { -1 }, *s);
                        }
                    }
                }
            }
        }
        out
    }

    fn sheet_ohms(&self, opt: &Meshing) -> Vec<f64> {
        let f = opt.f0.max(1e6);
        let pi = std::f64::consts::PI;
        let rs = (pi * f * engine::MU0 * crate::loss::COPPER).sqrt();
        let skin = (crate::loss::COPPER / (pi * f * engine::MU0)).sqrt();
        self.sheets
            .iter()
            .map(|s| if s.thickness > 0.0 { rs * self.roughness.factor(skin) } else { 0.0 })
            .collect()
    }

    pub fn build(&self, opt: &Meshing) -> Result<Sim, String> {
        let grid = self.mesh(opt);
        let n = grid.dims();
        let mut mats = Materials::new(&grid);
        let w = 2.0 * std::f64::consts::PI * opt.f0;
        for i in 0..n[0] - 1 {
            let xc = 0.5 * (grid.x[i] + grid.x[i + 1]);
            for j in 0..n[1] - 1 {
                let yc = 0.5 * (grid.y[j] + grid.y[j + 1]);
                if self.outline.len() >= 3 && !geom::point_in_polygon([xc, yc], &self.outline) {
                    continue;
                }
                for k in 0..n[2] - 1 {
                    let zc = 0.5 * (grid.z[k] + grid.z[k + 1]);
                    if let Some(d) = self.dielectrics.iter().find(|d| zc >= d.z0 && zc <= d.z1) {
                        let id = (i * (n[1] - 1) + j) * (n[2] - 1) + k;
                        mats.eps[id] = d.er as f32;
                        mats.sigma[id] = (w * engine::EPS0 * d.er * d.tan) as f32;
                    }
                }
            }
        }
        let ks: Vec<usize> = self.sheets.iter().map(|s| grid.nearest(2, s.z)).collect();
        let (nx, ny) = (n[0], n[1]);
        let grow = self.grown(opt, &grid);
        let mut sheet_x = vec![vec![false; nx * ny]; self.sheets.len()];
        let mut sheet_y = vec![vec![false; nx * ny]; self.sheets.len()];
        for (s, c) in &self.copper {
            let b = c.bounds();
            let i0 = grid.x.partition_point(|v| *v < b.min[0]).saturating_sub(1);
            let i1 = grid.x.partition_point(|v| *v <= b.max[0]).min(nx - 1);
            let j0 = grid.y.partition_point(|v| *v < b.min[1]).saturating_sub(1);
            let j1 = grid.y.partition_point(|v| *v <= b.max[1]).min(ny - 1);
            for i in i0..=i1 {
                for j in j0..=j1 {
                    if i + 1 < nx
                        && !sheet_x[*s][i * ny + j]
                        && c.contains_grown(
                            [0.5 * (grid.x[i] + grid.x[i + 1]), grid.y[j]],
                            grow[*s],
                        )
                    {
                        sheet_x[*s][i * ny + j] = true;
                    }
                    if j + 1 < ny
                        && !sheet_y[*s][i * ny + j]
                        && c.contains_grown(
                            [grid.x[i], 0.5 * (grid.y[j] + grid.y[j + 1])],
                            grow[*s],
                        )
                    {
                        sheet_y[*s][i * ny + j] = true;
                    }
                }
            }
        }
        let mut via_edges: std::collections::HashSet<(usize, usize, usize)> = Default::default();
        for (c, _, a, b) in &self.vias {
            let (i, j) = (grid.nearest(0, c[0]), grid.nearest(1, c[1]));
            let (lo, hi) = (ks[*a].min(ks[*b]), ks[*a].max(ks[*b]));
            for k in lo..hi {
                via_edges.insert((i, j, k));
            }
        }
        let mut ports = Vec::new();
        for p in &self.ports {
            let (i, j) = (grid.nearest(0, p.at[0]), grid.nearest(1, p.at[1]));
            let (a, b) = (ks[p.reference], ks[p.sheet]);
            let (lo, hi) = (a.min(b), a.max(b));
            if lo == hi {
                return Err(format!(
                    "port {} has no height, its layers mesh to the same plane",
                    p.name
                ));
            }
            let mut nodes: Vec<(usize, usize)> = Vec::new();
            if p.area.len() >= 3 {
                for (ii, x) in grid.x.iter().enumerate() {
                    for (jj, y) in grid.y.iter().enumerate() {
                        if geom::point_in_polygon([*x, *y], &p.area)
                            && !via_edges.contains(&(ii, jj, lo))
                        {
                            nodes.push((ii, jj));
                        }
                    }
                }
            }
            if nodes.is_empty() {
                nodes.push((i, j));
            }
            let columns: Vec<Vec<Edge>> = nodes
                .iter()
                .map(|(ii, jj)| (lo..hi).map(|k| Edge { comp: 2, at: [*ii, *jj, k] }).collect())
                .collect();
            ports.push(PortDef { name: p.name.clone(), columns, r: p.r });
        }
        let mut lumped = Vec::new();
        for e in &self.elements {
            let k = ks[e.sheet];
            let (dx, dy) = ((e.b[0] - e.a[0]).abs(), (e.b[1] - e.a[1]).abs());
            let comp = if dx >= dy { 0 } else { 1 };
            let (ia, ib) = (grid.nearest(comp, e.a[comp]), grid.nearest(comp, e.b[comp]));
            let other = 1 - comp;
            let row = grid.nearest(other, 0.5 * (e.a[other] + e.b[other]));
            let sheet = if comp == 0 { &sheet_x[e.sheet] } else { &sheet_y[e.sheet] };
            let edges: Vec<Edge> = (ia.min(ib)..ia.max(ib))
                .map(|t| if comp == 0 { [t, row, k] } else { [row, t, k] })
                .filter(|at| !sheet[at[0] * ny + at[1]])
                .map(|at| Edge { comp, at })
                .collect();
            if edges.is_empty() {
                return Err(format!(
                    "{}: the mesh leaves no gap between its pads, use a finer cell",
                    e.name
                ));
            }
            lumped.push(Lumped { name: e.name.clone(), edges, element: e.element });
        }
        let in_pml = |i: usize, len: usize| i < grid.pml || i + 1 + grid.pml >= len;
        let mut resistive: Vec<(usize, usize, f64, bool)> = Vec::new();
        for (s, rs) in self.sheet_ohms(opt).into_iter().enumerate() {
            if rs <= 0.0 {
                continue;
            }
            let k = ks[s];
            let thick = self.sheets[s].thickness;
            let (dz_above, dz_below) = (grid.z[k + 1] - grid.z[k], grid.z[k] - grid.z[k - 1]);
            for comp in 0..2 {
                let (along_axis, across_axis) =
                    if comp == 0 { (&grid.x, &grid.y) } else { (&grid.y, &grid.x) };
                let flags = if comp == 0 { &sheet_x[s] } else { &sheet_y[s] };
                let (na, nc) = (along_axis.len(), across_axis.len());
                for a in 0..na.saturating_sub(1) {
                    for c in 0..nc.saturating_sub(1) {
                        let (i, j) = if comp == 0 { (a, c) } else { (c, a) };
                        if !flags[i * ny + j] || in_pml(i, nx) || in_pml(j, ny) {
                            continue;
                        }
                        let flag = |cc: usize| {
                            let (ii, jj) = if comp == 0 { (a, cc) } else { (cc, a) };
                            flags[ii * ny + jj]
                        };
                        let run = |dir: isize| {
                            let mut k = 0;
                            let mut cc = c as isize;
                            while k < edge::BANDS {
                                cc += dir;
                                if cc < 0 || cc as usize + 1 >= nc || !flag(cc as usize) {
                                    break;
                                }
                                k += 1;
                            }
                            k
                        };
                        let along = along_axis[a + 1] - along_axis[a];
                        let id = engine::idx(n, i, j, k);
                        let (before, after) = (run(-1), run(1));
                        if before == 0 && after == 0 {
                            let across = across_axis[c + 1] - across_axis[c];
                            resistive.push((comp, id, rs * along / across, true));
                            continue;
                        }
                        let (band, inward) =
                            if before <= after { (before, 1isize) } else { (after, -1) };
                        let cell = |cc: usize| across_axis[cc + 1] - across_axis[cc];
                        let d = if inward > 0 { cell(c) } else { cell(c - 1) };
                        let dual = if before > 0 && after > 0 {
                            0.5 * (cell(c - 1) + cell(c))
                        } else {
                            0.5 * d
                        };
                        let r0 = rs * along / dual;
                        if band >= edge::BANDS || before.min(after) >= edge::BANDS {
                            resistive.push((comp, id, r0, true));
                            continue;
                        }
                        let w = edge::weights(d, dz_above, dz_below, thick);
                        resistive.push((comp, id, r0 * w.bands[band], band > 0));
                    }
                }
            }
        }
        let resistive_set: std::collections::HashSet<(usize, usize)> =
            resistive.iter().map(|(c, id, _, _)| (*c, *id)).collect();
        let lumped_edges: std::collections::HashSet<(usize, [usize; 3])> =
            lumped.iter().flat_map(|l| l.edges.iter().map(|e| (e.comp, e.at))).collect();
        let sheet_of_k: Vec<Option<usize>> =
            (0..n[2]).map(|k| ks.iter().position(|kk| *kk == k)).collect();
        let pec = |c: usize, at: [usize; 3]| -> bool {
            if lumped_edges.contains(&(c, at)) {
                return false;
            }
            match c {
                2 => via_edges.contains(&(at[0], at[1], at[2])),
                _ => match sheet_of_k[at[2]] {
                    Some(s) => {
                        let id = at[0] * ny + at[1];
                        let on = if c == 0 { sheet_x[s][id] } else { sheet_y[s][id] };
                        on && !resistive_set.contains(&(c, engine::idx(n, at[0], at[1], at[2])))
                    }
                    None => false,
                },
            }
        };
        let metres = Grid {
            x: grid.x.iter().map(|v| v * 1e-3).collect(),
            y: grid.y.iter().map(|v| v * 1e-3).collect(),
            z: grid.z.iter().map(|v| v * 1e-3).collect(),
            pml: grid.pml,
        };
        let pi = std::f64::consts::PI;
        let thinnest =
            self.sheets.iter().map(|s| s.thickness).filter(|t| *t > 0.0).fold(f64::MAX, f64::min);
        let dc = if thinnest < f64::MAX {
            let t = thinnest * 1e-3;
            crate::loss::COPPER / (engine::MU0 * t * t)
        } else {
            0.0
        };
        let omega0 = 2.0 * pi * opt.f0.max(1e6);
        let band = engine::SkinBand {
            omega0,
            x_min: (omega0 / 1000.0).max(dc),
            x_max: 2.0 * pi * opt.f_max.max(opt.f0) * 100.0,
        };
        Ok(Sim::new(metres, &mats, &pec, &lumped, ports, &resistive, band))
    }
}

fn pad_pml(mut line: Vec<f64>, pml: usize) -> Vec<f64> {
    let first = line[1] - line[0];
    let last = line[line.len() - 1] - line[line.len() - 2];
    let mut pre: Vec<f64> = (1..=pml).rev().map(|i| line[0] - first * i as f64).collect();
    let post: Vec<f64> = (1..=pml).map(|i| line[line.len() - 1] + last * i as f64).collect();
    pre.append(&mut line);
    pre.extend(post);
    pre
}

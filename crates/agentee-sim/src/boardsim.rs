use crate::fdtd::model::{Copper, PcbModel};
use crate::nodal::Problem;
use agentee_core::board::{Board, LayerKind};
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::layout::Layout;
use agentee_core::sim::{LayerMap, MapGrid, MapResult, PadRef, Reading, Sim, SimKind};

const SIGMA_CU: f64 = 5.8e7;
const K_CU: f64 = 390.0;
const K_FR4_PLANE: f64 = 0.8;
const K_FR4_THROUGH: f64 = 0.3;
const PLATING_MM: f64 = 0.025;

pub struct Raster {
    pub origin: P,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    pub copper: Vec<Vec<bool>>,
    pub inside: Vec<bool>,
    pub thickness: Vec<f64>,
}

impl Raster {
    pub fn at(&self, i: usize, j: usize) -> P {
        [self.origin[0] + i as f64 * self.cell, self.origin[1] + j as f64 * self.cell]
    }

    pub fn node(&self, p: P) -> (usize, usize) {
        let i =
            ((p[0] - self.origin[0]) / self.cell).round().clamp(0.0, self.nx as f64 - 1.0) as usize;
        let j =
            ((p[1] - self.origin[1]) / self.cell).round().clamp(0.0, self.ny as f64 - 1.0) as usize;
        (i, j)
    }

    pub fn nodes_in(&self, polys: &[Vec<P>]) -> Vec<(usize, usize)> {
        let mut b = Bounds::EMPTY;
        polys.iter().flatten().for_each(|q| b.add(*q));
        if b.is_empty() {
            return Vec::new();
        }
        let (i0, j0) = self.node(b.min);
        let (i1, j1) = self.node(b.max);
        let mut out = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                if polys.iter().any(|o| geom::point_in_polygon(self.at(i, j), o)) {
                    out.push((i, j));
                }
            }
        }
        if out.is_empty() {
            out.push(self.node(b.center()));
        }
        out
    }
}

pub fn raster(model: &PcbModel, board: &Board, cell: f64) -> Raster {
    let mut b = Bounds::EMPTY;
    model.outline.iter().for_each(|p| b.add(*p));
    if b.is_empty() {
        for (_, c) in &model.copper {
            b.union(&copper_bounds(c));
        }
    }
    let origin = b.min;
    let nx = ((b.max[0] - b.min[0]) / cell).ceil() as usize + 1;
    let ny = ((b.max[1] - b.min[1]) / cell).ceil() as usize + 1;
    let at = |i: usize, j: usize| [origin[0] + i as f64 * cell, origin[1] + j as f64 * cell];
    let inside: Vec<bool> = (0..nx * ny)
        .map(|k| {
            model.outline.len() < 3 || geom::point_in_polygon(at(k / ny, k % ny), &model.outline)
        })
        .collect();
    let mut copper = vec![vec![false; nx * ny]; model.sheets.len()];
    for (s, c) in &model.copper {
        let bb = copper_bounds(c);
        let i0 = (((bb.min[0] - origin[0]) / cell).floor().max(0.0)) as usize;
        let j0 = (((bb.min[1] - origin[1]) / cell).floor().max(0.0)) as usize;
        let i1 = ((((bb.max[0] - origin[0]) / cell).ceil()) as usize).min(nx - 1);
        let j1 = ((((bb.max[1] - origin[1]) / cell).ceil()) as usize).min(ny - 1);
        for i in i0..=i1 {
            for j in j0..=j1 {
                let k = i * ny + j;
                if !copper[*s][k] && contains(c, at(i, j)) {
                    copper[*s][k] = true;
                }
            }
        }
    }
    let thickness = model
        .sheets
        .iter()
        .map(|s| {
            board
                .stackup
                .layers
                .iter()
                .find(|l| l.name == s.name && l.kind == LayerKind::Copper)
                .map(|l| l.thickness.to_mm())
                .unwrap_or(0.035)
        })
        .collect();
    Raster { origin, cell, nx, ny, copper, inside, thickness }
}

fn contains(c: &Copper, p: P) -> bool {
    c.contains(p)
}

fn copper_bounds(c: &Copper) -> Bounds {
    c.bounds()
}

fn barrel_area_m2(r_mm: f64) -> f64 {
    let (ro, ri) = (r_mm * 1e-3, (r_mm - PLATING_MM).max(0.0) * 1e-3);
    std::f64::consts::PI * (ro * ro - ri * ri)
}

fn pad_sheets(layout: &Layout, model: &PcbModel, p: &PadRef) -> Vec<usize> {
    layout.parts[p.part].pads[p.pad]
        .copper
        .iter()
        .filter_map(|l| model.sheets.iter().position(|s| &s.name == l))
        .collect()
}

fn pad_nodes(layout: &Layout, r: &Raster, p: &PadRef) -> Vec<(usize, usize)> {
    r.nodes_in(&layout.parts[p.part].pads[p.pad].outlines)
}

pub fn dc(layout: &Layout, board: &Board, spec: &Sim, hash: u64) -> Result<MapResult, String> {
    let t0 = std::time::Instant::now();
    let model = PcbModel::geometry(layout, board);
    let r = raster(&model, board, spec.cell);
    let ns = model.sheets.len();
    let mut p = Problem::new([r.nx, r.ny, ns]);
    let id = |i: usize, j: usize, s: usize| (i * r.ny + j) * ns + s;
    for s in 0..ns {
        let g = SIGMA_CU * r.thickness[s] * 1e-3;
        for i in 0..r.nx {
            for j in 0..r.ny {
                if !r.copper[s][i * r.ny + j] {
                    continue;
                }
                p.g0[id(i, j, s)] = 1e-9;
                if i + 1 < r.nx && r.copper[s][(i + 1) * r.ny + j] {
                    p.gx[id(i, j, s)] = g as f32;
                }
                if j + 1 < r.ny && r.copper[s][i * r.ny + j + 1] {
                    p.gy[id(i, j, s)] = g as f32;
                }
            }
        }
    }
    let mut vias = Vec::new();
    for (c, rad, a, b) in &model.vias {
        let (i, j) = r.node(*c);
        let (lo, hi) = (*a.min(b), *a.max(b));
        for s in lo..hi {
            let len = (model.sheets[s].z - model.sheets[s + 1].z).abs() * 1e-3;
            let g = SIGMA_CU * barrel_area_m2(*rad) / len.max(1e-6);
            let k = id(i, j, s);
            p.gz[k] += g as f32;
        }
        vias.push((*c, i, j, lo, hi));
    }
    let centre = |pr: &PadRef| -> Option<usize> {
        let sheets = pad_sheets(layout, &model, pr);
        let s = *sheets.first()?;
        let nodes = pad_nodes(layout, &r, pr);
        let c = r.node({
            let mut b = Bounds::EMPTY;
            layout.parts[pr.part].pads[pr.pad].outlines.iter().flatten().for_each(|q| b.add(*q));
            b.center()
        });
        let pick =
            nodes.iter().copied().filter(|(i, j)| r.copper[s][i * r.ny + j]).min_by_key(
                |(i, j)| (*i as i64 - c.0 as i64).abs() + (*j as i64 - c.1 as i64).abs(),
            );
        pick.map(|(i, j)| id(i, j, s))
    };
    let pad_copper = |pr: &PadRef| -> Vec<usize> {
        let sheets = pad_sheets(layout, &model, pr);
        let Some(&s) = sheets.first() else { return Vec::new() };
        pad_nodes(layout, &r, pr)
            .into_iter()
            .filter(|(i, j)| r.copper[s][i * r.ny + j])
            .map(|(i, j)| id(i, j, s))
            .collect()
    };
    for l in &spec.links {
        let (a, b) = (pad_copper(&l.a), pad_copper(&l.b));
        if a.is_empty() || b.is_empty() {
            if let (Some(x), Some(y)) = (centre(&l.a), centre(&l.b)) {
                p.links.push((x, y, (1.0 / l.ohms) as f32));
            }
            continue;
        }
        let n = a.len().max(b.len());
        for k in 0..n {
            p.links.push((a[k % a.len()], b[k % b.len()], (1.0 / l.ohms / n as f64) as f32));
        }
    }
    let mut fixed_nodes = Vec::new();
    for s in &spec.supplies {
        for sh in pad_sheets(layout, &model, &s.pad) {
            for (i, j) in pad_nodes(layout, &r, &s.pad) {
                if r.copper[sh][i * r.ny + j] {
                    p.fixed[id(i, j, sh)] = Some(s.volts as f32);
                    fixed_nodes.push(id(i, j, sh));
                }
            }
        }
    }
    let spread = |p: &mut Problem, pr: &PadRef, amps: f64| {
        let sheets = pad_sheets(layout, &model, pr);
        let Some(&s) = sheets.first() else { return };
        let nodes: Vec<(usize, usize)> = pad_nodes(layout, &r, pr)
            .into_iter()
            .filter(|(i, j)| r.copper[s][i * r.ny + j])
            .collect();
        for (i, j) in &nodes {
            p.source[id(*i, *j, s)] += (amps / nodes.len().max(1) as f64) as f32;
        }
    };
    for l in &spec.loads {
        spread(&mut p, &l.pad, -l.amps);
        if let Some(ret) = &l.return_pad {
            spread(&mut p, ret, l.amps);
        }
    }
    let len = p.len();
    let mut parent: Vec<usize> = (0..len).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (ra, rb) = (root(parent, a), root(parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    };
    for n in 0..len {
        let (i, rest) = (n / (r.ny * ns), n % (r.ny * ns));
        let (j, s) = (rest / ns, rest % ns);
        if p.gx[n] > 0.0 && i + 1 < r.nx {
            join(&mut parent, n, id(i + 1, j, s));
        }
        if p.gy[n] > 0.0 && j + 1 < r.ny {
            join(&mut parent, n, id(i, j + 1, s));
        }
        if p.gz[n] > 0.0 && s + 1 < ns {
            join(&mut parent, n, id(i, j, s + 1));
        }
    }
    for (a, b, _) in p.links.clone() {
        join(&mut parent, a, b);
    }
    let mut level = std::collections::HashMap::new();
    for n in 0..len {
        if let Some(v) = p.fixed[n] {
            let rt = root(&mut parent, n);
            level.entry(rt).or_insert(v);
        }
    }
    p.offset = (0..len).map(|n| *level.get(&root(&mut parent, n)).unwrap_or(&0.0)).collect();
    for n in 0..len {
        if p.fixed[n].is_none() && !level.contains_key(&root(&mut parent, n)) {
            p.fixed[n] = Some(f32::NAN);
        }
        p.g0[n] = 0.0;
    }
    let sol = p.solve_direct()?;
    let v = &sol.phi;
    let dv = &sol.delta;
    let grid = MapGrid { origin: r.origin, cell: r.cell, width: r.nx, height: r.ny };
    let mut maps = Vec::new();
    let mut drops = Vec::new();
    let mut volts_maps = Vec::new();
    let mut j_max = (0.0f64, String::new());
    for s in 0..ns {
        let mut volts = vec![f32::NAN; r.nx * r.ny];
        let mut dens = vec![f32::NAN; r.nx * r.ny];
        let mut any = false;
        for i in 0..r.nx {
            for j in 0..r.ny {
                if !r.copper[s][i * r.ny + j] {
                    continue;
                }
                any = true;
                let k = id(i, j, s);
                volts[j * r.nx + i] = v[k];
                let grad = |a: usize, b: usize| (dv[a] - dv[b]) / (r.cell * 1e-3);
                let gx = match (
                    i + 1 < r.nx && r.copper[s][(i + 1) * r.ny + j],
                    i > 0 && r.copper[s][(i - 1) * r.ny + j],
                ) {
                    (true, true) => 0.5 * (grad(id(i + 1, j, s), k) + grad(k, id(i - 1, j, s))),
                    (true, false) => grad(id(i + 1, j, s), k),
                    (false, true) => grad(k, id(i - 1, j, s)),
                    _ => 0.0,
                };
                let gy = match (
                    j + 1 < r.ny && r.copper[s][i * r.ny + j + 1],
                    j > 0 && r.copper[s][i * r.ny + j - 1],
                ) {
                    (true, true) => 0.5 * (grad(id(i, j + 1, s), k) + grad(k, id(i, j - 1, s))),
                    (true, false) => grad(id(i, j + 1, s), k),
                    (false, true) => grad(k, id(i, j - 1, s)),
                    _ => 0.0,
                };
                let jd = SIGMA_CU * (gx * gx + gy * gy).sqrt() / 1e6;
                dens[j * r.nx + i] = jd as f32;
                if jd > j_max.0 {
                    let at = r.at(i, j);
                    j_max =
                        (jd, format!("{} at [{:.2}, {:.2}]", model.sheets[s].name, at[0], at[1]));
                }
            }
        }
        if any {
            let name = &model.sheets[s].name;
            let drop: Vec<f32> = (0..r.nx * r.ny)
                .map(|k| {
                    let (i, j) = (k % r.nx, k / r.nx);
                    let n = id(i, j, s);
                    if volts[k].is_finite() { (dv[n].abs() * 1e3) as f32 } else { f32::NAN }
                })
                .collect();
            drops.push(LayerMap::encode(name, "drop", "mV", grid, &drop));
            maps.push(LayerMap::encode(name, "current density", "A/mm2", grid, &dens));
            volts_maps.push(LayerMap::encode(name, "voltage", "V", grid, &volts));
        }
    }
    drops.extend(maps);
    drops.extend(volts_maps);
    let maps = drops;
    let mean = |pr: &PadRef| -> f64 {
        let sheets = pad_sheets(layout, &model, pr);
        let Some(&s) = sheets.first() else { return f64::NAN };
        let vals: Vec<f64> = pad_nodes(layout, &r, pr)
            .iter()
            .filter(|(i, j)| r.copper[s][i * r.ny + j])
            .map(|(i, j)| v[id(*i, *j, s)] as f64)
            .collect();
        vals.iter().sum::<f64>() / vals.len().max(1) as f64
    };
    let mut readings = Vec::new();
    let flux = |n: usize| -> f64 {
        let (i, rest) = (n / (r.ny * ns), n % (r.ny * ns));
        let (j, s) = (rest / ns, rest % ns);
        let mut out = 0.0;
        let mut add = |m: usize, g: f32| {
            if g > 0.0 && dv[m].is_finite() {
                out += g as f64 * (dv[n] - dv[m]);
            }
        };
        if i + 1 < r.nx {
            add(id(i + 1, j, s), p.gx[n]);
        }
        if i > 0 {
            add(id(i - 1, j, s), p.gx[id(i - 1, j, s)]);
        }
        if j + 1 < r.ny {
            add(id(i, j + 1, s), p.gy[n]);
        }
        if j > 0 {
            add(id(i, j - 1, s), p.gy[id(i, j - 1, s)]);
        }
        if s + 1 < ns {
            add(id(i, j, s + 1), p.gz[n]);
        }
        if s > 0 {
            add(id(i, j, s - 1), p.gz[id(i, j, s - 1)]);
        }
        for (a, b, g) in &p.links {
            if *a == n {
                add(*b, *g);
            }
            if *b == n {
                add(*a, *g);
            }
        }
        out
    };
    for s in &spec.supplies {
        let mut nodes = Vec::new();
        for sh in pad_sheets(layout, &model, &s.pad) {
            for (i, j) in pad_nodes(layout, &r, &s.pad) {
                if r.copper[sh][i * r.ny + j] {
                    nodes.push(id(i, j, sh));
                }
            }
        }
        let amps: f64 = nodes.iter().map(|n| flux(*n)).sum();
        readings.push(Reading {
            label: format!("supply {}", s.pad.label),
            value: s.volts,
            unit: "V".into(),
            detail: format!("sources {:.2} mA", amps * 1e3),
        });
    }
    for l in &spec.loads {
        let at = mean(&l.pad);
        let ret = l.return_pad.as_ref().map(mean).unwrap_or(0.0);
        readings.push(Reading {
            label: format!("load {}", l.pad.label),
            value: at - ret,
            unit: "V".into(),
            detail: format!("{} mA, pad {:.4} V, return {:.4} V", l.amps * 1e3, at, ret),
        });
    }
    readings.push(Reading {
        label: "peak current density".into(),
        value: j_max.0,
        unit: "A/mm2".into(),
        detail: j_max.1,
    });
    let mut via_i: Vec<(f64, String)> = vias
        .iter()
        .map(|(c, i, j, lo, _)| {
            let k = id(*i, *j, *lo);
            let i_amp = p.gz[k] as f64 * (dv[k] - dv[k + 1]);
            (i_amp.abs(), format!("via [{:.2}, {:.2}]", c[0], c[1]))
        })
        .collect();
    via_i.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (amps, label) in via_i.into_iter().take(5).filter(|x| x.0 > 1e-6) {
        readings.push(Reading {
            label,
            value: amps * 1e3,
            unit: "mA".into(),
            detail: "busiest vias".into(),
        });
    }
    Ok(MapResult {
        name: spec.name.clone(),
        kind: SimKind::Dc,
        maps,
        readings,
        iterations: sol.iterations,
        residual: sol.residual,
        cells: p.len(),
        seconds: t0.elapsed().as_secs_f64(),
        device: sol.device,
        spec_hash: hash,
    })
}

pub fn thermal(layout: &Layout, board: &Board, spec: &Sim, hash: u64) -> Result<MapResult, String> {
    let t0 = std::time::Instant::now();
    let model = PcbModel::geometry(layout, board);
    let r = raster(&model, board, spec.cell);
    let mut zs: Vec<(f64, Option<usize>)> = Vec::new();
    for (s, sh) in model.sheets.iter().enumerate() {
        if let Some((prev, _)) = zs.last().copied() {
            let gap = prev - sh.z;
            let m = (gap / 0.1).ceil().max(1.0) as usize;
            for k in 1..m {
                zs.push((prev - gap * k as f64 / m as f64, None));
            }
        }
        zs.push((sh.z, Some(s)));
    }
    let nz = zs.len();
    let slab: Vec<f64> = (0..nz)
        .map(|k| {
            let up = if k > 0 { (zs[k - 1].0 - zs[k].0) / 2.0 } else { 0.0 };
            let dn = if k + 1 < nz { (zs[k].0 - zs[k + 1].0) / 2.0 } else { 0.0 };
            (up + dn) * 1e-3
        })
        .collect();
    let mut p = Problem::new([r.nx, r.ny, nz]);
    p.reference = spec.ambient as f32;
    let id = |i: usize, j: usize, k: usize| (i * r.ny + j) * nz + k;
    let cell_m = r.cell * 1e-3;
    for i in 0..r.nx {
        for j in 0..r.ny {
            if !r.inside[i * r.ny + j] {
                continue;
            }
            let right = i + 1 < r.nx && r.inside[(i + 1) * r.ny + j];
            let down = j + 1 < r.ny && r.inside[i * r.ny + j + 1];
            for k in 0..nz {
                let mut g = K_FR4_PLANE * slab[k];
                let cu = |ii: usize, jj: usize| {
                    zs[k].1.map(|s| r.copper[s][ii * r.ny + jj]).unwrap_or(false)
                };
                let t_cu = zs[k].1.map(|s| r.thickness[s] * 1e-3).unwrap_or(0.0);
                if right {
                    let gg = g + if cu(i, j) && cu(i + 1, j) { K_CU * t_cu } else { 0.0 };
                    p.gx[id(i, j, k)] = gg as f32;
                }
                if down {
                    let gg = g + if cu(i, j) && cu(i, j + 1) { K_CU * t_cu } else { 0.0 };
                    p.gy[id(i, j, k)] = gg as f32;
                }
                if k + 1 < nz {
                    let dz = (zs[k].0 - zs[k + 1].0) * 1e-3;
                    g = K_FR4_THROUGH * cell_m * cell_m / dz.max(1e-9);
                    p.gz[id(i, j, k)] = g as f32;
                }
            }
            p.g0[id(i, j, 0)] = (spec.h_top * cell_m * cell_m) as f32;
            p.g0[id(i, j, nz - 1)] = (spec.h_bottom * cell_m * cell_m) as f32;
        }
    }
    let sheet_k: Vec<usize> =
        (0..model.sheets.len()).map(|s| zs.iter().position(|z| z.1 == Some(s)).unwrap()).collect();
    for (c, rad, a, b) in &model.vias {
        let (i, j) = r.node(*c);
        let (ka, kb) = (sheet_k[*a].min(sheet_k[*b]), sheet_k[*a].max(sheet_k[*b]));
        for k in ka..kb {
            let dz = (zs[k].0 - zs[k + 1].0) * 1e-3;
            p.gz[id(i, j, k)] += (K_CU * barrel_area_m2(*rad) / dz.max(1e-9)) as f32;
        }
    }
    let mut source_nodes: Vec<Vec<usize>> = Vec::new();
    for h in &spec.sources {
        let part = &layout.parts[h.part];
        let mut nodes = Vec::new();
        for pad in part
            .pads
            .iter()
            .filter(|q| !q.copper.is_empty() && (h.pads.is_empty() || h.pads.contains(&q.number)))
        {
            let Some(s) = model.sheets.iter().position(|sh| sh.name == pad.copper[0]) else {
                continue;
            };
            for (i, j) in r.nodes_in(&pad.outlines) {
                nodes.push(id(i, j, sheet_k[s]));
            }
        }
        if nodes.is_empty() {
            return Err(format!("{} has no copper pads to put its heat into", h.reference));
        }
        for n in &nodes {
            p.source[*n] += (h.watts / nodes.len() as f64) as f32;
        }
        source_nodes.push(nodes);
    }
    let sol = p.solve(1e-7, 50_000);
    let t = &sol.phi;
    let grid = MapGrid { origin: r.origin, cell: r.cell, width: r.nx, height: r.ny };
    let mut maps = Vec::new();
    let mut peak = (f64::MIN, String::new());
    for (s, sh) in model.sheets.iter().enumerate() {
        let k = sheet_k[s];
        let mut vals = vec![f32::NAN; r.nx * r.ny];
        for i in 0..r.nx {
            for j in 0..r.ny {
                if !r.inside[i * r.ny + j] {
                    continue;
                }
                let v = t[id(i, j, k)];
                vals[j * r.nx + i] = v;
                if v as f64 > peak.0 {
                    let at = r.at(i, j);
                    peak = (v as f64, format!("{} at [{:.2}, {:.2}]", sh.name, at[0], at[1]));
                }
            }
        }
        maps.push(LayerMap::encode(&sh.name, "temperature", "C", grid, &vals));
    }
    let mut readings = vec![Reading {
        label: "board peak".into(),
        value: peak.0,
        unit: "C".into(),
        detail: peak.1,
    }];
    for (h, nodes) in spec.sources.iter().zip(&source_nodes) {
        let lead = nodes.iter().map(|n| t[*n] as f64).fold(f64::MIN, f64::max);
        readings.push(Reading {
            label: format!("{} pads", h.reference),
            value: lead,
            unit: "C".into(),
            detail: format!("{} W", h.watts),
        });
        if let Some(th) = h.theta_jc {
            readings.push(Reading {
                label: format!("{} junction", h.reference),
                value: lead + h.watts * th,
                unit: "C".into(),
                detail: format!("pads + {} W x {} C/W", h.watts, th),
            });
        }
    }
    readings.push(Reading {
        label: "ambient".into(),
        value: spec.ambient,
        unit: "C".into(),
        detail: String::new(),
    });
    Ok(MapResult {
        name: spec.name.clone(),
        kind: SimKind::Thermal,
        maps,
        readings,
        iterations: sol.iterations,
        residual: sol.residual,
        cells: p.len(),
        seconds: t0.elapsed().as_secs_f64(),
        device: sol.device,
        spec_hash: hash,
    })
}

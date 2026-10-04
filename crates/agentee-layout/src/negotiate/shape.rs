use super::grid::{Grid, edge_dist};
use super::rules::{NetRule, Rules};
use super::search::{DIRS, Found, seg_cells};
use super::soft::{Piece, Run, Soft};
use agentee_core::geom::{self, P};

const REACH_CENTRE: f64 = 0.6;

#[derive(Clone, Debug)]
pub struct Neck {
    pub layer: usize,
    pub from: P,
    pub to: P,
    pub width: f64,
}

#[derive(Clone, Debug)]
pub struct Access {
    pub neck: Option<Neck>,
    pub via: Option<(P, usize)>,
    pub cost: f32,
}

#[derive(Clone)]
pub struct PadRef {
    pub layers: Vec<usize>,
    pub outline: Vec<P>,
    pub centre: P,
    pub pitch: Option<f64>,
}

pub struct Ctx<'a> {
    pub grid: &'a Grid,
    pub soft: &'a Soft,
    pub rules: &'a Rules,
    pub rule: &'a NetRule,
    pub net: u16,
    pub entry: &'a [P],
    pub entry_r: f64,
    pub own: &'a std::collections::HashSet<u32>,
}

impl Ctx<'_> {
    fn planar(&self, l: usize, p: P) -> bool {
        self.rule.track[l] || self.entry.iter().any(|e| geom::dist(*e, p) <= self.entry_r)
    }

    fn ok(&self, l: usize, x: i64, y: i64) -> bool {
        if !self.grid.inside(x, y) {
            return false;
        }
        let (x, y) = (x as usize, y as usize);
        let i = self.grid.idx(l, x, y);
        self.grid.track_ok(i, self.net, self.rule.need[l])
            && (self.own.contains(&(i as u32))
                || self.rule.bucket[l]
                    .is_none_or(|b| self.soft.tracks[b][y * self.grid.w + x] == 0))
    }

    pub fn clear_line(&self, l: usize, p: P, q: P) -> bool {
        let g = self.grid;
        let n = (geom::dist(p, q) / (g.g * 0.25)).ceil().max(1.0) as usize;
        (0..=n).all(|k| {
            let t = k as f64 / n as f64;
            let at = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            if !self.planar(l, at) {
                return false;
            }
            let (x, y) = g.cell(at);
            if self.ok(l, x, y) {
                return true;
            }
            let (fx, fy) = ((at[0] - g.x0) / g.g - 0.5, (at[1] - g.y0) / g.g - 0.5);
            let (cx, cy) = (fx.floor() as i64, fy.floor() as i64);
            [(cx, cy), (cx + 1, cy), (cx, cy + 1), (cx + 1, cy + 1)].into_iter().any(|(x, y)| {
                let c = [g.x0 + (x as f64 + 0.5) * g.g, g.y0 + (y as f64 + 0.5) * g.g];
                geom::dist(c, at) <= g.g * 0.6 && self.ok(l, x, y)
            })
        })
    }

    pub fn necks(&self, pad: &PadRef, l: usize) -> Vec<Neck> {
        let g = self.grid;
        let wide = self.rule.width[l];
        let c = self.rule.clearance;
        let slack = self.rules.slack;
        let p = pad.centre;
        let mut out = Vec::new();
        let room = |at: P| -> f64 {
            let (x, y) = g.cell(at);
            if !g.inside(x, y) {
                return 0.0;
            }
            let i = g.idx(l, x as usize, y as usize);
            let (d, q) = g.clearance_at(i, self.net);
            (d - c).min(q) - slack - 0.2 * g.g
        };
        let min_half = self.rules.min_width / 2.0 - 1e-9;
        let inside = |q: P| geom::point_in_polygon(q, &pad.outline);
        for (dx, dy) in DIRS {
            let norm = ((dx * dx + dy * dy) as f64).sqrt();
            let u = [dx as f64 / norm, dy as f64 / norm];
            let mut half = wide / 2.0;
            let mut from: Option<P> = None;
            let mut k = 0;
            while k as f64 * g.g <= self.rule.neck + 1e-9 {
                let len = k as f64 * g.g;
                k += 1;
                let q = [p[0] + u[0] * len, p[1] + u[1] * len];
                let r = room(q);
                if from.is_none() {
                    if !inside(q) {
                        break;
                    }
                    if r >= min_half {
                        from = Some(q);
                        half = half.min(r);
                    }
                    continue;
                }
                half = half.min(r);
                if half < min_half {
                    break;
                }
                if inside(q) && edge_dist(&pad.outline, q) > g.g {
                    continue;
                }
                let (x, y) = g.cell(q);
                if !self.ok(l, x, y) {
                    continue;
                }
                let width = (2.0 * half).min(wide);
                let width = (width * 1000.0).floor() / 1000.0;
                out.push(Neck { layer: l, from: from.unwrap_or(p), to: q, width });
                break;
            }
        }
        out
    }

    pub fn neck_width_hard(&self, l: usize, a: P, b: P, hard: bool) -> Option<f64> {
        let g = self.grid;
        let n = (geom::dist(a, b) / (g.g * 0.5)).ceil().max(1.0) as usize;
        let mut room = f64::MAX;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let at = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (x, y) = g.cell(at);
            if !g.inside(x, y) {
                return None;
            }
            let (x, y) = (x as usize, y as usize);
            let i = g.idx(l, x, y);
            let (d, q) = g.clearance_at(i, self.net);
            room = room.min((d - self.rule.clearance).min(q) - self.rules.slack - 0.2 * g.g);
            if hard
                && !self.own.contains(&(i as u32))
                && self.rule.bucket[l].is_some_and(|bk| self.soft.tracks[bk][y * g.w + x] > 0)
            {
                return None;
            }
        }
        if 2.0 * room >= self.rule.width[l] {
            return Some(self.rule.width[l]);
        }
        let w = (2.0 * room * 10000.0).round() / 10000.0;
        (w >= self.rules.min_width - 1e-9).then_some(w)
    }

    fn neck_load(&self, l: usize, a: P, b: P) -> f32 {
        let Some(bk) = self.rule.bucket[l] else { return 0.0 };
        let g = self.grid;
        seg_cells(g, a, b)
            .into_iter()
            .filter(|&(x, y)| !self.own.contains(&(g.idx(l, x, y) as u32)))
            .map(|(x, y)| self.soft.tracks[bk][y * g.w + x] as f32)
            .sum()
    }

    fn via_load(&self, vi: usize, c2: usize) -> f32 {
        self.soft.vias[self.rule.via_bucket[vi]][c2] as f32
    }

    pub fn stubs(
        &self,
        pad: &PadRef,
        l: usize,
        hard: bool,
        via_cost: f64,
        pres: f32,
    ) -> Vec<Access> {
        let g = self.grid;
        let c = pad.centre;
        let mut out = Vec::new();
        let (cx, cy) = g.cell(c);
        if !g.inside(cx, cy) {
            return out;
        }
        let cong = (1.0 - 0.9 * self.rule.crit as f32).max(0.05);
        let c2 = cy as usize * g.w + cx as usize;
        for (vi, &k) in self.rule.vias.iter().enumerate() {
            let o = &self.rules.vias[k];
            if !o.in_pad || !o.layers.contains(&l) {
                continue;
            }
            if !g.via_ok(c2, &o.layers, k, self.net, self.rule.via_need[vi]) {
                continue;
            }
            let load = self.via_load(vi, c2);
            if hard && load > 0.0 {
                continue;
            }
            let spare = if vi < self.rule.class_vias { 0.0 } else { via_cost };
            let hist = self.soft.hist[l * g.plane() + c2];
            let cost = (via_cost * o.cost + spare) as f32 * (1.0 + cong * (hist + pres * load));
            out.push(Access { neck: None, via: Some((c, k)), cost });
            break;
        }
        let inscribed =
            if geom::point_in_polygon(c, &pad.outline) { edge_dist(&pad.outline, c) } else { 0.0 };
        let reach = match pad.pitch {
            Some(p) => p * 0.75,
            None => inscribed + self.rule.neck,
        };
        for d in [1, 3, 5, 7, 0, 2, 4, 6] {
            let (dx, dy) = DIRS[d];
            let norm = ((dx * dx + dy * dy) as f64).sqrt();
            let u = [dx as f64 / norm, dy as f64 / norm];
            let mut s = inscribed.max(g.g);
            'walk: while s <= reach + 1e-9 {
                let q = [c[0] + u[0] * s, c[1] + u[1] * s];
                s += g.g;
                let (x, y) = g.cell(q);
                if !g.inside(x, y) {
                    break;
                }
                let site = g.center(x as usize, y as usize);
                let q2 = y as usize * g.w + x as usize;
                for (vi, &k) in self.rule.vias.iter().enumerate() {
                    let o = &self.rules.vias[k];
                    if !o.layers.contains(&l)
                        || !g.via_ok(q2, &o.layers, k, self.net, self.rule.via_need[vi])
                    {
                        continue;
                    }
                    let Some(w) = self.neck_width_hard(l, c, site, hard) else { continue };
                    let load = self.via_load(vi, q2) + self.neck_load(l, c, site);
                    if hard && load > 0.0 {
                        continue;
                    }
                    let spare = if vi < self.rule.class_vias { 0.0 } else { via_cost };
                    let hist = self.soft.hist[l * g.plane() + q2];
                    let base = geom::dist(c, site) + via_cost * o.cost + spare;
                    out.push(Access {
                        neck: Some(Neck { layer: l, from: c, to: site, width: w }),
                        via: Some((site, k)),
                        cost: base as f32 * (1.0 + cong * (hist + pres * load)),
                    });
                    break 'walk;
                }
            }
        }
        out
    }

    fn best_via(&self, at: (usize, usize), a: usize, b: usize) -> Option<usize> {
        let g = self.grid;
        let c2 = at.1 * g.w + at.0;
        self.rule
            .vias
            .iter()
            .enumerate()
            .take(self.rule.class_vias)
            .filter(|(_, k)| self.rules.vias[**k].joins(a, b))
            .filter(|(i, k)| {
                g.via_ok(c2, &self.rules.vias[**k].layers, **k, self.net, self.rule.via_need[*i])
            })
            .min_by(|(_, x), (_, y)| {
                let (x, y) = (&self.rules.vias[**x], &self.rules.vias[**y]);
                x.cost.total_cmp(&y.cost).then(x.layers.len().cmp(&y.layers.len()))
            })
            .map(|(_, k)| *k)
    }

    pub fn piece(&self, found: &Found, pads: &[PadRef], access: &[Access]) -> Piece {
        let g = self.grid;
        if found.cells.len() <= 1 {
            return Piece::default();
        }
        let mut runs: Vec<(usize, Vec<P>)> = Vec::new();
        let mut vias = Vec::new();
        for &(l, x, y) in &found.cells {
            let p = g.center(x, y);
            match runs.last_mut() {
                Some((rl, pts)) if *rl == l => pts.push(p),
                Some((rl, pts)) => {
                    let from = *rl;
                    let Some(o) = self.best_via((x, y), from, l) else { continue };
                    let centred = pads.iter().find(|pd| {
                        self.rules.vias[o].in_pad
                            && g.cell(pd.centre) == (x as i64, y as i64)
                            && pd.layers.iter().any(|q| *q == from || *q == l)
                    });
                    let at = centred.map(|pd| pd.centre).unwrap_or(p);
                    if let Some(last) = pts.last_mut() {
                        *last = at;
                    }
                    vias.push((at, o));
                    runs.push((l, vec![at]));
                }
                None => runs.push((l, vec![p])),
            }
        }
        let mut extra = Vec::new();
        let start = (found.tag > 0).then(|| access.get(found.tag as usize - 1)).flatten();
        let end = (found.end_tag > 0).then(|| access.get(found.end_tag as usize - 1)).flatten();
        match start {
            Some(a) => {
                let link = match (&a.via, runs.first()) {
                    (None, Some((_, pts))) => pts.first().copied(),
                    _ => None,
                };
                extra.extend(a.neck.clone().map(|n| (n, link)));
                vias.extend(a.via);
            }
            None => {
                if let Some((l, pts)) = runs.first_mut() {
                    self.extend(*l, pts, pads, true);
                }
            }
        }
        match end {
            Some(a) => {
                let link = match (&a.via, runs.last()) {
                    (None, Some((_, pts))) => pts.last().copied(),
                    _ => None,
                };
                extra.extend(a.neck.clone().map(|n| (n, link)));
                vias.extend(a.via);
            }
            None => {
                if let Some((l, pts)) = runs.last_mut() {
                    self.extend(*l, pts, pads, false);
                }
            }
        }
        let mut tracks: Vec<Run> = runs
            .into_iter()
            .map(|(l, pts)| {
                let pts = simplify(&self.chamfer(l, &self.octilinear(l, &simplify(&pts))));
                Run { layer: l, points: pts, width: self.rule.width[l], neck: false }
            })
            .filter(|r| r.points.len() >= 2 || !vias.is_empty())
            .collect();
        for (n, link) in extra {
            let neck = n.width < self.rule.width[n.layer] - 1e-6;
            let mut points = vec![n.from, n.to];
            if let Some(c) = link.filter(|c| geom::dist(*c, n.to) > 1e-6) {
                points.push(c);
            }
            tracks.push(Run { layer: n.layer, points, width: n.width, neck });
        }
        Piece { tracks, vias }
    }

    fn extend(&self, l: usize, pts: &mut Vec<P>, pads: &[PadRef], front: bool) {
        let end = if front { pts[0] } else { *pts.last().unwrap() };
        let Some(pad) =
            pads.iter().find(|p| p.layers.contains(&l) && geom::point_in_polygon(end, &p.outline))
        else {
            return;
        };
        let inside = |q: P| geom::point_in_polygon(q, &pad.outline);
        if front {
            let mut k = 0;
            while k + 2 < pts.len() && inside(pts[k + 1]) {
                k += 1;
            }
            pts.drain(..k);
        } else {
            let mut k = pts.len() - 1;
            while k >= 2 && inside(pts[k - 1]) {
                k -= 1;
            }
            pts.truncate(k + 1);
        }
        let end = if front { pts[0] } else { *pts.last().unwrap() };
        if geom::dist(pad.centre, end) < 1e-9
            || geom::dist(pad.centre, end) > REACH_CENTRE
            || !self.clear_line(l, pad.centre, end)
        {
            return;
        }
        if front {
            pts.insert(0, pad.centre);
        } else {
            pts.push(pad.centre);
        }
    }

    fn octilinear(&self, l: usize, pts: &[P]) -> Vec<P> {
        if pts.len() < 3 {
            return pts.to_vec();
        }
        let n = pts.len();
        let mut out = vec![pts[0]];
        let mut i = 0;
        while i < n - 1 {
            let before =
                (out.len() >= 2).then(|| heading(out[out.len() - 2], out[out.len() - 1])).flatten();
            let mut found = None;
            for j in (i + 1..n.min(i + 160)).rev() {
                let mut variants = [dogleg(pts[i], pts[j], true), dogleg(pts[i], pts[j], false)];
                if let Some(h) = before {
                    let keeps = |v: &Vec<P>| {
                        heading(v[0], v[1]).is_some_and(|g| (g[0] * h[0] + g[1] * h[1]) > 0.99)
                    };
                    if keeps(&variants[1]) && !keeps(&variants[0]) {
                        variants.swap(0, 1);
                    }
                }
                if j == i + 1 {
                    found = Some((j, vec![pts[i], pts[j]]));
                    break;
                }
                if let Some(v) = variants
                    .into_iter()
                    .find(|v| v.windows(2).all(|w| self.clear_line(l, w[0], w[1])))
                {
                    found = Some((j, v));
                    break;
                }
            }
            match found {
                Some((j, v)) => {
                    out.extend_from_slice(&v[1..]);
                    i = j;
                }
                None => {
                    out.push(pts[i + 1]);
                    i += 1;
                }
            }
        }
        simplify(&out)
    }

    fn chamfer(&self, l: usize, pts: &[P]) -> Vec<P> {
        if pts.len() < 3 {
            return pts.to_vec();
        }
        let mut out = vec![pts[0]];
        for k in 1..pts.len() - 1 {
            let (a, b, c) = (*out.last().unwrap(), pts[k], pts[k + 1]);
            let (Some(u), Some(v)) = (heading(a, b), heading(b, c)) else {
                out.push(b);
                continue;
            };
            if (u[0] * v[0] + u[1] * v[1]).abs() > 1e-6 {
                out.push(b);
                continue;
            }
            let reach = (geom::dist(a, b).min(geom::dist(b, c)) * 0.5).min(1.0);
            let mut done = false;
            let mut cut = reach;
            while cut >= self.grid.g {
                let p = [b[0] - u[0] * cut, b[1] - u[1] * cut];
                let q = [b[0] + v[0] * cut, b[1] + v[1] * cut];
                if self.clear_line(l, p, q) {
                    out.push(p);
                    out.push(q);
                    done = true;
                    break;
                }
                cut *= 0.5;
            }
            if !done {
                out.push(b);
            }
        }
        out.push(*pts.last().unwrap());
        out
    }
}

fn dogleg(p: P, q: P, diagonal_first: bool) -> Vec<P> {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let d = dx.abs().min(dy.abs());
    if d < 1e-9 || (dx.abs() - dy.abs()).abs() < 1e-9 {
        return vec![p, q];
    }
    let diag = [d * dx.signum(), d * dy.signum()];
    let m = if diagonal_first {
        [p[0] + diag[0], p[1] + diag[1]]
    } else {
        [q[0] - diag[0], q[1] - diag[1]]
    };
    vec![p, m, q]
}

fn heading(a: P, b: P) -> Option<P> {
    let l = geom::dist(a, b);
    (l > 1e-9).then(|| [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
}

pub fn simplify(pts: &[P]) -> Vec<P> {
    let mut v: Vec<P> = Vec::new();
    for &p in pts {
        if v.last().is_some_and(|q| geom::dist(*q, p) < 1e-9) {
            continue;
        }
        v.push(p);
    }
    let mut out: Vec<P> = Vec::new();
    for &p in &v {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b[0] - a[0]) * (p[1] - b[1]) - (b[1] - a[1]) * (p[0] - b[0]);
            let dot = (b[0] - a[0]) * (p[0] - b[0]) + (b[1] - a[1]) * (p[1] - b[1]);
            if cross.abs() < 1e-9 && dot > 0.0 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

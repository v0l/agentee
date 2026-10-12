use super::grid::Grid;
use super::rules::{Bucket, Need, NetRule, Rules, um};
use super::search::{BLOCKED, Query, Source, Window};
use super::shape::{Seg, simplify};
use super::soft::{Copper, Footprint, Piece, Run, Soft};
use super::{Env, NetState};
use agentee_core::board::Board;
use agentee_core::geom::{self, P};
use agentee_core::layout::Layout;

pub const NOBODY: u16 = u16::MAX;
const BREAKOUTS: [f64; 4] = [1.5, 0.75, 3.0, 6.0];
const TRIES: usize = 40;

pub struct PairRule {
    pub p: usize,
    pub n: usize,
    pub gap: f64,
    pub clear: f64,
    pub rule: NetRule,
}

#[derive(Clone, Default)]
pub struct Partner {
    pub fp: Option<Footprint>,
    pub segs: Vec<Seg>,
    pub vias: Vec<(P, f64, f64, Vec<usize>)>,
    pub clear: f64,
    pub gap: f64,
    pub hole_cu: f64,
}

pub fn rule(
    rules: &mut Rules,
    layout: &Layout,
    board: &Board,
    p: usize,
    n: usize,
) -> Option<PairRule> {
    let (rp, rn) = (rules.nets.get(p)?.as_ref()?, rules.nets.get(n)?.as_ref()?);
    let (np, nn) = (&layout.nets[p], &layout.nets[n]);
    if np.class != nn.class || rp.domain != rn.domain {
        return None;
    }
    let gap = board.netclasses.iter().find(|c| c.name == np.class)?.diff_gap?.to_mm();
    let clear = np.clearance.max(nn.clearance);
    if gap + 1e-6 < clear {
        return None;
    }
    let nl = layout.copper.len();
    let track: Vec<bool> = (0..nl).map(|l| rp.track[l] && rn.track[l]).collect();
    if !track.iter().any(|t| *t) {
        return None;
    }
    let width: Vec<f64> = (0..nl).map(|l| rp.width[l] + rn.width[l] + gap).collect();
    let clearance = rp.clearance.max(rn.clearance);
    let band = rp.band.max(rn.band);
    let crit = rules.casts[p] || rules.casts[n];
    let domain = rp.domain;
    let slack = rules.slack;
    let mut bucket = vec![None; nl];
    for l in (0..nl).filter(|&l| track[l]) {
        let h = width[l] / 2.0;
        let found = rules.buckets.iter().position(|b| {
            b.layer == l
                && um(b.h) == um(h)
                && um(b.c) == um(band)
                && b.crit == crit
                && b.domain == domain
        });
        bucket[l] = Some(found.unwrap_or_else(|| {
            rules.buckets.push(Bucket { layer: l, h, c: band, crit, domain, zone: None });
            rules.buckets.len() - 1
        }));
    }
    let need: Vec<Need> = width
        .iter()
        .map(|w| Need { d: um(w / 2.0 + band + slack), q: um(w / 2.0 + slack), hole: 0 })
        .collect();
    let rule = NetRule {
        width: width.clone(),
        clearance,
        band,
        track,
        vias: Vec::new(),
        class_vias: 0,
        neck: rp.neck,
        narrowest: width,
        crit: rp.crit.max(rn.crit),
        shadows: rp.shadows || rn.shadows,
        domain,
        zone: None,
        bucket,
        via_bucket: Vec::new(),
        need: need.clone(),
        band_need: need,
        via_need: Vec::new(),
    };
    Some(PairRule { p, n, gap, clear, rule })
}

impl Partner {
    pub fn of(pieces: &[Piece], fp: Option<&Footprint>, rules: &Rules, pair: &PairRule) -> Partner {
        let mut segs = Vec::new();
        let mut vias = Vec::new();
        for p in pieces {
            for r in p.trimmed.as_ref().unwrap_or(&p.tracks) {
                let h = r.width / 2.0;
                if r.points.len() == 1 {
                    segs.push((r.layer, r.points[0], r.points[0], h));
                }
                for w in r.points.windows(2) {
                    segs.push((r.layer, w[0], w[1], h));
                }
            }
            for &(at, k) in &p.vias {
                let o = &rules.vias[k];
                vias.push((at, o.r, o.dr, o.layers.clone()));
            }
        }
        Partner {
            fp: fp.cloned(),
            segs,
            vias,
            clear: pair.clear,
            gap: pair.gap,
            hole_cu: rules.hole_cu,
        }
    }

    pub fn has(&self, bucket: usize, c: usize) -> bool {
        self.fp.as_ref().is_some_and(|fp| fp.0[bucket].binary_search(&(c as u32)).is_ok())
    }

    pub fn has_via(&self, bucket: usize, c: usize) -> bool {
        self.fp.as_ref().is_some_and(|fp| fp.1[bucket].binary_search(&(c as u32)).is_ok())
    }

    pub fn near_track(&self, l: usize, at: P, h: f64, margin: f64) -> bool {
        let gap = self.clear + margin - 1e-4;
        self.segs
            .iter()
            .any(|&(sl, a, b, sh)| sl == l && geom::point_segment_distance(at, a, b) < h + sh + gap)
            || self.vias.iter().any(|(v, r, dr, ls)| {
                let d = geom::dist(at, *v);
                ls.contains(&l) && (d < h + r + gap || d < h + dr + self.hole_cu - 1e-4)
            })
    }

    pub fn near_via(&self, at: P, r: f64, dr: f64, layers: &[usize], hole_gap: f64) -> bool {
        let gap = self.clear - 1e-4;
        self.segs.iter().any(|&(sl, a, b, sh)| {
            layers.contains(&sl) && {
                let d = geom::point_segment_distance(at, a, b) - sh;
                d < r + gap || d < dr + self.hole_cu - 1e-4
            }
        }) || self.vias.iter().any(|(v, vr, vdr, ls)| {
            let d = geom::dist(at, *v);
            let shared = ls.iter().any(|l| layers.contains(l));
            (shared
                && (d < r + vr + gap
                    || d < dr + vr + self.hole_cu - 1e-4
                    || d < vdr + r + self.hole_cu - 1e-4))
                || d < dr + vdr + hole_gap - 1e-4
        })
    }

    pub fn mask(&self, grid: &Grid, win: &Window, width: &[f64], out: &mut [i8]) {
        let (ww, area) = (win.ww(), win.ww() * win.wh());
        let mut block = |l: usize, lo: P, hi: P, reach: f64, mark: &dyn Fn(P) -> i8| {
            let (x0, y0, x1, y1) = grid.span(lo, hi, reach);
            if x1 == usize::MAX || y1 == usize::MAX {
                return;
            }
            for y in y0.max(win.y0)..=y1.min(win.y1) {
                for x in x0.max(win.x0)..=x1.min(win.x1) {
                    let k = l * area + (y - win.y0) * ww + (x - win.x0);
                    match mark(grid.center(x, y)) {
                        BLOCKED => out[k] = BLOCKED,
                        1 if out[k] == 0 => out[k] = 1,
                        _ => {}
                    }
                }
            }
        };
        let margin = grid.g * 0.1;
        let tol = 0.1 * self.gap + 0.005;
        for &(l, a, b, sh) in &self.segs {
            let h = width[l] / 2.0;
            let reach = h + sh + self.clear + margin;
            let paired = h + sh + self.gap;
            let off = (paired + tol, h + sh + 3.0 * self.gap);
            let lo = [a[0].min(b[0]), a[1].min(b[1])];
            let hi = [a[0].max(b[0]), a[1].max(b[1])];
            block(l, lo, hi, off.1.max(reach), &|c| {
                let d = geom::point_segment_distance(c, a, b);
                if d < reach - 1e-4 {
                    BLOCKED
                } else if d > off.0 && d < off.1 {
                    1
                } else {
                    0
                }
            });
        }
        for (v, r, dr, ls) in &self.vias {
            for &l in ls {
                let h = width[l] / 2.0;
                let reach = (h + r + self.clear).max(h + dr + self.hole_cu) + margin;
                block(l, *v, *v, reach, &|c| {
                    if geom::dist(c, *v) < reach - 1e-4 { BLOCKED } else { 0 }
                });
            }
        }
    }

    pub fn clashes(&self, p: &Piece, rules: &Rules) -> bool {
        p.tracks.iter().any(|r| {
            let h = r.width / 2.0;
            r.points.windows(2).any(|w| {
                self.segs.iter().any(|&(sl, a, b, sh)| {
                    sl == r.layer
                        && geom::segment_segment_distance(w[0], w[1], a, b) - h - sh
                            < self.clear - 1e-4
                }) || self.vias.iter().any(|(v, vr, vdr, ls)| {
                    let d = geom::point_segment_distance(*v, w[0], w[1]) - h;
                    ls.contains(&r.layer)
                        && (d - vr < self.clear - 1e-4 || d - vdr < self.hole_cu - 1e-4)
                })
            })
        }) || p.vias.iter().any(|&(at, k)| {
            let o = &rules.vias[k];
            self.near_via(at, o.r, o.dr, &o.layers, rules.hole_gap)
        })
    }
}

pub fn offset(pts: &[P], d: f64) -> Vec<P> {
    let n = pts.len();
    let normal = |a: P, b: P| {
        let l = geom::dist(a, b).max(1e-12);
        [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]
    };
    (0..n)
        .map(|i| {
            let before = (i > 0).then(|| normal(pts[i - 1], pts[i]));
            let after = (i + 1 < n).then(|| normal(pts[i], pts[i + 1]));
            match (before, after) {
                (Some(u), Some(v)) => {
                    let m = [u[0] + v[0], u[1] + v[1]];
                    let dot = (m[0] * u[0] + m[1] * u[1]).max(1e-6);
                    [pts[i][0] + m[0] * d / dot, pts[i][1] + m[1] * d / dot]
                }
                (Some(u), None) | (None, Some(u)) => [pts[i][0] + u[0] * d, pts[i][1] + u[1] * d],
                (None, None) => pts[i],
            }
        })
        .collect()
}

fn ends(st: &NetState) -> Option<(P, P)> {
    let c = &st.copper;
    let pads: Vec<usize> = (0..c.groups).filter(|&g| c.has_pad(g)).collect();
    if pads.len() != 2 {
        return None;
    }
    let first = |g: usize| {
        c.items.iter().zip(&c.group).find_map(|(it, &k)| match it {
            super::conn::Item::Pad { centre, .. } if k == g => Some(*centre),
            _ => None,
        })
    };
    Some((first(pads[0])?, first(pads[1])?))
}

fn clear_run(env: &Env, l: usize, pts: &[P], h: f64, net: usize, clear: f64) -> bool {
    let g = env.grid;
    pts.windows(2).all(|w| {
        let n = (geom::dist(w[0], w[1]) / (g.g * 0.25)).ceil().max(1.0) as usize;
        (0..=n).all(|k| {
            let t = k as f64 / n as f64;
            let at = [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
            let (d, q) = g.exact(l, at, net as u16);
            d > h + clear && q > h
        })
    })
}

fn debug(m: impl FnOnce() -> String) {
    if std::env::var("AGENTEE_PAIR_DEBUG").is_ok() {
        eprintln!("pair: {}", m());
    }
}

pub fn centreline(
    env: &Env,
    pr: &PairRule,
    sp: &NetState,
    sn: &NetState,
    pres: f32,
    hard: bool,
) -> Option<[Piece; 2]> {
    let name = format!("{}/{}", env.layout.nets[pr.p].name, env.layout.nets[pr.n].name);
    let mut why = Vec::new();
    for reach in BREAKOUTS {
        match centreline_inner(env, pr, sp, sn, pres, hard, reach) {
            Ok(r) => {
                debug(|| format!("{name}: coupled with {reach} mm breakouts"));
                return Some(r);
            }
            Err(e) => why.push(e),
        }
    }
    debug(|| format!("{name}: {}", why.join("; ")));
    None
}

fn centreline_inner(
    env: &Env,
    pr: &PairRule,
    sp: &NetState,
    sn: &NetState,
    pres: f32,
    hard: bool,
    reach: f64,
) -> Result<[Piece; 2], String> {
    let ((ap, bp), (a2, b2)) = (ends(sp).ok_or("not two ends")?, ends(sn).ok_or("not two ends")?);
    let (an, bn) =
        if geom::dist(ap, a2) + geom::dist(bp, b2) <= geom::dist(ap, b2) + geom::dist(bp, a2) {
            (a2, b2)
        } else {
            (b2, a2)
        };
    let mid = |a: P, b: P| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (ma, mb) = (mid(ap, an), mid(bp, bn));
    let (ra, rb) = (geom::dist(ap, an) / 2.0 + reach, geom::dist(bp, bn) / 2.0 + reach);
    if geom::dist(ma, mb) < ra + rb {
        return Err("too short to couple".into());
    }
    let grid = env.grid;
    let rule = &pr.rule;
    let lo = [ap[0].min(an[0]).min(bp[0]).min(bn[0]), ap[1].min(an[1]).min(bp[1]).min(bn[1])];
    let hi = [ap[0].max(an[0]).max(bp[0]).max(bn[0]), ap[1].max(an[1]).max(bp[1]).max(bn[1])];
    let none = std::collections::HashSet::new();
    let mut margin = env.opts.margin;
    for _ in 0..2 {
        let win = Window::around(grid, lo, hi, margin);
        margin *= 3.0;
        if !win.fits() {
            continue;
        }
        let mut sources = Vec::new();
        let mut targets = Vec::new();
        for l in (0..grid.nl).filter(|&l| rule.track[l]) {
            for (c, r, src) in [(ma, ra, true), (mb, rb, false)] {
                let (x0, y0, x1, y1) = grid.span(c, c, r);
                if x1 == usize::MAX || y1 == usize::MAX {
                    continue;
                }
                for y in y0.max(win.y0)..=y1.min(win.y1) {
                    for x in x0.max(win.x0)..=x1.min(win.x1) {
                        let at = grid.center(x, y);
                        let d = geom::dist(at, c);
                        let i3 = grid.idx(l, x, y);
                        if d > r || !grid.track_ok(i3, NOBODY, rule.need[l]) {
                            continue;
                        }
                        if hard
                            && rule.bucket[l].is_some_and(|b| env.soft.track(b, y * grid.w + x) > 0)
                        {
                            continue;
                        }
                        if src {
                            sources.push(Source {
                                at: i3,
                                cost: d as f32,
                                tag: 0,
                                via_only: false,
                                landed: false,
                            });
                        } else {
                            targets.push((i3, 0));
                        }
                    }
                }
            }
        }
        if sources.is_empty() || targets.is_empty() {
            return Err(format!("{} sources, {} targets", sources.len(), targets.len()));
        }
        let q = Query {
            grid,
            soft: env.soft,
            rules: env.rules,
            rule,
            net: NOBODY,
            window: win,
            sources,
            targets,
            entry: Vec::new(),
            entry_r: 0.0,
            pres,
            hard,
            via_cost: env.opts.via_cost,
            vias: false,
            bend_cost: env.opts.bend_cost * 4.0,
            zone: env.zone,
            reserve: env.reserve,
            reserve_cost: super::RESERVE_COST,
            zone_cost: env.opts.zone_cost,
            extra: None,
            outside: env.opts.corridor_cost as f32,
            gain: env.opts.prefer_gain as f32,
            own: &none,
            holes: &[],
            old: None,
            partner: None,
            entering: None,
        };
        let Some(found) = q.run() else { continue };
        let l = found.cells.first().ok_or("empty path")?.0;
        let pts =
            simplify(&found.cells.iter().map(|&(_, x, y)| grid.center(x, y)).collect::<Vec<_>>());
        if pts.len() < 2 {
            return Err("path of one point".into());
        }
        let (wp, wn) = (env.rules.rule(pr.p).width[l], env.rules.rule(pr.n).width[l]);
        let s = (wp / 2.0 + pr.gap + wn / 2.0) / 2.0;
        let (left, right) = (offset(&pts, s), offset(&pts, -s));
        let (first, last) = (0, pts.len() - 1);
        let cost = |a: &[P], b: &[P]| {
            geom::dist(a[first], ap)
                + geom::dist(b[first], an)
                + geom::dist(a[last], bp)
                + geom::dist(b[last], bn)
        };
        let (runp, runn) =
            if cost(&left, &right) <= cost(&right, &left) { (left, right) } else { (right, left) };
        if !clear_run(env, l, &runp, wp / 2.0, pr.p, pr.clear)
            || !clear_run(env, l, &runn, wn / 2.0, pr.n, pr.clear)
        {
            return Err("an offset run breaks clearance".into());
        }
        let piece = |pts: Vec<P>, w: f64| Piece {
            tracks: vec![Run { layer: l, points: pts, width: w, neck: false }],
            vias: Vec::new(),
            trimmed: None,
            ms: 0.0,
            coupled: true,
        };
        return Ok([piece(runp, wp), piece(runn, wn)]);
    }
    Err("no centreline".into())
}

pub struct Couples {
    pub list: Vec<(usize, usize, usize)>,
    pub tries: Vec<usize>,
}

impl Couples {
    pub fn new(states: &mut [NetState], rules: &Rules) -> Couples {
        let mut list = Vec::new();
        for (ci, pr) in rules.pairs.iter().enumerate() {
            let sp = states.iter().position(|s| s.net == pr.p);
            let sn = states.iter().position(|s| s.net == pr.n);
            let (Some(sp), Some(sn)) = (sp, sn) else { continue };
            if states[sp].needed != 1 || states[sn].needed != 1 {
                continue;
            }
            states[sp].mate = Some((sn, ci));
            states[sn].mate = Some((sp, ci));
            list.push((sp, sn, ci));
        }
        let tries = vec![0; list.len()];
        Couples { list, tries }
    }

    pub fn place(
        &mut self,
        states: &mut [NetState],
        todo: &mut Vec<usize>,
        env: &Env,
        pres: f32,
        hard: bool,
    ) {
        for (k, &(sp, sn, ci)) in self.list.iter().enumerate() {
            if !todo.contains(&sp) && !todo.contains(&sn) {
                continue;
            }
            if self.tries[k] >= TRIES && !hard {
                continue;
            }
            let holds = |st: &NetState| {
                st.bad.as_ref().is_some_and(|b| {
                    b.len() == st.pieces.len()
                        && st.pieces.iter().zip(b).any(|(p, b)| p.coupled && !b)
                })
            };
            if holds(&states[sp]) && holds(&states[sn]) {
                continue;
            }
            self.tries[k] += 1;
            for &s in &[sp, sn] {
                let n = states[s].pieces.len();
                states[s].bad = Some(vec![true; n]);
                if !todo.contains(&s) {
                    todo.push(s);
                }
            }
            for &s in &[sp, sn] {
                if let Some(fp) = &states[s].fp {
                    env.soft.apply(fp, false);
                }
            }
            let runs = centreline(env, &env.rules.pairs[ci], &states[sp], &states[sn], pres, hard);
            for &s in &[sp, sn] {
                if let Some(fp) = &states[s].fp {
                    env.soft.apply(fp, true);
                }
            }
            let Some([rp, rn]) = runs else {
                self.tries[k] = TRIES;
                continue;
            };
            for (s, run) in [(sp, rp), (sn, rn)] {
                let st = &mut states[s];
                st.pieces.retain(|p| !p.coupled);
                let n = st.pieces.len();
                let rule = env.rules.rule(st.net);
                let fp = Soft::footprint(
                    env.grid,
                    env.rules,
                    &Copper::of(std::slice::from_ref(&run), rule, &st.pads),
                );
                if let Some(old) = st.fp.take() {
                    env.soft.apply(&old, false);
                }
                env.soft.apply(&fp, true);
                st.fp = Some(fp);
                st.pieces.push(run);
                let mut bad = vec![true; n];
                bad.push(false);
                st.bad = Some(bad);
            }
        }
    }
}

pub fn partner_of(states: &[NetState], si: usize, rules: &Rules) -> Option<Partner> {
    let (mj, ci) = states[si].mate?;
    let m = &states[mj];
    Some(Partner::of(&m.pieces, m.fp.as_ref(), rules, &rules.pairs[ci]))
}

pub fn uncouple_short(states: &mut [NetState], couples: &Couples, env: &Env) -> usize {
    let mut won = 0;
    for &(sp, sn, _) in &couples.list {
        let short = |st: &NetState| st.joined < st.needed;
        let coupled = |st: &NetState| st.pieces.iter().any(|p| p.coupled);
        if !(short(&states[sp]) || short(&states[sn]))
            || !(coupled(&states[sp]) || coupled(&states[sn]))
        {
            continue;
        }
        let before = states[sp].joined + states[sn].joined;
        let held = [super::save(&states[sp]), super::save(&states[sn])];
        for &s in &[sp, sn] {
            states[s].bad = None;
        }
        super::reroute(states, sp, env, super::MAX_PRES, true);
        super::reroute(states, sn, env, super::MAX_PRES, true);
        let after = states[sp].joined + states[sn].joined;
        if after > before {
            won += after - before;
            debug(|| {
                format!(
                    "{}: routed uncoupled, {after} joined",
                    env.layout.nets[states[sp].net].name
                )
            });
        } else {
            let [hp, hn] = held;
            super::restore(&mut states[sn], hn, env.soft);
            super::restore(&mut states[sp], hp, env.soft);
        }
    }
    won
}

fn along(pts: &[P], q: P) -> (f64, f64) {
    let mut run = 0.0;
    let mut best = (f64::MAX, 0.0);
    for w in pts.windows(2) {
        let len = geom::dist(w[0], w[1]);
        let d = geom::point_segment_distance(q, w[0], w[1]);
        if d < best.0 {
            let t = if len > 1e-12 {
                (((q[0] - w[0][0]) * (w[1][0] - w[0][0]) + (q[1] - w[0][1]) * (w[1][1] - w[0][1]))
                    / len)
                    .clamp(0.0, len)
            } else {
                0.0
            };
            best = (d, run + t);
        }
        run += len;
    }
    best
}

fn cut(pts: &[P], from: f64, to: f64) -> Vec<P> {
    let mut out = Vec::new();
    let mut run = 0.0;
    let at = |a: P, b: P, t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    for w in pts.windows(2) {
        let len = geom::dist(w[0], w[1]).max(1e-12);
        let (s, e) = (run, run + len);
        if e >= from && s <= to {
            let a = at(w[0], w[1], ((from - s) / len).clamp(0.0, 1.0));
            let b = at(w[0], w[1], ((to - s) / len).clamp(0.0, 1.0));
            if out.last().is_none_or(|p: &P| geom::dist(*p, a) > 1e-9) {
                out.push(a);
            }
            out.push(b);
        }
        run = e;
    }
    simplify(&out)
}

pub fn trim_runs(st: &mut NetState) {
    let Some(k) = st.pieces.iter().position(|p| p.coupled) else { return };
    let run = &st.pieces[k].tracks[0];
    let (l, h) = (run.layer, run.width / 2.0);
    let pts = run.points.clone();
    let mut ts: Vec<f64> = Vec::new();
    let mut touch = |q: P, reach: f64| {
        let (d, t) = along(&pts, q);
        if d <= h + reach + 1e-3 {
            ts.push(t);
        }
    };
    for (i, p) in st.pieces.iter().enumerate().filter(|(i, _)| *i != k) {
        let _ = i;
        for r in p.trimmed.as_ref().unwrap_or(&p.tracks).iter().filter(|r| r.layer == l) {
            for q in &r.points {
                touch(*q, r.width / 2.0);
            }
        }
        for &(at, _) in &p.vias {
            touch(at, 0.0);
        }
    }
    for it in &st.copper.items {
        match it {
            super::conn::Item::Seg { layer, shape: super::grid::Shape::Seg(a, b, sh) }
                if *layer == l =>
            {
                touch(*a, *sh);
                touch(*b, *sh);
            }
            super::conn::Item::Via { at, layers, .. } if layers.contains(&l) => touch(*at, 0.0),
            _ => {}
        }
    }
    let total: f64 = pts.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
    for pad in st.pads.iter().filter(|p| p.layers.contains(&l)) {
        for (q, t) in [(pts[0], 0.0), (pts[pts.len() - 1], total)] {
            if geom::point_in_polygon(q, &pad.outline) {
                ts.push(t);
            }
        }
    }
    if ts.len() < 2 {
        return;
    }
    let lo = ts.iter().copied().fold(f64::MAX, f64::min);
    let hi = ts.iter().copied().fold(f64::MIN, f64::max);
    if hi - lo < 1e-6 || (lo < 1e-6 && hi > total - 1e-6) {
        return;
    }
    let kept = cut(&pts, lo, hi);
    if kept.len() >= 2 {
        st.pieces[k].tracks[0].points = kept;
    }
}

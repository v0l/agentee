use super::Options;
use agentee_core::board::Board;
use agentee_core::layout::Layout;
use agentee_core::place;
pub use agentee_core::rules::Isolation;

const NECKDOWN: f64 = 0.5;
const POUR_STRIP: f64 = 0.25;
const PLANE_SHARE: f64 = 0.5;
const SHADOW_CRIT: f64 = 0.8;
pub const SHADOW_GAP: f64 = 0.3;

pub struct ViaOpt {
    pub name: String,
    pub layers: Vec<usize>,
    pub r: f64,
    pub dr: f64,
    pub cost: f64,
    pub in_pad: bool,
}

impl ViaOpt {
    pub fn joins(&self, a: usize, b: usize) -> bool {
        self.layers.contains(&a) && self.layers.contains(&b)
    }
}

#[derive(Clone, Copy)]
pub struct Need {
    pub d: u16,
    pub q: u16,
    pub hole: u16,
}

pub struct NetRule {
    pub width: Vec<f64>,
    pub clearance: f64,
    pub band: f64,
    pub track: Vec<bool>,
    pub vias: Vec<usize>,
    pub class_vias: usize,
    pub neck: f64,
    pub narrowest: Vec<f64>,
    pub crit: f64,
    pub shadows: bool,
    pub domain: Option<usize>,
    pub bucket: Vec<Option<usize>>,
    pub via_bucket: Vec<usize>,
    pub need: Vec<Need>,
    pub band_need: Vec<Need>,
    pub via_need: Vec<Need>,
}

pub struct Bucket {
    pub layer: usize,
    pub h: f64,
    pub c: f64,
    pub crit: bool,
    pub domain: Option<usize>,
}

pub struct ViaBucket {
    pub r: f64,
    pub dr: f64,
    pub c: f64,
    pub layers: Vec<usize>,
    pub domain: Option<usize>,
}

pub struct Rules {
    pub vias: Vec<ViaOpt>,
    pub nets: Vec<Option<NetRule>>,
    pub buckets: Vec<Bucket>,
    pub via_buckets: Vec<ViaBucket>,
    pub slack: f64,
    pub hole_gap: f64,
    pub hole_cu: f64,
    pub pth_cu: f64,
    pub inner_pth_cu: f64,
    pub hole_smd: f64,
    pub npth: f64,
    pub edge: f64,
    pub min_width: f64,
    pub reach: f64,
    pub shadow: Vec<Vec<usize>>,
    pub cut: Vec<Vec<usize>>,
    pub casts: Vec<bool>,
    pub iso: Isolation,
}

fn plane_cover(layout: &Layout) -> Vec<Vec<(usize, f64)>> {
    let area = agentee_core::geom::signed_area(&layout.outline).abs().max(1e-9);
    layout
        .copper
        .iter()
        .map(|l| {
            let mut by_net: Vec<(usize, f64)> = Vec::new();
            for z in layout.zones.iter().filter(|z| &z.layer == l) {
                let filled = z.mask.iter().filter(|m| **m != 0).count() as f64 * z.cell * z.cell;
                match by_net.iter_mut().find(|e| e.0 == z.net) {
                    Some(e) => e.1 += filled / area,
                    None => by_net.push((z.net, filled / area)),
                }
            }
            by_net
        })
        .collect()
}

pub fn um(v: f64) -> u16 {
    (v * 1000.0).round().clamp(0.0, 65534.0) as u16
}

impl Rules {
    pub fn new(
        layout: &Layout,
        board: &Board,
        opts: &Options,
        routed: &[usize],
    ) -> Result<Rules, String> {
        let copper = &layout.copper;
        let nl = copper.len();
        let layer_of = |n: &str| copper.iter().position(|c| c == n);
        let r = &board.rules;
        let slack = opts.grid * 0.1;
        let mut vias = Vec::new();
        for v in &board.vias {
            if board.stackup.drills_via(v).is_err() {
                continue;
            }
            vias.push(ViaOpt {
                name: v.name.clone(),
                layers: v.copper_layers(copper).iter().filter_map(|c| layer_of(c)).collect(),
                r: v.diameter.to_mm() / 2.0,
                dr: v.drill.to_mm() / 2.0,
                cost: v.cost,
                in_pad: opts.via_in_pad && v.drill.to_mm() <= r.max_filled_via_drill.to_mm() + 1e-6,
            });
        }
        if vias.len() > 32 {
            vias.truncate(32);
        }
        let mut rules = Rules {
            vias,
            nets: (0..layout.nets.len()).map(|_| None).collect(),
            buckets: Vec::new(),
            via_buckets: Vec::new(),
            slack,
            hole_gap: r.min_hole_to_hole.to_mm(),
            hole_cu: r.min_via_hole_to_copper.to_mm(),
            pth_cu: r.min_pth_hole_to_copper.to_mm(),
            inner_pth_cu: r.min_inner_pth_hole_to_copper.to_mm(),
            hole_smd: r.min_hole_to_smd_pad.to_mm(),
            npth: r.min_npth_to_copper.to_mm(),
            edge: r.min_copper_to_edge.to_mm(),
            min_width: r.min_track_width.to_mm(),
            reach: 0.0,
            shadow: Vec::new(),
            cut: Vec::new(),
            casts: Vec::new(),
            iso: Isolation::new(board, &layout.nets, layout.copper.len()),
        };
        let cover = plane_cover(layout);
        let plane: Vec<bool> = (0..nl)
            .map(|l| {
                l > 0 && l + 1 < nl && cover[l].iter().map(|e| e.1).sum::<f64>() >= PLANE_SHARE
            })
            .collect();
        let ground: Vec<bool> = (0..nl)
            .map(|l| {
                plane[l]
                    && cover[l]
                        .iter()
                        .filter(|e| place::is_ground(&layout.nets[e.0].name))
                        .map(|e| e.1)
                        .sum::<f64>()
                        >= PLANE_SHARE
            })
            .collect();
        rules.cut = (0..nl)
            .map(|l| {
                if !ground[l] {
                    return Vec::new();
                }
                [l.wrapping_sub(1), l + 1].into_iter().filter(|&m| m < nl).collect()
            })
            .collect();
        rules.shadow = (0..nl)
            .map(|l| {
                [l.wrapping_sub(1), l + 1].into_iter().filter(|&m| m < nl && ground[m]).collect()
            })
            .collect();
        let referenced = |l: usize, refs: &[String]| {
            [l.wrapping_sub(1), l + 1].into_iter().filter(|&m| m < nl && plane[m]).any(|m| {
                cover[m].iter().any(|&(zn, share)| {
                    share >= PLANE_SHARE && refs.iter().any(|r| r == &layout.nets[zn].name)
                })
            })
        };
        let paired: Vec<usize> = layout.pairs.iter().flat_map(|p| [p.p, p.n]).collect();
        let refs_of = |name: &str| -> Vec<String> {
            layout
                .interfaces
                .iter()
                .filter(|i| i.lanes.iter().any(|ln| ln.nets.iter().any(|x| x == name)))
                .flat_map(|i| i.spec.reference.clone())
                .collect()
        };
        let crit_of = |n: usize| {
            let net = &layout.nets[n];
            let class = board.netclasses.iter().find(|c| c.name == net.class);
            opts.criticality.get(&net.class).copied().unwrap_or_else(|| match class {
                Some(c) if place::is_rf_class(board, &c.name) => 1.0,
                Some(c) if c.impedance.is_some() || c.diff_gap.is_some() => 0.8,
                Some(c) if c.layers.len() == 1 => 0.8,
                _ => 0.0,
            })
        };
        rules.casts = (0..layout.nets.len())
            .map(|n| {
                crit_of(n) >= SHADOW_CRIT
                    || !refs_of(&layout.nets[n].name).is_empty()
                    || paired.contains(&n)
            })
            .collect();
        for &n in routed {
            let net = &layout.nets[n];
            let class = board.netclasses.iter().find(|c| c.name == net.class);
            let width: Vec<f64> = copper
                .iter()
                .map(|l| class.map(|c| c.width_on(l).to_mm()).unwrap_or(net.width))
                .collect();
            let mut track: Vec<bool> = match class.filter(|c| !c.layers.is_empty()) {
                Some(c) => copper.iter().map(|l| c.layers.contains(l)).collect(),
                None => vec![true; nl],
            };
            let refs = refs_of(&net.name);
            if !refs.is_empty() {
                let kept: Vec<bool> = (0..nl).map(|l| track[l] && referenced(l, &refs)).collect();
                if kept.iter().any(|k| *k) {
                    track = kept;
                }
            }
            let names: Vec<String> = class.map(|c| c.via.clone()).unwrap_or_default();
            let mut net_vias: Vec<usize> =
                names.iter().filter_map(|v| rules.vias.iter().position(|o| &o.name == v)).collect();
            if net_vias.is_empty() && !rules.vias.is_empty() {
                net_vias.push(0);
            }
            let class_vias = net_vias.len();
            let mut spare: Vec<usize> =
                (0..rules.vias.len()).filter(|k| !net_vias.contains(k)).collect();
            spare.sort_by(|&a, &b| rules.vias[a].r.total_cmp(&rules.vias[b].r));
            net_vias.extend(spare);
            let crit = crit_of(n);
            let clearance = class
                .into_iter()
                .flat_map(|c| {
                    copper
                        .iter()
                        .zip(&track)
                        .filter(|(_, on)| **on)
                        .filter_map(|(l, _)| board.impedance_gap(c, l, Board::NECK_SHARE))
                })
                .fold(net.clearance, f64::max);
            let strip = layout
                .zones
                .iter()
                .filter(|z| copper.iter().zip(&track).any(|(l, on)| *on && *l == z.layer))
                .map(|z| z.min_width)
                .fold(None, |a: Option<f64>, w| Some(a.map_or(w, |a| a.max(w))))
                .unwrap_or(POUR_STRIP);
            let poured = class.filter(|c| {
                copper
                    .iter()
                    .zip(&track)
                    .any(|(l, on)| *on && board.needs_pour(c, l, Board::NECK_SHARE))
            });
            let band = poured.and_then(|c| c.coplanar_gap).map_or(clearance, |s| {
                clearance.max(s.to_mm() + strip + net.clearance + opts.grid)
            });
            let domain = rules.iso.of(n);
            let mut bucket = vec![None; nl];
            for l in 0..nl {
                if !track[l] && !net_vias.iter().any(|&k| rules.vias[k].layers.contains(&l)) {
                    continue;
                }
                let h = width[l] / 2.0;
                let found = rules.buckets.iter().position(|b| {
                    b.layer == l
                        && um(b.h) == um(h)
                        && um(b.c) == um(band)
                        && b.crit == rules.casts[n]
                        && b.domain == domain
                });
                bucket[l] = Some(found.unwrap_or_else(|| {
                    rules.buckets.push(Bucket {
                        layer: l,
                        h,
                        c: band,
                        crit: rules.casts[n],
                        domain,
                    });
                    rules.buckets.len() - 1
                }));
            }
            let mut via_bucket = Vec::new();
            for &k in &net_vias {
                let o = &rules.vias[k];
                let found = rules.via_buckets.iter().position(|b| {
                    um(b.r) == um(o.r)
                        && um(b.dr) == um(o.dr)
                        && um(b.c) == um(band)
                        && b.layers == o.layers
                        && b.domain == domain
                });
                via_bucket.push(found.unwrap_or_else(|| {
                    rules.via_buckets.push(ViaBucket {
                        r: o.r,
                        dr: o.dr,
                        c: band,
                        layers: o.layers.clone(),
                        domain,
                    });
                    rules.via_buckets.len() - 1
                }));
            }
            let need_at = |c: f64| -> Vec<Need> {
                width
                    .iter()
                    .map(|w| Need { d: um(w / 2.0 + c + slack), q: um(w / 2.0 + slack), hole: 0 })
                    .collect()
            };
            let need = need_at(clearance);
            let band_need = need_at(band);
            let via_need = net_vias
                .iter()
                .map(|&k| {
                    let o = &rules.vias[k];
                    Need {
                        d: um((o.dr + rules.hole_cu).max(o.r + clearance) + slack),
                        q: um(o.r + slack),
                        hole: um(o.dr + rules.hole_gap + slack),
                    }
                })
                .collect();
            let neck = class.and_then(|c| c.neckdown).map(|l| l.to_mm()).unwrap_or(NECKDOWN);
            let narrowest = copper
                .iter()
                .map(|l| {
                    class
                        .and_then(|c| board.impedance_widths(c, l, Board::NECK_SHARE))
                        .map_or(rules.min_width, |(lo, _)| lo.max(rules.min_width))
                })
                .collect();
            let shadows = rules.casts[n];
            rules.nets[n] = Some(NetRule {
                width,
                clearance,
                band,
                track,
                vias: net_vias,
                class_vias,
                neck,
                narrowest,
                crit,
                shadows,
                domain,
                bucket,
                via_bucket,
                need,
                band_need,
                via_need,
            });
        }
        let floor = rules.edge.max(rules.npth).max(rules.pth_cu).max(rules.inner_pth_cu);
        let max_c = layout
            .nets
            .iter()
            .map(|n| n.clearance)
            .chain(rules.nets.iter().flatten().map(|r| r.band))
            .fold(floor, f64::max);
        let widest = rules
            .nets
            .iter()
            .flatten()
            .flat_map(|r| r.width.iter().map(|w| w / 2.0))
            .fold(0.0, f64::max);
        let via_reach =
            rules.vias.iter().map(|o| (o.r + max_c).max(o.dr + rules.hole_cu)).fold(0.0, f64::max);
        rules.reach = (widest + max_c).max(via_reach) + slack + opts.grid;
        Ok(rules)
    }

    pub fn rule(&self, net: usize) -> &NetRule {
        self.nets[net].as_ref().expect("routed net has a rule")
    }
}

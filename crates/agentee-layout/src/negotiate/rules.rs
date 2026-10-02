use super::Options;
use agentee_core::board::Board;
use agentee_core::layout::Layout;
use agentee_core::place;

const NECKDOWN: f64 = 0.5;

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
    pub track: Vec<bool>,
    pub vias: Vec<usize>,
    pub class_vias: usize,
    pub neck: f64,
    pub crit: f64,
    pub bucket: Vec<Option<usize>>,
    pub via_bucket: Vec<usize>,
    pub need: Vec<Need>,
    pub via_need: Vec<Need>,
}

pub struct Bucket {
    pub layer: usize,
    pub h: f64,
    pub c: f64,
}

pub struct ViaBucket {
    pub r: f64,
    pub dr: f64,
    pub c: f64,
    pub layers: Vec<usize>,
}

pub struct Rules {
    pub vias: Vec<ViaOpt>,
    pub nets: Vec<Option<NetRule>>,
    pub buckets: Vec<Bucket>,
    pub via_buckets: Vec<ViaBucket>,
    pub slack: f64,
    pub hole_gap: f64,
    pub hole_cu: f64,
    pub hole_smd: f64,
    pub edge: f64,
    pub min_width: f64,
    pub reach: f64,
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
        let slack = opts.grid * 0.6;
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
                in_pad: opts.via_in_pad
                    && v.drill.to_mm() <= r.max_filled_via_drill.to_mm() + 1e-6,
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
            hole_smd: r.min_hole_to_smd_pad.to_mm(),
            edge: r.min_copper_to_edge.to_mm(),
            min_width: r.min_track_width.to_mm(),
            reach: 0.0,
        };
        for &n in routed {
            let net = &layout.nets[n];
            let class = board.netclasses.iter().find(|c| c.name == net.class);
            let width: Vec<f64> = copper
                .iter()
                .map(|l| class.map(|c| c.width_on(l).to_mm()).unwrap_or(net.width))
                .collect();
            let track: Vec<bool> = match class.filter(|c| !c.layers.is_empty()) {
                Some(c) => copper.iter().map(|l| c.layers.contains(l)).collect(),
                None => vec![true; nl],
            };
            let names: Vec<String> = class.map(|c| c.via.clone()).unwrap_or_default();
            let mut net_vias: Vec<usize> = names
                .iter()
                .filter_map(|v| rules.vias.iter().position(|o| &o.name == v))
                .collect();
            if net_vias.is_empty() && !rules.vias.is_empty() {
                net_vias.push(0);
            }
            let class_vias = net_vias.len();
            let mut spare: Vec<usize> =
                (0..rules.vias.len()).filter(|k| !net_vias.contains(k)).collect();
            spare.sort_by(|&a, &b| rules.vias[a].r.total_cmp(&rules.vias[b].r));
            net_vias.extend(spare);
            let crit = opts.criticality.get(&net.class).copied().unwrap_or_else(|| {
                match class {
                    Some(c) if place::is_rf_class(board, &c.name) => 1.0,
                    Some(c) if c.impedance.is_some() || c.diff_gap.is_some() => 0.8,
                    Some(c) if c.layers.len() == 1 => 0.8,
                    _ => 0.0,
                }
            });
            let clearance = net.clearance;
            let mut bucket = vec![None; nl];
            for l in 0..nl {
                if !track[l] && !net_vias.iter().any(|&k| rules.vias[k].layers.contains(&l)) {
                    continue;
                }
                let h = width[l] / 2.0;
                let found = rules.buckets.iter().position(|b| {
                    b.layer == l && um(b.h) == um(h) && um(b.c) == um(clearance)
                });
                bucket[l] = Some(found.unwrap_or_else(|| {
                    rules.buckets.push(Bucket { layer: l, h, c: clearance });
                    rules.buckets.len() - 1
                }));
            }
            let mut via_bucket = Vec::new();
            for &k in &net_vias {
                let o = &rules.vias[k];
                let found = rules.via_buckets.iter().position(|b| {
                    um(b.r) == um(o.r)
                        && um(b.dr) == um(o.dr)
                        && um(b.c) == um(clearance)
                        && b.layers == o.layers
                });
                via_bucket.push(found.unwrap_or_else(|| {
                    rules.via_buckets.push(ViaBucket {
                        r: o.r,
                        dr: o.dr,
                        c: clearance,
                        layers: o.layers.clone(),
                    });
                    rules.via_buckets.len() - 1
                }));
            }
            let need = width
                .iter()
                .map(|w| Need {
                    d: um(w / 2.0 + clearance + slack),
                    q: um(w / 2.0 + clearance + slack),
                    hole: 0,
                })
                .collect();
            let via_need = net_vias
                .iter()
                .map(|&k| {
                    let o = &rules.vias[k];
                    Need {
                        d: um(o.dr + rules.hole_cu + slack),
                        q: um(o.r + clearance + slack),
                        hole: um(o.dr + rules.hole_gap + slack),
                    }
                })
                .collect();
            let neck = class.and_then(|c| c.neckdown).map(|l| l.to_mm()).unwrap_or(NECKDOWN);
            rules.nets[n] = Some(NetRule {
                width,
                clearance,
                track,
                vias: net_vias,
                class_vias,
                neck,
                crit,
                bucket,
                via_bucket,
                need,
                via_need,
            });
        }
        let max_c = layout.nets.iter().map(|n| n.clearance).fold(rules.edge, f64::max);
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

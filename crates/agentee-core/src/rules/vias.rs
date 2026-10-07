use super::{Context, Rule, Violation};
use crate::board::DrillKind;
use crate::drc::via::{ViaOnPad, via_on_pad};
use crate::drc::{CuShape, Owner};
use crate::drc::{edge_distance, is_smd, near};
use crate::footprint::PadKind;
use crate::geom;
use crate::graphic::Bounds;
use crate::layout::{PlacedPad, Via};

fn pad_name<C: Context>(cx: &C, p: usize, k: usize) -> String {
    format!("{}.{}", cx.parts()[p].reference, cx.parts()[p].pads[k].number)
}

pub fn spot(v: &Via) -> String {
    format!("[{:.3}, {:.3}]", v.at[0], v.at[1])
}

pub struct SmdPad {
    pub pad: PlacedPad,
    pub name: String,
    pub part: Option<usize>,
}

fn as_smd<C: Context>(cx: &C, i: usize) -> Option<SmdPad> {
    let c = cx.item(i);
    let CuShape::Poly(rings) = &c.shape else { return None };
    let (pad, name, part) = match c.owner {
        Owner::Pad(p, k) => {
            let base = &cx.parts()[p].pads[k];
            if !is_smd(base) {
                return None;
            }
            (base.clone(), pad_name(cx, p, k), Some(p))
        }
        Owner::Copper(_) => (
            PlacedPad {
                number: String::new(),
                net: c.net,
                kind: PadKind::Smd,
                outlines: Vec::new(),
                copper: Vec::new(),
                mask: Vec::new(),
                paste: Vec::new(),
                drill: None,
            },
            cx.describe(i),
            None,
        ),
        _ => return None,
    };
    let pad = PlacedPad { outlines: rings.clone(), copper: c.layers.clone(), ..pad };
    Some(SmdPad { pad, name, part })
}

fn smd_near<C: Context>(cx: &C, b: &Bounds, reach: f64, layers: &[String]) -> Vec<SmdPad> {
    cx.items_near(b, reach)
        .into_iter()
        .filter_map(|i| as_smd(cx, i))
        .filter(|q| q.pad.copper.iter().any(|l| layers.contains(l)))
        .collect()
}

fn via_pads<C: Context>(cx: &C, reach: impl Fn(&Via) -> f64) -> Vec<(usize, SmdPad)> {
    let mut out = Vec::new();
    for vi in cx.via_subjects() {
        let v = cx.via(vi);
        let r = reach(v);
        let mut b = Bounds::EMPTY;
        b.add(v.at);
        for q in smd_near(cx, &b, r, &v.layers) {
            if near(&crate::drc::rings_bounds(&q.pad.outlines), v.at, r) {
                out.push((vi, q));
            }
        }
    }
    let most = cx
        .board()
        .vias
        .iter()
        .map(|s| s.diameter.to_mm())
        .fold(0.0, f64::max)
        .max(cx.board().rules.min_hole_to_smd_pad.to_mm() * 2.0 + 1.0);
    for i in cx.item_subjects(0.0).into_iter().filter(|&i| cx.planned_item(i)) {
        let Some(q) = as_smd(cx, i) else { continue };
        let b = crate::drc::rings_bounds(&q.pad.outlines);
        for j in cx.items_near(&b, most) {
            let Owner::Via(k) = cx.item(j).owner else { continue };
            let v = cx.via(k);
            if cx.planned_via(k) || !q.pad.copper.iter().any(|l| v.layers.contains(l)) {
                continue;
            }
            if near(&b, v.at, reach(v)) {
                let SmdPad { pad, name, part } = &q;
                out.push((k, SmdPad { pad: pad.clone(), name: name.clone(), part: *part }));
            }
        }
    }
    out
}

pub struct ViaCutsPad;

impl Rule for ViaCutsPad {
    fn id(&self) -> &'static str {
        "via-cuts-pad"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        for (vi, sp) in via_pads(cx, |v| v.diameter / 2.0) {
            let v = cx.via(vi);
            let rad = v.diameter / 2.0;
            {
                let q = &sp.pad;
                if q.net != Some(v.net) {
                    continue;
                }
                let Some(ViaOnPad::Cuts { centre_inside, edge }) = via_on_pad(v, q) else {
                    continue;
                };
                let name = sp.name.clone();
                let how = if centre_inside {
                    format!(
                        "sits in {name} {edge:.3} mm from its edge, so the {:.3} mm drill crosses it",
                        v.drill / 2.0
                    )
                } else if edge < rad - 1e-6 {
                    format!("cuts {:.3} mm into {name}", rad - edge)
                } else {
                    format!("touches the edge of {name}")
                };
                out.push(Violation {
                    rule: self.id(),
                    group: format!("via {}", spot(v)),
                    subject: format!("via {}", spot(v)),
                    other: name,
                    gap: edge,
                    need: rad,
                    at: v.at,
                    detail: how,
                    nets: Some((v.net, v.net)),
                    ..Default::default()
                });
            }
        }
    }
}

pub struct ViaAnnulusPastPad;

impl Rule for ViaAnnulusPastPad {
    fn id(&self) -> &'static str {
        "via-annulus-past-pad"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        for (vi, sp) in via_pads(cx, |v| v.diameter / 2.0) {
            let v = cx.via(vi);
            {
                let q = &sp.pad;
                if q.net != Some(v.net) {
                    continue;
                }
                if let Some(ViaOnPad::AnnulusPast { edge }) = via_on_pad(v, q) {
                    out.push(Violation {
                        rule: self.id(),
                        group: format!("via {}", spot(v)),
                        subject: format!("via {}", spot(v)),
                        other: sp.name.clone(),
                        gap: edge,
                        need: v.diameter / 2.0,
                        at: v.at,
                        nets: Some((v.net, v.net)),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

pub struct ViaInPad;

pub fn vias_in_pads<C: Context>(cx: &C) -> Vec<(usize, SmdPad)> {
    via_pads(cx, |v| v.diameter / 2.0)
        .into_iter()
        .filter(|(vi, q)| {
            let v = cx.via(*vi);
            q.pad.net == Some(v.net) && via_on_pad(v, &q.pad).is_some_and(ViaOnPad::in_pad)
        })
        .collect()
}

impl Rule for ViaInPad {
    fn id(&self) -> &'static str {
        "via-in-pad"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        for (vi, sp) in vias_in_pads(cx) {
            let v = cx.via(vi);
            let fill = v.fill.map(|f| f.describe()).unwrap_or("filled and capped");
            out.push(Violation {
                rule: self.id(),
                group: "vias".into(),
                subject: format!("via {}", spot(v)),
                other: sp.name.clone(),
                at: v.at,
                detail: format!("{} {fill}", v.kind.name()),
                ..Default::default()
            });
        }
    }
}

pub struct ViaInPadFill;

impl Rule for ViaInPadFill {
    fn id(&self) -> &'static str {
        "via-in-pad-fill"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let max = cx.board().rules.max_filled_via_drill.to_mm();
        for (vi, sp) in vias_in_pads(cx) {
            let v = cx.via(vi);
            if v.drill > max + 1e-6 {
                out.push(Violation {
                    rule: self.id(),
                    group: "drill".into(),
                    subject: format!("via {}", spot(v)),
                    other: sp.name.clone(),
                    gap: v.drill,
                    need: max,
                    at: v.at,
                    ..Default::default()
                });
            }
            if let Some(f) = v.fill.filter(|f| *f != crate::board::ViaFill::FilledCapped) {
                out.push(Violation {
                    rule: self.id(),
                    group: "fill".into(),
                    subject: format!("via {}", spot(v)),
                    other: sp.name.clone(),
                    at: v.at,
                    detail: format!(
                        "`{}` is {} (IPC-4761 type {})",
                        v.name,
                        f.describe(),
                        f.ipc4761()
                    ),
                    ..Default::default()
                });
            }
        }
    }
}

pub struct HoleToSmdPad;

impl Rule for HoleToSmdPad {
    fn id(&self) -> &'static str {
        "hole-to-smd-pad"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let need = cx.board().rules.min_hole_to_smd_pad.to_mm();
        for (vi, sp) in via_pads(cx, |v| v.drill / 2.0 + need) {
            let v = cx.via(vi);
            let hole = v.drill / 2.0;
            {
                let q = &sp.pad;
                if !(q.net == Some(v.net) || q.net.is_none()) || via_on_pad(v, q).is_some() {
                    continue;
                }
                let gap =
                    q.outlines.iter().map(|o| edge_distance(o, v.at)).fold(f64::MAX, f64::min)
                        - hole;
                if gap + 1e-6 < need {
                    out.push(Violation {
                        rule: self.id(),
                        group: sp.part.map_or_else(
                            || "planned pads".into(),
                            |p| format!("part {}", cx.parts()[p].reference),
                        ),
                        subject: format!("via {}", spot(v)),
                        other: sp.name.clone(),
                        gap,
                        need,
                        at: v.at,
                        detail: format!("{} at {}", sp.name, spot(v)),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

pub struct StackedVia;

fn out_of_build_order<C: Context>(cx: &C, a: &Via, b: &Via) -> Option<String> {
    let st = &cx.board().stackup;
    let copper = cx.copper();
    let last = copper.len().checked_sub(1)?;
    let (sa, sb) = (a.span_of(copper)?, b.span_of(copper)?);
    let (upper, lower) = match (sa.1 == sb.0, sb.1 == sa.0) {
        (true, _) => ((a, sa), (b, sb)),
        (_, true) => ((b, sb), (a, sa)),
        _ => return None,
    };
    let (top, under) = match upper.1.0.cmp(&(last - lower.1.1)) {
        std::cmp::Ordering::Less => (upper.0, lower.0),
        std::cmp::Ordering::Greater => (lower.0, upper.0),
        std::cmp::Ordering::Equal => return None,
    };
    let order = |v: &Via| st.drill_order(v.hole.first()?, v.hole.last()?, v.drill_kind, v.stacked);
    let (top_steps, under_steps) = (order(top)?, order(under)?);
    (under_steps.1 > top_steps.0).then(|| {
        format!(
            "`{}` under `{}` is drilled at lamination step {}, after step {} of the via on top",
            under.name,
            top.name,
            under_steps.1 + 1,
            top_steps.0 + 1
        )
    })
}

impl Rule for StackedVia {
    fn id(&self) -> &'static str {
        "stacked-via"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let allowed = cx.board().rules.stacked_microvias;
        for i in cx.via_subjects() {
            let a = cx.via(i);
            let below: Vec<&Via> = (0..i)
                .filter(|&j| cx.counts(cx.planned_via(i), cx.planned_via(j)))
                .map(|j| cx.via(j))
                .filter(|b| b.net == a.net && geom::dist(a.at, b.at) <= 1e-6)
                .collect();
            if below.is_empty() {
                continue;
            }
            let order = below.iter().find_map(|b| out_of_build_order(cx, a, b));
            let spot = format!("[{:.3}, {:.3}] ({})", a.at[0], a.at[1], cx.nets()[a.net].name);
            let group = if below.iter().any(|b| a.shares_dielectric(b, cx.copper())) {
                "doubled"
            } else if std::iter::once(a)
                .chain(below.iter().copied())
                .any(|v| v.drill_kind == DrillKind::ControlledDepth)
            {
                "depth"
            } else if order.is_some() {
                "order"
            } else if !allowed {
                "stacked"
            } else {
                continue;
            };
            let detail = match (&order, group) {
                (Some(o), "order") => format!("{spot} ({o})"),
                _ => spot.clone(),
            };
            out.push(Violation {
                rule: self.id(),
                group: group.into(),
                subject: spot,
                at: a.at,
                detail,
                ..Default::default()
            });
        }
    }
}

fn template_pads<C: Context>(
    cx: &C,
    t: &super::zone::Template,
    reach: f64,
    window: &Bounds,
) -> Vec<PlacedPad> {
    smd_near(cx, window, reach, &t.layers).into_iter().map(|q| q.pad).collect()
}

fn edge_gap(q: &PlacedPad, p: crate::geom::P) -> f64 {
    q.outlines.iter().map(|o| edge_distance(o, p)).fold(f64::MAX, f64::min)
}

fn inside(q: &PlacedPad, p: crate::geom::P) -> bool {
    q.outlines.iter().any(|o| geom::point_in_polygon(p, o))
}

impl super::zone::Constrains for ViaCutsPad {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let super::zone::Kind::Via { r, .. } = t.kind else { return };
        let window = zone.window();
        for q in &template_pads(cx, t, r, &window) {
            if !t.owns(q.net) {
                continue;
            }
            let b = crate::drc::rings_bounds(&q.outlines);
            zone.forbid_where(None, &b, r, |c, m| edge_gap(q, c) < r + m);
        }
    }
}

impl super::zone::Constrains for ViaInPadFill {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let super::zone::Kind::Via { r, drill, fill, .. } = t.kind else { return };
        let max = cx.board().rules.max_filled_via_drill.to_mm();
        let open = fill.is_some_and(|f| f != crate::board::ViaFill::FilledCapped);
        if drill <= max + 1e-6 && !open {
            return;
        }
        let window = zone.window();
        for q in &template_pads(cx, t, r, &window) {
            if !t.owns(q.net) {
                continue;
            }
            let b = crate::drc::rings_bounds(&q.outlines);
            zone.forbid_where(None, &b, r, |c, m| inside(q, c) || edge_gap(q, c) < r + m);
        }
    }
}

impl super::zone::Constrains for HoleToSmdPad {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let super::zone::Kind::Via { drill, .. } = t.kind else { return };
        let need = cx.board().rules.min_hole_to_smd_pad.to_mm();
        let window = zone.window();
        for q in &template_pads(cx, t, need + drill, &window) {
            if !(t.owns(q.net) || q.net.is_none()) {
                continue;
            }
            let b = crate::drc::rings_bounds(&q.outlines);
            zone.forbid_where(None, &b, need + drill, |c, m| {
                !inside(q, c) && edge_gap(q, c) - drill / 2.0 < need + m
            });
        }
    }
}

impl super::zone::Constrains for StackedVia {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let super::zone::Kind::Via { .. } = t.kind else { return };
        let window = zone.window();
        for k in 0..cx.via_count() {
            let v = cx.via(k);
            if t.owns(Some(v.net)) && crate::drc::near(&window, v.at, zone.cell) {
                zone.forbid(None, &crate::drc::CuShape::Circle(v.at, 0.0), 1e-3);
            }
        }
    }
}

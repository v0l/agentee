use super::{Category, Ctx, Report, Rule, Setup};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::graphic::Fill;
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "isolation-domain",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a net whose class or name puts it in two isolation domains",
        when: "boards with [[domains]]",
        applies: has_domains,
        check: domain_overlap,
    },
    Rule {
        id: "isolation-unassigned",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "nets in no isolation domain, which no barrier covers",
        when: "boards with [[domains]]",
        applies: has_domains,
        check: unassigned,
    },
    Rule {
        id: "isolation-clearance",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two domains on one layer closer than the clearance of the barrier between them, pads of one footprint and pours included",
        when: "boards with [[barriers]] that set `clearance`, or netclass voltages that need one",
        applies: has_barriers,
        check: barrier_clearance,
    },
    Rule {
        id: "creepage",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two domains closer along an outer surface than the barrier creepage; board cutouts and non-plated holes at least the pollution degree's groove width lengthen the path, narrower ones are bridged",
        when: "boards with [[barriers]] that set `creepage`, or netclass voltages that need one",
        applies: has_creepage,
        check: creepage,
    },
    Rule {
        id: "spark-gap",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a footprint spark gap whose electrodes are not the declared gap apart, sit under the fab clearance minimum, or have solder mask across the gap",
        when: "footprints with [[spark_gaps]]",
        applies: has_spark_gaps,
        check: spark_gaps,
    },
];

fn has_domains(s: &Setup) -> bool {
    s.domains
}

fn has_barriers(s: &Setup) -> bool {
    s.barrier_clearance
}

fn has_creepage(s: &Setup) -> bool {
    s.creepage
}

fn has_spark_gaps(s: &Setup) -> bool {
    s.spark_gaps
}

fn domains_of(cx: &Ctx) -> Vec<Vec<usize>> {
    cx.nets.iter().map(|n| cx.board.domain_of(&n.name, &n.class)).collect()
}

fn domain_overlap(cx: &Ctx, r: &mut Report) {
    for (n, ds) in domains_of(cx).iter().enumerate() {
        if ds.len() > 1 {
            let names: Vec<&str> = ds.iter().map(|&d| cx.board.domains[d].name.as_str()).collect();
            r.emit(
                format!("net {}", cx.nets[n].name),
                format!(
                    "{} (class {}) is in domains {}: a net sits in one domain, narrow the `classes` or `nets` of the others",
                    cx.nets[n].name,
                    cx.nets[n].class,
                    names.join(" and ")
                ),
            );
        }
    }
}

fn unassigned(cx: &Ctx, r: &mut Report) {
    let free: Vec<String> = domains_of(cx)
        .iter()
        .enumerate()
        .filter(|(_, ds)| ds.is_empty())
        .map(|(n, _)| cx.nets[n].name.clone())
        .collect();
    if !free.is_empty() {
        r.emit(
            "domains",
            format!(
                "{} nets are in no isolation domain, so no barrier is checked for them: {}",
                free.len(),
                super::list(&free)
            ),
        );
    }
}

fn from_rule(cx: &Ctx, rule: impl crate::rules::Rule, what: &str, r: &mut Report) {
    let placed = crate::rules::Placed::new(cx);
    let mut found = Vec::new();
    rule.eval(&placed, &mut found);
    let mut hits: BTreeMap<(usize, usize), (crate::rules::Violation, usize)> = BTreeMap::new();
    for v in found {
        let Some(key) = v.nets else { continue };
        match hits.get_mut(&key) {
            Some((h, count)) => {
                *count += 1;
                let mut layers = std::mem::take(&mut h.layers);
                for l in &v.layers {
                    if !layers.contains(l) {
                        layers.push(l.clone());
                    }
                }
                if v.gap < h.gap - 1e-9 {
                    *h = v;
                }
                h.layers = layers;
            }
            None => {
                hits.insert(key, (v, 1));
            }
        }
    }
    for ((a, b), (h, count)) in hits {
        let more = if count > 1 { format!(", {count} places in all") } else { String::new() };
        r.emit(
            format!("{what} {} {}", cx.nets[a].name, cx.nets[b].name),
            format!(
                "{} on {} at [{:.3}, {:.3}]{more}",
                h.detail,
                h.layers.join(", "),
                h.at[0],
                h.at[1]
            ),
        );
    }
}

fn barrier_clearance(cx: &Ctx, r: &mut Report) {
    from_rule(cx, crate::rules::IsolationClearance, "isolation clearance", r);
}

fn creepage(cx: &Ctx, r: &mut Report) {
    from_rule(cx, crate::rules::Creepage, "creepage", r);
}

fn spark_gaps(cx: &Ctx, r: &mut Report) {
    let min = cx.board.rules.min_clearance.to_mm();
    for (pi, part) in cx.parts.iter().enumerate() {
        for gap in &part.footprint.spark_gaps {
            let pads = |n: &str| -> Vec<usize> {
                (0..part.pads.len()).filter(|&k| part.pads[k].number == n).collect()
            };
            let (left, right) = (pads(&gap.pads[0]), pads(&gap.pads[1]));
            let at = format!("{} spark gap {}-{}", part.reference, gap.pads[0], gap.pads[1]);
            let mut measured: Option<(f64, P, P, String)> = None;
            for &k in &left {
                for &m in &right {
                    let (a, b) = (&part.pads[k], &part.pads[m]);
                    for layer in a.copper.iter().filter(|l| b.copper.contains(l)) {
                        let mut ea = Vec::new();
                        let mut eb = Vec::new();
                        a.outlines.iter().for_each(|o| crate::rules::barrier_edges(o, &mut ea));
                        b.outlines.iter().for_each(|o| crate::rules::barrier_edges(o, &mut eb));
                        if let Some((d, p, q)) = crate::rules::closest_edges(&ea, &eb)
                            && measured.as_ref().is_none_or(|x| d < x.0)
                        {
                            measured = Some((d, p, q, layer.clone()));
                        }
                    }
                }
            }
            let Some((d, p, q, layer)) = measured else {
                r.emit(
                    &at,
                    "the two electrodes share no copper layer, so there is no gap to fire across",
                );
                continue;
            };
            if left.iter().chain(&right).any(|&k| part.pads[k].net.is_none())
                || left.iter().any(|&k| right.iter().any(|&m| part.pads[k].net == part.pads[m].net))
            {
                r.emit(&at, "both electrodes need a net, and different nets, for the gap to protect anything");
            }
            if d + 1e-6 < min {
                r.emit(
                    &at,
                    format!(
                        "the electrodes are {} apart on {layer}, under the {} fab clearance minimum",
                        Length::mm(d),
                        Length::mm(min)
                    ),
                );
            }
            if (d - gap.gap).abs() > 0.01 {
                r.emit(
                    &at,
                    format!(
                        "the electrodes are {} apart on {layer}, the footprint declares {}",
                        Length::mm(d),
                        Length::mm(gap.gap)
                    ),
                );
            }
            let mask = if layer == cx.copper.first().map(String::as_str).unwrap_or("F.Cu") {
                "F.Mask"
            } else if Some(&layer) == cx.copper.last() {
                "B.Mask"
            } else {
                continue;
            };
            let open = mask_openings(cx, pi, mask);
            let covered = (1..10).any(|k| {
                let t = k as f64 / 10.0;
                let m = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
                !open.iter().any(|o| geom::point_in_polygon(m, o))
            });
            if covered {
                r.emit(
                    &at,
                    format!(
                        "solder mask covers the gap on {mask} at [{:.3}, {:.3}]; draw a filled {mask} shape over it in the footprint",
                        (p[0] + q[0]) / 2.0,
                        (p[1] + q[1]) / 2.0
                    ),
                );
            }
        }
    }
}

fn mask_openings(cx: &Ctx, pi: usize, mask: &str) -> Vec<Vec<P>> {
    let part = &cx.parts[pi];
    let mut out: Vec<Vec<P>> = part
        .pads
        .iter()
        .filter(|q| q.mask.iter().any(|m| m == mask))
        .flat_map(|q| q.outlines.iter().cloned())
        .collect();
    let tf = part.transform();
    for g in &part.footprint.graphics {
        if part.flip_layer(&g.layer) != mask || g.fill != Fill::Solid {
            continue;
        }
        let path: Vec<P> =
            crate::footprint::graphic_path(g).into_iter().map(|p| tf.apply(p)).collect();
        if path.len() >= 3 {
            out.push(path);
        }
    }
    out
}

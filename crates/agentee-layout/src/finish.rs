use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;

pub struct Finish;

struct Copper {
    layer: String,
    shape: Shape,
}

enum Shape {
    Seg(P, P, f64),
    Disc(P, f64),
    Poly(Vec<P>),
}

impl Shape {
    fn gap(&self, q: P, r: f64) -> f64 {
        match self {
            Shape::Seg(a, b, w) => geom::point_segment_distance(q, *a, *b) - w / 2.0 - r,
            Shape::Disc(c, d) => geom::dist(q, *c) - d - r,
            Shape::Poly(v) => {
                if geom::point_in_polygon(q, v) {
                    -r
                } else {
                    let n = v.len();
                    (0..n)
                        .map(|i| geom::point_segment_distance(q, v[i], v[(i + 1) % n]))
                        .fold(f64::MAX, f64::min)
                        - r
                }
            }
        }
    }
}

fn section(text: &str, phase: &str) -> Option<String> {
    let start = format!("# plan {phase}\n");
    let end = format!("# end plan {phase}\n");
    let a = text.find(&start)? + start.len();
    let b = text[a..].find(&end)? + a;
    Some(text[a..b].to_string())
}

fn trimmed(points: &[P], width: f64, others: &[&Copper], layer: &str) -> Option<Vec<P>> {
    let touches =
        |q: P| others.iter().any(|c| c.layer == layer && c.shape.gap(q, width / 2.0) <= 1e-6);
    let start = points[0];
    let anchors: Vec<&&Copper> = others
        .iter()
        .filter(|c| c.layer == layer && c.shape.gap(start, width / 2.0) <= 1e-6)
        .collect();
    let external = |q: P| {
        others.iter().any(|c| {
            c.layer == layer
                && c.shape.gap(q, width / 2.0) <= 1e-6
                && !anchors.iter().any(|a| std::ptr::eq(**a, *c))
        })
    };
    if touches(*points.last()?) {
        return None;
    }
    for k in (1..points.len()).rev() {
        let (a, b) = (points[k - 1], points[k]);
        let len = geom::dist(a, b);
        let steps = (len / 0.02).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let t = 1.0 - s as f64 / steps as f64;
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if external(q) {
                let mut out: Vec<P> = points[..k].to_vec();
                if geom::dist(q, a) > 1e-6 {
                    out.push(q);
                }
                return Some(out);
            }
        }
    }
    Some(Vec::new())
}

impl Phase for Finish {
    fn name(&self) -> &'static str {
        "finish"
    }

    fn run(
        &self,
        model: &mut Model,
        _cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "finish".into(), ..Default::default() };
        let Some(body) = section(&model.text, "escape") else {
            report.notes.push("no escape plan to tidy".into());
            return report;
        };
        let Ok(doc) = body.parse::<toml::Table>() else {
            report.failed.push("the escape plan does not parse".into());
            return report;
        };
        let l = &model.layout;
        let net_of = |name: &str| l.nets.iter().position(|n| n.name == name);
        let mut copper: Vec<(usize, Copper)> = Vec::new();
        for t in &l.tracks {
            for w in t.points.windows(2) {
                copper.push((
                    t.net,
                    Copper { layer: t.layer.clone(), shape: Shape::Seg(w[0], w[1], t.width) },
                ));
            }
        }
        for v in &l.vias {
            for ly in &v.layers {
                copper.push((
                    v.net,
                    Copper { layer: ly.clone(), shape: Shape::Disc(v.at, v.diameter / 2.0) },
                ));
            }
        }
        for p in &l.parts {
            for pad in &p.pads {
                let Some(n) = pad.net else { continue };
                for o in &pad.outlines {
                    for ly in &pad.copper {
                        copper
                            .push((n, Copper { layer: ly.clone(), shape: Shape::Poly(o.clone()) }));
                    }
                }
            }
        }
        let tracks = doc.get("tracks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut kept = Vec::new();
        let (mut cut, mut dropped) = (0, 0);
        for t in tracks {
            let Some(tt) = t.as_table() else { continue };
            let name = tt.get("net").and_then(|v| v.as_str()).unwrap_or("");
            let layer = tt.get("layer").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let width = tt.get("width").and_then(|v| v.as_float()).unwrap_or(0.1);
            let points: Vec<P> = tt
                .get("points")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|q| {
                            let q = q.as_array()?;
                            Some([q.first()?.as_float()?, q.get(1)?.as_float()?])
                        })
                        .collect()
                })
                .unwrap_or_default();
            let Some(n) = net_of(name) else { continue };
            if points.len() < 2 || l.nets[n].unrouted > 0 {
                kept.push((name.to_string(), layer, width, points));
                continue;
            }
            let mut bb = Bounds::EMPTY;
            points.iter().for_each(|q| bb.add(*q));
            let own: Vec<&Copper> = copper
                .iter()
                .filter(|(cn, c)| {
                    *cn == n
                        && !matches!(&c.shape, Shape::Seg(a, b, _) if points.windows(2).any(|w| geom::dist(w[0], *a) < 1e-6 && geom::dist(w[1], *b) < 1e-6))
                })
                .map(|(_, c)| c)
                .collect();
            match trimmed(&points, width, &own, &layer) {
                None => kept.push((name.to_string(), layer, width, points)),
                Some(p) if p.len() >= 2 => {
                    cut += 1;
                    kept.push((name.to_string(), layer, width, p));
                }
                Some(_) => dropped += 1,
            }
        }
        let mut out = String::new();
        for (name, layer, width, points) in &kept {
            out += &crate::track_toml(name, layer, Some(*width), points);
        }
        if let Some(vias) = doc.get("vias").and_then(|v| v.as_array()) {
            for v in vias {
                let Some(vt) = v.as_table() else { continue };
                let net = vt.get("net").and_then(|x| x.as_str()).unwrap_or("");
                let via = vt.get("via").and_then(|x| x.as_str()).unwrap_or("");
                let Some(at) = vt.get("at").and_then(|x| x.as_array()) else { continue };
                let (Some(x), Some(y)) =
                    (at.first().and_then(|v| v.as_float()), at.get(1).and_then(|v| v.as_float()))
                else {
                    continue;
                };
                out += &crate::via_toml(net, [x, y], via);
            }
        }
        report.notes.push(format!(
            "escape tracks of routed nets: {cut} cut back to where routing joins, {dropped} removed"
        ));
        report.changed = cut + dropped > 0;
        model.finished_escape = Some(out);
        report
    }
}

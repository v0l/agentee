pub mod gerber;

use agentee_core::board::{Board, DrillKind, LayerKind, ViaKind};
use agentee_core::font;
use agentee_core::footprint::{PadKind, graphic_path};
use agentee_core::geom::P;
use agentee_core::graphic::{Fill, Shape};
use agentee_core::layout::Layout;
use agentee_core::schematic::Schematic;
use gerber::Gerber;
use serde::Serialize;
use std::fmt::Write;
use std::path::Path;

#[derive(Serialize)]
pub struct Report {
    pub dir: String,
    pub files: Vec<String>,
    pub parts_placed: usize,
    pub bom_lines: usize,
    pub holes: usize,
}

fn layer_file(name: &str) -> String {
    name.replace('.', "_")
}

fn copper_function(i: usize, n: usize) -> String {
    let side = if i == 0 {
        "Top"
    } else if i + 1 == n {
        "Bot"
    } else {
        "Inr"
    };
    format!("Copper,L{},{side}", i + 1)
}

type Rings = Vec<Vec<P>>;
type Filled = (Rings, Rings);
type BomKey = (String, String, String, String);

fn copper(layout: &Layout, layer: &str, function: &str) -> Gerber {
    let mut g = Gerber::new(function);
    let mut zones: Vec<Filled> = Vec::new();
    for z in layout.zones.iter().filter(|z| z.layer == layer) {
        let (outer, holes): (Vec<_>, Vec<_>) =
            z.rings.iter().cloned().partition(|r| agentee_core::contour::area(r) > 0.0);
        zones.push((outer, holes));
    }
    let depth = |k: usize| {
        let Some(p) = zones[k].0.first().and_then(|r| r.first()) else { return 0 };
        zones
            .iter()
            .enumerate()
            .filter(|(j, z)| {
                *j != k && z.1.iter().any(|h| agentee_core::geom::point_in_polygon(*p, h))
            })
            .count()
    };
    let mut order: Vec<usize> = (0..zones.len()).collect();
    order.sort_by_key(|k| depth(*k));
    for k in order {
        g.polarity(true);
        for r in &zones[k].0 {
            g.region(r);
        }
        g.polarity(false);
        for r in &zones[k].1 {
            g.region(r);
        }
    }
    g.polarity(true);
    for t in layout.tracks.iter().filter(|t| t.layer == layer) {
        g.stroke(&t.points, t.width);
    }
    for part in &layout.parts {
        for pad in part.pads.iter().filter(|q| q.copper.iter().any(|c| c == layer)) {
            for o in &pad.outlines {
                g.region(o);
            }
        }
    }
    for v in layout.vias.iter().filter(|v| v.layers.iter().any(|l| l == layer)) {
        g.flash_circle(v.at, v.diameter);
    }
    footprint_graphics(&mut g, layout, layer);
    g
}

fn footprint_graphics(g: &mut Gerber, layout: &Layout, layer: &str) {
    for part in &layout.parts {
        let tf = part.transform();
        for gr in part.footprint.graphics.iter().filter(|gr| part.flip_layer(&gr.layer) == layer) {
            if matches!(gr.shape, Shape::Text { .. }) {
                continue;
            }
            let path: Vec<P> = graphic_path(gr).into_iter().map(|p| tf.apply(p)).collect();
            if gr.fill == Fill::Solid && path.len() >= 3 {
                g.region(&path);
            }
            if gr.width.to_mm() > 0.0 {
                g.stroke(&path, gr.width.to_mm());
            }
        }
    }
}

fn openings(layout: &Layout, layer: &str, function: &str, paste: bool) -> Gerber {
    let mut g = Gerber::new(function);
    if !paste && layout.test.vias && layer == layout.test.mask() {
        for v in layout.vias.iter().filter(|v| v.layers.contains(&layout.test.copper())) {
            g.flash_circle(v.at, v.diameter);
        }
    }
    for part in &layout.parts {
        for pad in &part.pads {
            let on = if paste {
                pad.paste.iter().any(|m| m == layer)
            } else {
                pad.mask.iter().any(|m| m == layer)
            };
            if on {
                for o in &pad.outlines {
                    g.region(o);
                }
            }
        }
    }
    footprint_graphics(&mut g, layout, layer);
    g
}

fn silk(layout: &Layout, board: &Board, layer: &str, function: &str) -> Gerber {
    let mut g = Gerber::new(function);
    let min = board.rules.min_silk_width.to_mm();
    for a in layout.artwork.iter().filter(|a| a.layer == layer) {
        for r in &a.polygons {
            g.region(r);
        }
    }
    let draw = |g: &mut Gerber,
                gr: &agentee_core::graphic::Graphic,
                tf: &agentee_core::geom::Transform| {
        if matches!(gr.shape, Shape::Text { .. }) {
            return;
        }
        let path: Vec<P> = graphic_path(gr).into_iter().map(|p| tf.apply(p)).collect();
        if gr.fill == Fill::Solid && path.len() >= 3 {
            g.region(&path);
        }
        g.stroke(&path, gr.width.to_mm().max(min));
    };
    let identity = agentee_core::geom::Transform { at: [0.0, 0.0], rotation: 0.0, mirror: false };
    for gr in layout.graphics.iter().filter(|g| g.layer == layer) {
        draw(&mut g, gr, &identity);
    }
    for part in &layout.parts {
        let tf = part.transform();
        for gr in &part.footprint.graphics {
            if part.flip_layer(&gr.layer) == layer {
                draw(&mut g, gr, &tf);
            }
        }
    }
    let mut texts: Vec<_> =
        layout.parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
    texts.extend(layout.board_texts());
    let bottom = layer.starts_with("B.");
    for t in texts.iter().filter(|t| t.layer == layer) {
        let w = font::default_thickness(t.size).max(min);
        for st in font::strokes(&t.text, t.at, t.size, t.rotation, t.anchor, bottom) {
            g.stroke(&st, w);
        }
    }
    g
}

fn edge(layout: &Layout) -> Gerber {
    let mut g = Gerber::new("Profile,NP");
    for r in std::iter::once(&layout.outline).chain(&layout.board_cutouts) {
        let mut ring = r.clone();
        if let Some(f) = ring.first().copied() {
            ring.push(f);
        }
        g.stroke(&ring, 0.1);
    }
    g
}

struct Hole {
    at: P,
    size: [f64; 2],
    rotation: f64,
    plated: bool,
    span: Option<(usize, usize)>,
    depth: bool,
}

fn holes(layout: &Layout) -> Vec<Hole> {
    let last = layout.copper.len().saturating_sub(1);
    let mut v: Vec<Hole> = layout
        .vias
        .iter()
        .map(|x| Hole {
            at: x.at,
            size: [x.drill, x.drill],
            rotation: 0.0,
            plated: true,
            span: x.span_of(&layout.copper).filter(|s| *s != (0, last)),
            depth: x.drill_kind == DrillKind::ControlledDepth,
        })
        .collect();
    for part in &layout.parts {
        for pad in &part.pads {
            if let Some((at, size, rot)) = pad.drill {
                v.push(Hole {
                    at,
                    size,
                    rotation: rot,
                    plated: pad.kind != PadKind::Npth,
                    span: None,
                    depth: false,
                });
            }
        }
    }
    v
}

struct DrillSpan<'a> {
    file: String,
    function: String,
    comment: String,
    holes: Vec<&'a Hole>,
}

fn span_function(a: usize, b: usize, layers: usize) -> &'static str {
    if a == 0 || b + 1 == layers { "Blind" } else { "Buried" }
}

fn span_file(cu: &[String], a: usize, b: usize, depth: bool) -> String {
    let suffix = if depth { "-controlled-depth" } else { "" };
    format!("drill-{}-{}{suffix}.drl", cu[a], cu[b])
}

fn drill_spans<'a>(hs: &'a [Hole], layout: &Layout) -> Vec<DrillSpan<'a>> {
    let cu = &layout.copper;
    let mut spans: Vec<((usize, usize), bool)> =
        hs.iter().filter_map(|h| Some((h.span?, h.depth))).collect();
    spans.sort_unstable();
    spans.dedup();
    spans
        .into_iter()
        .map(|((a, b), depth)| {
            let comment = if depth {
                let (side, stop) = if a == 0 { (&cu[a], &cu[b]) } else { (&cu[b], &cu[a]) };
                format!(
                    "; span {} to {}, controlled depth blind vias drilled from {side} after the last press, stopping on {stop}",
                    cu[a], cu[b]
                )
            } else {
                let kinds: Vec<&str> = layout
                    .vias
                    .iter()
                    .filter(|v| {
                        v.span_of(cu) == Some((a, b)) && v.drill_kind != DrillKind::ControlledDepth
                    })
                    .map(|v| v.kind.name())
                    .fold(Vec::new(), |mut k, n| {
                        if !k.contains(&n) {
                            k.push(n);
                        }
                        k
                    });
                format!("; span {} to {}, {} vias", cu[a], cu[b], kinds.join(" and "))
            };
            DrillSpan {
                file: span_file(cu, a, b, depth),
                function: format!("Plated,{},{},{}", a + 1, b + 1, span_function(a, b, cu.len())),
                comment,
                holes: hs.iter().filter(|h| h.span == Some((a, b)) && h.depth == depth).collect(),
            }
        })
        .collect()
}

fn backdrills(layout: &Layout) -> Vec<(String, String, Vec<Hole>)> {
    let cu = &layout.copper;
    let mut out: Vec<(String, String, Vec<Hole>)> = Vec::new();
    for v in &layout.vias {
        let Some(bd) = &v.backdrill else { continue };
        let (Some(side), Some(stop)) =
            (cu.iter().position(|c| *c == bd.from), cu.iter().position(|c| *c == bd.to))
        else {
            continue;
        };
        let (a, b) = (side.min(stop), side.max(stop));
        let file = format!("drill-backdrill-{}-{}.drl", bd.from, bd.to);
        let function = format!("NonPlated,{},{},{}", a + 1, b + 1, span_function(a, b, cu.len()));
        let comment = format!(
            "; backdrill from {} stopping before {}, max stub {:.3} mm",
            bd.from,
            bd.to,
            bd.max_stub.to_mm()
        );
        let d = bd.diameter.to_mm();
        let hole =
            Hole { at: v.at, size: [d, d], rotation: 0.0, plated: false, span: None, depth: false };
        match out.iter_mut().find(|x| x.0 == file) {
            Some(x) => x.2.push(hole),
            None => out.push((file, format!("{function}\n{comment}"), vec![hole])),
        }
    }
    out
}

fn excellon(holes: &[&Hole], plated: bool, layers: usize) -> String {
    let function = format!(
        "{},1,{},{}",
        if plated { "Plated" } else { "NonPlated" },
        layers,
        if plated { "PTH" } else { "NPTH" }
    );
    excellon_with(holes, &function)
}

fn excellon_with(holes: &[&Hole], function: &str) -> String {
    let mut tools: Vec<f64> = Vec::new();
    for h in holes {
        let d = (h.size[0].min(h.size[1]) * 1000.0).round() / 1000.0;
        if !tools.iter().any(|t| (t - d).abs() < 1e-9) {
            tools.push(d);
        }
    }
    tools.sort_by(f64::total_cmp);
    let mut out = String::new();
    out += "M48\n";
    let _ = writeln!(out, "; #@! TF.FileFunction,{function}");
    out += "FMAT,2\nMETRIC\n";
    for (i, t) in tools.iter().enumerate() {
        let _ = writeln!(out, "T{}C{:.3}", i + 1, t);
    }
    out += "%\nG90\nG05\n";
    let c = |v: f64| format!("{:.3}", v);
    for (i, t) in tools.iter().enumerate() {
        let mine: Vec<&&Hole> = holes
            .iter()
            .filter(|h| ((h.size[0].min(h.size[1]) * 1000.0).round() / 1000.0 - t).abs() < 1e-9)
            .collect();
        if mine.is_empty() {
            continue;
        }
        let _ = writeln!(out, "T{}", i + 1);
        for h in mine {
            let (w, hgt) = (h.size[0], h.size[1]);
            if (w - hgt).abs() < 1e-6 {
                let _ = writeln!(out, "X{}Y{}", c(h.at[0]), c(-h.at[1]));
            } else {
                let half = (w.max(hgt) - w.min(hgt)) / 2.0;
                let along = if w > hgt { h.rotation } else { h.rotation + 90.0 };
                let d = agentee_core::geom::rotate([half, 0.0], along);
                let (a, b) = ([h.at[0] - d[0], h.at[1] - d[1]], [h.at[0] + d[0], h.at[1] + d[1]]);
                let _ = writeln!(out, "X{}Y{}G85X{}Y{}", c(a[0]), c(-a[1]), c(b[0]), c(-b[1]));
            }
        }
    }
    out += "M30\n";
    out
}

fn csv(cells: &[String]) -> String {
    cells
        .iter()
        .map(|c| {
            if c.contains([',', '"', '\n']) {
                format!("\"{}\"", c.replace('"', "\"\""))
            } else {
                c.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn assembled(layout: &Layout, sch: &Schematic, reference: &str) -> bool {
    let part = sch.parts.iter().find(|p| p.reference == reference);
    let dnp = part.is_some_and(|p| p.dnp || p.fields.get("assembly").is_some_and(|v| v == "no"));
    let mech = layout.parts.iter().find(|p| p.reference == reference).is_some_and(|p| {
        p.footprint_name.starts_with("MountingHole") || p.footprint_name.starts_with("Fiducial")
    });
    !dnp && !mech
}

fn bom(layout: &Layout, sch: &Schematic) -> (String, String, usize) {
    let mut groups: Vec<(BomKey, Vec<String>)> = Vec::new();
    for p in &sch.parts {
        if !assembled(layout, sch, &p.reference) {
            continue;
        }
        let fp = p.footprint.clone().unwrap_or_default();
        let mpn = p.fields.get("mpn").cloned().unwrap_or_default();
        let lcsc = p.fields.get("lcsc").cloned().unwrap_or_default();
        let key = (p.value.clone(), fp, mpn, lcsc);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push(p.reference.clone()),
            None => groups.push((key, vec![p.reference.clone()])),
        }
    }
    for g in groups.iter_mut() {
        g.1.sort_by(|a, b| agentee_core::footprint::natural_cmp(a, b));
    }
    groups.sort_by(|a, b| agentee_core::footprint::natural_cmp(&a.1[0], &b.1[0]));
    let mut generic = String::from("Qty,Designators,Value,Footprint,MPN\n");
    let mut jlc = String::from("Comment,Designator,Footprint,LCSC Part #\n");
    for ((value, fp, mpn, lcsc), refs) in &groups {
        let _ = writeln!(
            generic,
            "{}",
            csv(&[refs.len().to_string(), refs.join(","), value.clone(), fp.clone(), mpn.clone()])
        );
        let _ =
            writeln!(jlc, "{}", csv(&[value.clone(), refs.join(","), fp.clone(), lcsc.clone()]));
    }
    (generic, jlc, groups.len())
}

fn cpl(layout: &Layout, sch: &Schematic) -> (String, usize) {
    let mut out = String::from("Designator,Mid X,Mid Y,Layer,Rotation\n");
    let mut n = 0;
    let mut parts: Vec<_> =
        layout.parts.iter().filter(|p| assembled(layout, sch, &p.reference)).collect();
    parts.sort_by(|a, b| agentee_core::footprint::natural_cmp(&a.reference, &b.reference));
    for p in parts {
        let at = p.at.to_mm();
        let _ = writeln!(
            out,
            "{}",
            csv(&[
                p.reference.clone(),
                format!("{:.4}mm", at[0]),
                format!("{:.4}mm", -at[1]),
                if p.bottom { "Bottom".into() } else { "Top".into() },
                format!("{}", p.rotation.rem_euclid(360.0)),
            ])
        );
        n += 1;
    }
    (out, n)
}

fn notes(layout: &Layout, board: &Board) -> String {
    let st = &board.stackup;
    let mut out = String::new();
    let _ = writeln!(out, "Board {} ({})", board.name, board.description);
    let _ = writeln!(out, "Layout {}", layout.name);
    if let Some(w) = &layout.watermark {
        let _ = writeln!(
            out,
            "Generated by {}, watermark on {} at [{:.2}, {:.2}] mm",
            w.text, w.layer, w.at[0], w.at[1]
        );
    }
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    layout.outline.iter().for_each(|p| b.add(*p));
    if !b.is_empty() {
        let [w, h] = b.size();
        let _ = writeln!(out, "Outline {w:.2} x {h:.2} mm");
    }
    if !layout.board_cutouts.is_empty() {
        let _ = writeln!(
            out,
            "Internal cutouts {}, routed through the board along Edge_Cuts",
            layout.board_cutouts.len()
        );
    }
    let _ = writeln!(out, "Finished thickness {:.2} mm", st.thickness().to_mm());
    let _ = writeln!(out, "Finish {}, mask {}, silk {}", st.finish, st.mask_color, st.silk_color);
    if let Some(p) = &st.preset {
        let _ = writeln!(out, "Stackup preset {p}");
    }
    out += "\nStackup, top to bottom:\n";
    for l in &st.layers {
        let extra = if l.kind.is_dielectric() {
            format!(", {} er {:.2}", l.material, l.er)
        } else {
            String::new()
        };
        let _ = writeln!(out, "  {:<10} {:?} {:.4} mm{extra}", l.name, l.kind, l.thickness.to_mm());
    }
    let controlled: Vec<_> = board.netclasses.iter().filter(|n| n.impedance.is_some()).collect();
    if !controlled.is_empty() {
        out += "\nControlled impedance:\n";
        for n in controlled {
            let _ = writeln!(
                out,
                "  {}: {} ohm +/-{}%, track {:.3} mm{}",
                n.name,
                n.impedance.map(|z| z.0).unwrap_or_default(),
                n.impedance_tolerance.0,
                n.track_width.to_mm(),
                n.coplanar_gap
                    .map(|g| format!(", coplanar gap {:.3} mm", g.to_mm()))
                    .unwrap_or_default()
            );
        }
    }
    out += &drill_notes(layout);
    let mut in_pad: Vec<usize> =
        agentee_core::drc::vias_in_pads(&layout.parts, &layout.vias).iter().map(|x| x.0).collect();
    in_pad.dedup();
    let n = in_pad.iter().filter(|&&v| layout.vias[v].fill.is_none()).count();
    if n > 0 {
        let _ = writeln!(out, "\n{n} vias sit in SMD pads: fill and cap them (IPC-4761 type VII).");
    }
    let edge: Vec<String> = layout
        .parts
        .iter()
        .flat_map(|p| {
            p.pads
                .iter()
                .zip(&p.footprint.pads)
                .filter(|(q, f)| f.edge && !q.copper.is_empty())
                .map(move |(q, _)| format!("{}.{}", p.reference, q.number))
        })
        .collect();
    if !edge.is_empty() {
        let _ = writeln!(
            out,
            "\nEdge pads, copper meant to reach the board edge, do not pull it back: {}.",
            edge.join(", ")
        );
    }
    let probes = agentee_core::testpoint::probes(&layout.test, &layout.parts, &layout.nets);
    if !probes.is_empty() {
        let _ = writeln!(
            out,
            "\n{} test points, probed from {}, listed in testpoints.csv.",
            probes.len(),
            if layout.test.bottom() { "the bottom" } else { "the top" }
        );
    }
    if layout.test.vias {
        let _ = writeln!(
            out,
            "\nVias are left untented on the {} side for probing.",
            if layout.test.bottom() { "bottom" } else { "top" }
        );
    }
    out += "\nCoordinates are mm, origin at the board's top-left corner, Y up in the Gerbers.\n";
    out
}

fn drill_notes(layout: &Layout) -> String {
    let cu = &layout.copper;
    let mut out = String::new();
    if layout
        .vias
        .iter()
        .all(|v| v.kind == ViaKind::Through && v.backdrill.is_none() && v.fill.is_none())
    {
        return out;
    }
    out += "\nVia types (IPC-4761 fill types), one drill file per span, controlled depth vias in their own:\n";
    let mut names: Vec<&str> = layout.vias.iter().map(|v| v.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let all: Vec<&agentee_core::layout::Via> =
            layout.vias.iter().filter(|v| v.name == name).collect();
        let v = all[0];
        let (a, b) = v.span_of(cu).unwrap_or((0, cu.len().saturating_sub(1)));
        let file = if v.kind == ViaKind::Through {
            "drill-PTH.drl".to_string()
        } else {
            span_file(cu, a, b, v.drill_kind == DrillKind::ControlledDepth)
        };
        let how = match v.drill_kind {
            DrillKind::Laser => "laser drilled".to_string(),
            DrillKind::Mechanical => "mechanically drilled".to_string(),
            DrillKind::ControlledDepth => {
                let (side, stop) = if a == 0 { (&cu[a], &cu[b]) } else { (&cu[b], &cu[a]) };
                format!(
                    "mechanically drilled from {side} to a controlled depth, stopping on {stop}"
                )
            }
        };
        let fill = v
            .fill
            .map(|f| format!(", {} (type {})", f.describe(), f.ipc4761()))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {name}: {} {} to {}, {how}, {:.3} mm drill, {:.3} mm pad, {} holes{fill}, {file}",
            v.kind.name(),
            cu[a],
            cu[b],
            v.drill,
            v.diameter,
            all.len()
        );
        if let Some(bd) = &v.backdrill {
            let _ = writeln!(
                out,
                "    backdrill from {} with a {:.3} mm drill, keep {} connected, leave at most {:.3} mm of stub, drill-backdrill-{}-{}.drl",
                bd.from,
                bd.diameter.to_mm(),
                bd.to,
                bd.max_stub.to_mm(),
                bd.from,
                bd.to
            );
        }
    }
    out
}

fn testpoints(layout: &Layout) -> String {
    let mut out = String::from("Ref,Pad,Net,X,Y,Side,Pad Diameter\n");
    for p in agentee_core::testpoint::probes(&layout.test, &layout.parts, &layout.nets) {
        let _ = writeln!(
            out,
            "{}",
            csv(&[
                p.reference,
                p.pad,
                p.net,
                format!("{:.4}mm", p.at[0]),
                format!("{:.4}mm", -p.at[1]),
                if p.side == "B" { "Bottom".into() } else { "Top".into() },
                format!("{:.4}mm", p.diameter),
            ])
        );
    }
    out
}

fn zip_files(dir: &Path, names: &[String], out: &Path) -> Result<(), String> {
    use std::io::Write as _;
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for n in names {
        let body = std::fs::read(dir.join(n)).map_err(|e| format!("{n}: {e}"))?;
        z.start_file(n.as_str(), opts).map_err(|e| e.to_string())?;
        z.write_all(&body).map_err(|e| e.to_string())?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn ipc356(layout: &Layout) -> String {
    let inch = |mm: f64| (mm / 25.4 * 10000.0).round() as i64;
    let coord = |v: f64| {
        let n = inch(v);
        format!("{}{:06}", if n < 0 { '-' } else { '+' }, n.abs())
    };
    let size = |mm: f64| format!("{:04}", inch(mm).clamp(0, 9999));
    let net = |n: Option<usize>| {
        let name = n.map(|k| layout.nets[k].name.clone()).unwrap_or_else(|| "N/C".into());
        let name: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        let tail: String =
            name.chars().rev().take(14).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{tail:<14}")
    };
    let n = layout.copper.len();
    let bottom = format!("A{:02}", n.max(2));
    let mut out = String::from("C  IPC-D-356 bare board netlist\n");
    let _ = writeln!(out, "C  {}", layout.name);
    out += "P  JOB   agentee\nP  CODE 00\nP  UNITS CUST 0\nP  arrayDim   N\n";
    for part in &layout.parts {
        for pad in part.pads.iter().filter(|q| !q.copper.is_empty()) {
            let mut b = agentee_core::graphic::Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|p| b.add(*p));
            let c = pad.drill.map(|d| d.0).unwrap_or_else(|| b.center());
            let rot = (part.rotation.rem_euclid(360.0)).round() as i64 % 360;
            let [w, h] = match rot {
                90 | 270 => [b.size()[1], b.size()[0]],
                _ => b.size(),
            };
            let reference: String = part.reference.chars().take(6).collect();
            let test_point = agentee_core::testpoint::is_test_point(part);
            let pin: String = pad.number.chars().take(4).collect();
            let (record, hole, access, mask) = match pad.drill {
                Some((_, d, _)) => (
                    317,
                    format!(
                        "D{}{}",
                        size(d[0].min(d[1])),
                        if pad.kind == PadKind::Npth { 'U' } else { 'P' }
                    ),
                    match (test_point, layout.test.bottom()) {
                        (true, true) => bottom.as_str(),
                        (true, false) => "A01",
                        (false, _) => "A00",
                    },
                    "S0",
                ),
                None if pad.copper.iter().any(|l| l == "F.Cu") => {
                    (327, "      ".to_string(), "A01", "S2")
                }
                None => (327, "      ".to_string(), bottom.as_str(), "S1"),
            };
            let _ = writeln!(
                out,
                "{record}{}   {reference:<6}-{pin:<4} {hole}{access}X{}Y{}X{}Y{}R{rot:03}{mask}",
                net(pad.net),
                coord(c[0]),
                coord(-c[1]),
                size(w),
                size(h),
            );
        }
    }
    let probe_cu = layout.test.copper();
    for v in &layout.vias {
        let probed = layout.test.vias && v.layers.contains(&probe_cu);
        let on: Vec<usize> =
            v.layers.iter().filter_map(|l| layout.copper.iter().position(|c| c == l)).collect();
        let a = on.iter().copied().min().unwrap_or(0);
        let b = on.iter().copied().max().unwrap_or(n.saturating_sub(1));
        let side = match (a == 0, b + 1 == n) {
            (true, true) => "A00".to_string(),
            (true, false) => "A01".to_string(),
            (false, true) => bottom.clone(),
            (false, false) => format!("A{:02}", a + 1),
        };
        let (mid, access, mask) = match (probed, layout.test.bottom()) {
            (false, _) => ('M', side, "S3"),
            (true, true) => (' ', bottom.clone(), "S1"),
            (true, false) => (' ', "A01".to_string(), "S2"),
        };
        let _ = writeln!(
            out,
            "317{}   VIA        {mid}D{}P{access}X{}Y{}X{}Y0000R000{mask}",
            net(Some(v.net)),
            size(v.drill),
            coord(v.at[0]),
            coord(-v.at[1]),
            size(v.diameter),
        );
    }
    out += "999\n";
    out
}

pub fn package(
    layout: &Layout,
    board: &Board,
    sch: &Schematic,
    dir: &Path,
) -> Result<Report, String> {
    if layout.watermark.is_none() {
        return Err(layout
            .watermark_problem
            .clone()
            .unwrap_or_else(|| "the layout has no watermark".into()));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut write = |name: String, body: String| -> Result<(), String> {
        std::fs::write(dir.join(&name), body).map_err(|e| format!("{name}: {e}"))?;
        files.push(name);
        Ok(())
    };
    let cu = &layout.copper;
    for (i, l) in cu.iter().enumerate() {
        write(
            format!("{}.gbr", layer_file(l)),
            copper(layout, l, &copper_function(i, cu.len())).finish(),
        )?;
    }
    let names: Vec<(String, LayerKind)> =
        board.stackup.layers.iter().map(|l| (l.name.clone(), l.kind)).collect();
    for (name, kind) in names {
        let top = name.starts_with("F.");
        let side = if top { "Top" } else { "Bot" };
        let g = match kind {
            LayerKind::Mask => openings(layout, &name, &format!("Soldermask,{side}"), false),
            LayerKind::Paste => openings(layout, &name, &format!("Paste,{side}"), true),
            LayerKind::Silk => silk(layout, board, &name, &format!("Legend,{side}")),
            _ => continue,
        };
        if kind == LayerKind::Paste && g.is_empty() {
            continue;
        }
        write(format!("{}.gbr", layer_file(&name)), g.finish())?;
    }
    write("Edge_Cuts.gbr".into(), edge(layout).finish())?;
    let hs = holes(layout);
    let pth: Vec<&Hole> = hs.iter().filter(|h| h.plated && h.span.is_none()).collect();
    let npth: Vec<&Hole> = hs.iter().filter(|h| !h.plated).collect();
    write("drill-PTH.drl".into(), excellon(&pth, true, cu.len()))?;
    if !npth.is_empty() {
        write("drill-NPTH.drl".into(), excellon(&npth, false, cu.len()))?;
    }
    for span in drill_spans(&hs, layout) {
        let body = excellon_with(&span.holes, &span.function).replacen(
            "M48\n",
            &format!("M48\n{}\n", span.comment),
            1,
        );
        write(span.file, body)?;
    }
    for (file, head, bd) in backdrills(layout) {
        let refs: Vec<&Hole> = bd.iter().collect();
        let (function, comment) = head.split_once('\n').unwrap_or((&head, ""));
        let body =
            excellon_with(&refs, function).replacen("M48\n", &format!("M48\n{comment}\n"), 1);
        write(file, body)?;
    }
    let (generic, jlc, lines) = bom(layout, sch);
    write("bom.csv".into(), generic)?;
    write("bom-jlcpcb.csv".into(), jlc)?;
    let (placement, placed) = cpl(layout, sch);
    write("cpl.csv".into(), placement)?;
    write("fab-notes.txt".into(), notes(layout, board))?;
    write(format!("{}.d356", layout.name), ipc356(layout))?;
    write("testpoints.csv".into(), testpoints(layout))?;
    let gerbers: Vec<String> =
        files.iter().filter(|f| f.ends_with(".gbr") || f.ends_with(".drl")).cloned().collect();
    let zip_name = format!("{}-gerbers.zip", layout.name);
    zip_files(dir, &gerbers, &dir.join(&zip_name))?;
    files.push(zip_name);
    Ok(Report {
        dir: dir.display().to_string(),
        files,
        parts_placed: placed,
        bom_lines: lines,
        holes: hs.len(),
    })
}

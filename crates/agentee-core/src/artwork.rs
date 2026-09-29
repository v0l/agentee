use crate::geom::{self, P};
use usvg::tiny_skia_path::{self, PathSegment};

pub const ICONS: &[(&str, &str)] = &[
    (
        "arrow",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><path d="M0 3.5H12V0L20 5L12 10V6.5H0Z"/></svg>"#,
    ),
    (
        "warning",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 18"><path fill-rule="evenodd" d="M10 0L20 18H0ZM10 4L3.4 16H16.6Z"/><path d="M9 6.5H11L10.6 12H9.4Z"/><circle cx="10" cy="14" r="1"/></svg>"#,
    ),
    (
        "ground",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M5.3 0H6.7V6H12V7.4H0V6H5.3ZM2 8.6H10V10H2ZM4 10.8H8V12H4Z"/></svg>"#,
    ),
    (
        "antenna",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 14"><path d="M0 0H1.6L6 5.2L10.4 0H12L6.8 6.2V14H5.2V6.2Z"/></svg>"#,
    ),
    (
        "lightning",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 16"><path d="M6 0L0 9H4.2L3 16L10 6H5.6Z"/></svg>"#,
    ),
    (
        "pin1",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0H10L0 10Z"/></svg>"#,
    ),
    (
        "ce",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 12"><path fill-rule="evenodd" d="M11 0.6A6 6 0 1 0 11 11.4V9.6A4.2 4.2 0 1 1 11 2.4Z"/><path d="M24 0.6A6 6 0 1 0 24 11.4V9.6A4.2 4.2 0 0 1 20.05 6.9H23V5.1H20.05A4.2 4.2 0 0 1 24 2.4Z"/></svg>"#,
    ),
];

pub fn icon(name: &str) -> Option<&'static str> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

pub fn icon_names() -> Vec<&'static str> {
    ICONS.iter().map(|(n, _)| *n).collect()
}

pub fn svg_polygons(svg: &str, height: f64) -> Result<Vec<Vec<P>>, String> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut rings: Vec<Vec<P>> = Vec::new();
    collect(tree.root(), &mut rings);
    rings.retain(|r| r.len() >= 3 && geom::signed_area(r).abs() > 1e-9);
    if rings.is_empty() {
        return Err("the SVG has no filled or stroked shapes".into());
    }
    let mut b = crate::graphic::Bounds::EMPTY;
    rings.iter().flatten().for_each(|p| b.add(*p));
    let scale = height / b.size()[1].max(1e-9);
    let c = b.center();
    for r in rings.iter_mut() {
        for p in r.iter_mut() {
            *p = [(p[0] - c[0]) * scale, (p[1] - c[1]) * scale];
        }
    }
    Ok(keyhole(rings))
}

fn collect(g: &usvg::Group, out: &mut Vec<Vec<P>>) {
    for node in g.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out),
            usvg::Node::Path(p) => {
                let ts = p.abs_transform();
                if p.fill().is_some()
                    && let Some(path) = p.data().clone().transform(ts)
                {
                    out.extend(flatten(&path));
                }
                if let Some(s) = p.stroke()
                    && let Some(outline) = p.data().stroke(&s.to_tiny_skia(), 4.0)
                    && let Some(path) = outline.transform(ts)
                {
                    out.extend(flatten(&path));
                }
            }
            _ => {}
        }
    }
}

fn flatten(path: &tiny_skia_path::Path) -> Vec<Vec<P>> {
    let size = path.bounds().width().max(path.bounds().height()) as f64;
    let steps =
        |a: P, b: P| ((geom::dist(a, b) / size.max(1e-9) * 64.0).ceil() as usize).clamp(4, 32);
    let mut rings = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let pt = |p: tiny_skia_path::Point| [p.x as f64, p.y as f64];
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if cur.len() >= 3 {
                    rings.push(std::mem::take(&mut cur));
                }
                cur.clear();
                cur.push(pt(p));
            }
            PathSegment::LineTo(p) => cur.push(pt(p)),
            PathSegment::QuadTo(c, p) => {
                let (a, c, e) = (*cur.last().unwrap_or(&[0.0, 0.0]), pt(c), pt(p));
                let n = steps(a, e) * 2;
                for k in 1..=n {
                    let t = k as f64 / n as f64;
                    let u = 1.0 - t;
                    cur.push([
                        u * u * a[0] + 2.0 * u * t * c[0] + t * t * e[0],
                        u * u * a[1] + 2.0 * u * t * c[1] + t * t * e[1],
                    ]);
                }
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let (a, b, c, e) = (*cur.last().unwrap_or(&[0.0, 0.0]), pt(c1), pt(c2), pt(p));
                let n = steps(a, e) * 2;
                for k in 1..=n {
                    let t = k as f64 / n as f64;
                    let u = 1.0 - t;
                    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push([
                        w0 * a[0] + w1 * b[0] + w2 * c[0] + w3 * e[0],
                        w0 * a[1] + w1 * b[1] + w2 * c[1] + w3 * e[1],
                    ]);
                }
            }
            PathSegment::Close => {
                if cur.len() >= 3 {
                    rings.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 3 {
        rings.push(cur);
    }
    for r in rings.iter_mut() {
        r.dedup_by(|a, b| geom::dist(*a, *b) < 1e-9);
        if r.len() > 1 && geom::dist(r[0], *r.last().unwrap()) < 1e-9 {
            r.pop();
        }
    }
    rings
}

fn keyhole(rings: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let depth: Vec<usize> = rings
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rings
                .iter()
                .enumerate()
                .filter(|(j, o)| *j != i && geom::point_in_polygon(r[0], o))
                .count()
        })
        .collect();
    let mut outers: Vec<(usize, Vec<P>)> = Vec::new();
    for (i, r) in rings.iter().enumerate() {
        if depth[i].is_multiple_of(2) {
            let mut r = r.clone();
            if geom::signed_area(&r) < 0.0 {
                r.reverse();
            }
            outers.push((i, r));
        }
    }
    for (i, hole) in rings.iter().enumerate() {
        if depth[i].is_multiple_of(2) {
            continue;
        }
        let Some(k) = outers
            .iter()
            .enumerate()
            .filter(|(_, (oi, o))| depth[*oi] + 1 == depth[i] && geom::point_in_polygon(hole[0], o))
            .map(|(k, _)| k)
            .next()
        else {
            continue;
        };
        let mut hole = hole.clone();
        if geom::signed_area(&hole) > 0.0 {
            hole.reverse();
        }
        let h = (0..hole.len()).max_by(|a, b| hole[*a][0].total_cmp(&hole[*b][0])).unwrap_or(0);
        let outer = &outers[k].1;
        let j = (0..outer.len())
            .min_by(|a, b| {
                geom::dist(outer[*a], hole[h]).total_cmp(&geom::dist(outer[*b], hole[h]))
            })
            .unwrap_or(0);
        let mut joined = outer[..=j].to_vec();
        joined.extend(hole[h..].iter().chain(hole[..=h].iter()).copied());
        joined.extend(outer[j..].iter().copied());
        outers[k].1 = joined;
    }
    outers.into_iter().map(|(_, r)| r).collect()
}

pub fn place(polys: &[Vec<P>], at: P, rotation: f64, mirror: bool) -> Vec<Vec<P>> {
    polys
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| {
                    let q = if mirror { [-p[0], p[1]] } else { *p };
                    let [x, y] = geom::rotate(q, rotation);
                    [x + at[0], y + at[1]]
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_to_the_asked_height() {
        for (name, svg) in ICONS {
            let polys = svg_polygons(svg, 3.0).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut b = crate::graphic::Bounds::EMPTY;
            polys.iter().flatten().for_each(|p| b.add(*p));
            assert!((b.size()[1] - 3.0).abs() < 1e-6, "{name}");
        }
    }

    #[test]
    fn a_ring_keeps_its_hole() {
        let polys = svg_polygons(icon("warning").unwrap(), 18.0).unwrap();
        let area: f64 = polys.iter().map(|r| geom::signed_area(r).abs()).sum();
        let outer = 0.5 * 20.0 * 18.0;
        let inner = 0.5 * 13.2 * 12.0;
        let bar = 0.5 * (2.0 + 1.2) * 5.5;
        let dot = std::f64::consts::PI;
        let want = outer - inner + bar + dot;
        assert!((area - want).abs() / want < 0.02, "{area} vs {want}");
    }
}

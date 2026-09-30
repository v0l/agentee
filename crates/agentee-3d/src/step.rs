use crate::{Mesh, MeshBuilder};
use std::collections::{HashMap, HashSet, VecDeque};
use truck_meshalgo::prelude::*;
use truck_stepio::r#in::{
    Table,
    alias::{
        BSplineSurface, Curve3D, KnotVec, Line, Surface, Tolerance, control_point::ControlPoint,
    },
};

type M = [[f64; 4]; 3];

const IDENTITY: M = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

fn mul(a: &M, b: &M) -> M {
    let mut out = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            out[i][j] =
                (0..3).map(|k| a[i][k] * b[k][j]).sum::<f64>() + if j == 3 { a[i][3] } else { 0.0 };
        }
    }
    out
}

fn inverse_rigid(a: &M) -> M {
    let mut out = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[j][i];
        }
        out[i][3] = -(0..3).map(|k| a[k][i] * a[k][3]).sum::<f64>();
    }
    out
}

struct Step<'a> {
    bodies: HashMap<u64, &'a str>,
}

fn records(text: &str) -> HashMap<u64, &str> {
    let mut out = HashMap::new();
    let Some(start) = text.find("DATA;") else { return out };
    let data = &text[start + 5..];
    let bytes = data.as_bytes();
    let (mut quoted, mut begin) = (false, 0usize);
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'\'' => quoted = !quoted,
            b';' if !quoted => {
                let rec = data[begin..i].trim();
                begin = i + 1;
                let Some(rest) = rec.strip_prefix('#') else { continue };
                let Some(eq) = rest.find('=') else { continue };
                if let Ok(id) = rest[..eq].trim().parse::<u64>() {
                    out.insert(id, rest[eq + 1..].trim());
                }
            }
            _ => {}
        }
    }
    out
}

fn kind(body: &str) -> &str {
    body.find('(').map(|i| body[..i].trim()).unwrap_or(body)
}

fn args(body: &str) -> Vec<&str> {
    let Some(open) = body.find('(') else { return Vec::new() };
    let (mut depth, mut quoted, mut begin) = (0i32, false, open + 1);
    let mut out = Vec::new();
    for (i, c) in body.char_indices().skip(open) {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    out.push(body[begin..i].trim());
                    break;
                }
            }
            ',' if !quoted && depth == 1 => {
                out.push(body[begin..i].trim());
                begin = i + 1;
            }
            _ => {}
        }
    }
    out
}

fn refs(s: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let j = i + 1 + b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if let Ok(v) = s[i + 1..j].parse() {
                out.push(v);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn floats(s: &str) -> Vec<f64> {
    s.trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect()
}

impl<'a> Step<'a> {
    fn kind(&self, id: u64) -> &str {
        self.bodies.get(&id).map(|b| kind(b)).unwrap_or("")
    }

    fn vector(&self, id: Option<u64>, kind: &str) -> Option<[f64; 3]> {
        let body = self.bodies.get(&id?)?;
        if !body.starts_with(kind) {
            return None;
        }
        let a = args(body);
        let v = floats(a.get(1)?);
        (v.len() >= 3).then(|| [v[0], v[1], v[2]])
    }

    fn placement(&self, id: u64, unit: f64) -> M {
        let Some(body) = self.bodies.get(&id) else { return IDENTITY };
        if !body.starts_with("AXIS2_PLACEMENT_3D") {
            return IDENTITY;
        }
        let a = args(body);
        let r = |k: usize| a.get(k).and_then(|s| refs(s).first().copied());
        let origin = self.vector(r(1), "CARTESIAN_POINT").unwrap_or([0.0; 3]);
        let norm = |v: [f64; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
            [v[0] / l, v[1] / l, v[2] / l]
        };
        let z = norm(self.vector(r(2), "DIRECTION").unwrap_or([0.0, 0.0, 1.0]));
        let xr = self.vector(r(3), "DIRECTION").unwrap_or([1.0, 0.0, 0.0]);
        let d = xr[0] * z[0] + xr[1] * z[1] + xr[2] * z[2];
        let x = norm([xr[0] - d * z[0], xr[1] - d * z[1], xr[2] - d * z[2]]);
        let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
        [
            [x[0], y[0], z[0], origin[0] * unit],
            [x[1], y[1], z[1], origin[1] * unit],
            [x[2], y[2], z[2], origin[2] * unit],
        ]
    }

    fn unit(&self) -> f64 {
        for body in self.bodies.values() {
            if body.starts_with('(') && body.contains("LENGTH_UNIT") {
                if body.contains("'INCH'") {
                    return 25.4;
                }
                if body.contains(".MILLI.") {
                    return 1.0;
                }
                if body.contains(".CENTI.") {
                    return 10.0;
                }
                if body.contains(".METRE.") {
                    return 1000.0;
                }
            }
        }
        1.0
    }

    fn colour_of(&self, id: u64, depth: usize) -> Option<[f32; 3]> {
        let body = self.bodies.get(&id)?;
        match kind(body) {
            "COLOUR_RGB" => {
                let a = args(body);
                let v: Vec<f32> = a.iter().skip(1).filter_map(|s| s.parse().ok()).collect();
                (v.len() == 3).then(|| [v[0], v[1], v[2]])
            }
            "DRAUGHTING_PRE_DEFINED_COLOUR" => Some(match args(body).first()?.trim_matches('\'') {
                "red" => [1.0, 0.0, 0.0],
                "green" => [0.0, 1.0, 0.0],
                "blue" => [0.0, 0.0, 1.0],
                "yellow" => [1.0, 1.0, 0.0],
                "magenta" => [1.0, 0.0, 1.0],
                "cyan" => [0.0, 1.0, 1.0],
                "black" => [0.05, 0.05, 0.05],
                "white" => [1.0, 1.0, 1.0],
                _ => return None,
            }),
            "CURVE_STYLE" | "POINT_STYLE" => None,
            _ if depth < 12 => refs(body).into_iter().find_map(|r| self.colour_of(r, depth + 1)),
            _ => None,
        }
    }
}

pub fn parse(text: &str) -> Result<Mesh, String> {
    let step = Step { bodies: records(text) };
    if step.bodies.is_empty() {
        return Err("no DATA section".into());
    }
    let unit = step.unit();
    let mut colours: HashMap<u64, [f32; 3]> = HashMap::new();
    for body in step.bodies.values() {
        let k = kind(body);
        if k == "STYLED_ITEM" || k == "OVER_RIDING_STYLED_ITEM" {
            let a = args(body);
            let (Some(styles), Some(item)) =
                (a.get(1), a.get(2).and_then(|s| refs(s).first().copied()))
            else {
                continue;
            };
            if let Some(c) = refs(styles).into_iter().find_map(|r| step.colour_of(r, 0)) {
                colours.insert(item, c);
            }
        }
    }
    let mut items: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut identity: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut children: HashMap<u64, Vec<(u64, M)>> = HashMap::new();
    for (&id, body) in &step.bodies {
        let k = kind(body);
        if body.starts_with('(') {
            if body.contains("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
                let r = refs(body);
                if r.len() >= 3 {
                    let t = step.bodies.get(&r[2]).copied().unwrap_or("");
                    let m = if t.starts_with("ITEM_DEFINED_TRANSFORMATION") {
                        let tr = refs(t);
                        if tr.len() >= 2 {
                            mul(
                                &step.placement(tr[1], unit),
                                &inverse_rigid(&step.placement(tr[0], unit)),
                            )
                        } else {
                            IDENTITY
                        }
                    } else {
                        IDENTITY
                    };
                    children.entry(r[1]).or_default().push((r[0], m));
                }
            }
            continue;
        }
        if k == "SHAPE_REPRESENTATION_RELATIONSHIP" || k == "REPRESENTATION_RELATIONSHIP" {
            let r = refs(body);
            if r.len() >= 2 {
                identity.entry(r[0]).or_default().push(r[1]);
                identity.entry(r[1]).or_default().push(r[0]);
            }
        } else if k.ends_with("SHAPE_REPRESENTATION")
            && k != "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"
        {
            let a = args(body);
            if let Some(list) = a.get(1) {
                items.insert(id, refs(list));
            }
        }
    }
    for (&rep, list) in &items {
        for &it in list {
            let body = step.bodies.get(&it).copied().unwrap_or("");
            if kind(body) != "MAPPED_ITEM" {
                continue;
            }
            let r = refs(body);
            let Some(map) = r.first().and_then(|m| step.bodies.get(m)) else { continue };
            let mr = refs(map);
            if mr.len() >= 2 && r.len() >= 2 {
                let m =
                    mul(&step.placement(r[1], unit), &inverse_rigid(&step.placement(mr[0], unit)));
                children.entry(rep).or_default().push((mr[1], m));
            }
        }
    }
    let component = |start: u64| -> Vec<u64> {
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        let mut out = Vec::new();
        while let Some(r) = queue.pop_front() {
            out.push(r);
            for &n in identity.get(&r).into_iter().flatten() {
                if seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        out
    };
    let mut placed_below: HashSet<u64> = HashSet::new();
    for list in children.values() {
        for (c, _) in list {
            placed_below.extend(component(*c));
        }
    }
    let mut instances: Vec<(u64, M)> = Vec::new();
    let mut done_roots: HashSet<u64> = HashSet::new();
    let mut reps: Vec<u64> = items.keys().copied().collect();
    reps.sort();
    for rep in reps {
        if placed_below.contains(&rep) || done_roots.contains(&rep) {
            continue;
        }
        let comp = component(rep);
        done_roots.extend(comp.iter().copied());
        let mut stack = vec![(comp, IDENTITY, 0usize)];
        while let Some((comp, m, depth)) = stack.pop() {
            for r in &comp {
                for &it in items.get(r).into_iter().flatten() {
                    instances.push((it, m));
                }
                if depth < 24 {
                    for (c, cm) in children.get(r).into_iter().flatten() {
                        stack.push((component(*c), mul(&m, cm), depth + 1));
                    }
                }
            }
        }
    }
    let text = text.replace(".PCURVE_S1.", ".CURVE_3D.").replace(".PCURVE_S2.", ".CURVE_3D.");
    let table = ruststep::parser::parse(&text)
        .map_err(|e| format!("STEP parse: {e:?}"))
        .map(|ex| ex.data.first().map(Table::from_data_section))?
        .ok_or("empty STEP")?;
    let mut shells: Vec<(u64, u64, M)> = Vec::new();
    let mut seen = HashSet::new();
    for (item, m) in instances {
        let body = step.bodies.get(&item).copied().unwrap_or("");
        let found: Vec<u64> = match kind(body) {
            "MANIFOLD_SOLID_BREP"
            | "BREP_WITH_VOIDS"
            | "SHELL_BASED_SURFACE_MODEL"
            | "FACETED_BREP" => {
                refs(body).into_iter().filter(|s| table.shell.contains_key(s)).collect()
            }
            "CLOSED_SHELL" | "OPEN_SHELL" => vec![item],
            _ => Vec::new(),
        };
        for s in found {
            let key = (s, m.iter().flatten().map(|v| (v * 1e6).round() as i64).collect::<Vec<_>>());
            if seen.insert(key) {
                shells.push((s, item, m));
            }
        }
    }
    if shells.is_empty() {
        let mut ids: Vec<u64> = table.shell.keys().copied().collect();
        ids.sort();
        shells = ids.into_iter().map(|s| (s, s, IDENTITY)).collect();
    }
    let mut out = MeshBuilder::default();
    let mut cache: HashMap<u64, Option<FaceTris>> = HashMap::new();
    for (shell, owner, m) in shells {
        let tris =
            cache.entry(shell).or_insert_with(|| triangulate(&step, &table, shell, unit)).clone();
        let Some(faces) = tris else { continue };
        let base =
            colours.get(&owner).or_else(|| colours.get(&shell)).copied().unwrap_or([0.6, 0.6, 0.6]);
        for (face, verts) in faces {
            let colour = face.and_then(|f| colours.get(&f)).copied().unwrap_or(base);
            for v in verts.chunks_exact(3) {
                let tri = [0, 1, 2].map(|k| {
                    let [p, n] = v[k];
                    let pw = [0, 1, 2]
                        .map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3]);
                    let nw = [0, 1, 2].map(|i| m[i][0] * n[0] + m[i][1] * n[1] + m[i][2] * n[2]);
                    (pw.map(|x| x as f32), nw.map(|x| x as f32))
                });
                out.push(colour, tri);
            }
        }
    }
    Ok(out.finish())
}

fn trim_to<C>(c: &mut C, a: Point3, b: Point3)
where
    C: ParametricCurve<Point = Point3>
        + BoundedCurve
        + Cut
        + SearchNearestParameter<D1, Point = Point3>,
{
    let tol = 1e-6;
    if c.front().distance(a) < tol && c.back().distance(b) < tol {
        return;
    }
    let (t0, t1) = c.range_tuple();
    let (Some(ta), Some(tb)) = (
        c.search_nearest_parameter(a, Some(t0), 100),
        c.search_nearest_parameter(b, Some(t1), 100),
    ) else {
        return;
    };
    let span = (t1 - t0).abs().max(1e-12);
    if tb - ta < span * 1e-6 || c.subs(ta).distance(a) > 1e-4 || c.subs(tb).distance(b) > 1e-4 {
        return;
    }
    if tb < t1 - span * 1e-9 {
        let _ = c.cut(tb);
    }
    if ta > t0 + span * 1e-9 {
        *c = c.cut(ta);
    }
}

fn valid_range(knots: &KnotVec, degree: usize) -> Option<(f64, f64)> {
    if knots.is_clamped(degree) || knots.len() < 2 * degree + 2 {
        return None;
    }
    let (lo, hi) = (knots[degree], knots[knots.len() - 1 - degree]);
    (hi > lo).then_some((lo, hi))
}

fn clamp_curve<C: Cut>(c: &mut C, range: Option<(f64, f64)>) {
    if let Some((lo, hi)) = range {
        let mut part = c.cut(lo);
        let _ = part.cut(hi);
        *c = part;
    }
}

fn clamp_surface<P: ControlPoint<f64> + Tolerance>(s: &mut BSplineSurface<P>) {
    if let Some((lo, hi)) = valid_range(s.uknot_vec(), s.udegree()) {
        let mut part = s.ucut(lo);
        let _ = part.ucut(hi);
        *s = part;
    }
    if let Some((lo, hi)) = valid_range(s.vknot_vec(), s.vdegree()) {
        let mut part = s.vcut(lo);
        let _ = part.vcut(hi);
        *s = part;
    }
}

fn finite_surface(s: &Surface) -> bool {
    let (ur, vr) = s.try_range_tuple();
    let ((u0, u1), (v0, v1)) = (ur.unwrap_or((0.0, 1.0)), vr.unwrap_or((0.0, 1.0)));
    [u0, u1, v0, v1].iter().all(|x| x.is_finite())
        && (0..=8).all(|i| {
            (0..=8).all(|j| {
                let p = s.subs(u0 + (u1 - u0) * i as f64 / 8.0, v0 + (v1 - v0) * j as f64 / 8.0);
                p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
            })
        })
}

fn finite_curve(c: &Curve3D) -> bool {
    let (t0, t1) = c.range_tuple();
    t0.is_finite()
        && t1.is_finite()
        && (0..=32).all(|i| {
            let p = c.subs(t0 + (t1 - t0) * i as f64 / 32.0);
            p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
        })
}

type FaceTris = Vec<(Option<u64>, Vec<[[f64; 3]; 2]>)>;

fn triangulate(step: &Step, table: &Table, shell: u64, unit: f64) -> Option<FaceTris> {
    let holder = table.shell.get(&shell)?;
    let face_ids: Vec<u64> = step
        .bodies
        .get(&shell)
        .map(|b| {
            refs(b)
                .into_iter()
                .map(|f| {
                    if step.kind(f) == "ORIENTED_FACE" {
                        step.bodies.get(&f).and_then(|b| refs(b).first().copied()).unwrap_or(f)
                    } else {
                        f
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut compressed = table.to_compressed_shell(holder).ok()?;
    let verts = &compressed.vertices;
    for e in compressed.edges.iter_mut() {
        let (a, b) = (verts[e.vertices.0], verts[e.vertices.1]);
        match &mut e.curve {
            Curve3D::BSplineCurve(c) => {
                clamp_curve(c, valid_range(c.knot_vec(), c.degree()));
                trim_to(c, a, b)
            }
            Curve3D::NurbsCurve(c) => {
                clamp_curve(c, valid_range(c.knot_vec(), c.degree()));
                trim_to(c, a, b)
            }
            _ => {}
        }
        if !finite_curve(&e.curve) {
            e.curve = Curve3D::Line(Line(a, b));
        }
    }
    for f in compressed.faces.iter_mut() {
        match &mut f.surface {
            Surface::BSplineSurface(s) => clamp_surface(s),
            Surface::NurbsSurface(s) => clamp_surface(s.non_rationalized_mut()),
            _ => {}
        }
    }
    let kept: Vec<usize> = (0..compressed.faces.len())
        .filter(|&i| finite_surface(&compressed.faces[i].surface))
        .collect();
    if kept.len() < compressed.faces.len() {
        compressed.faces = kept.iter().map(|&i| compressed.faces[i].clone()).collect();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for v in &compressed.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let diag = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
        let tol =
            if diag.is_finite() && diag > 0.0 { (diag * 0.002).max(0.002 / unit) } else { 0.01 };
        compressed.robust_triangulation(tol)
    }))
    .ok()?;
    let mut out = Vec::new();
    for (i, face) in result.faces.iter().enumerate() {
        let Some(poly) = &face.surface else { continue };
        let poly = if face.orientation { poly.clone() } else { poly.inverse() };
        let pos = poly.positions();
        let nor = poly.normals();
        let mut verts = Vec::new();
        for tri in poly.faces().triangle_iter() {
            let p = tri.map(|v| {
                let q = pos[v.pos];
                [q.x * unit, q.y * unit, q.z * unit]
            });
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let fnrm = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let fl = (fnrm[0] * fnrm[0] + fnrm[1] * fnrm[1] + fnrm[2] * fnrm[2]).sqrt();
            if fl < 1e-18 {
                continue;
            }
            let fnrm = fnrm.map(|x| x / fl);
            for k in 0..3 {
                let n = tri[k]
                    .nor
                    .and_then(|j| nor.get(j))
                    .map(|n| [n.x, n.y, n.z])
                    .filter(|n| n[0] * fnrm[0] + n[1] * fnrm[1] + n[2] * fnrm[2] > 0.0)
                    .unwrap_or(fnrm);
                verts.push([p[k], n]);
            }
        }
        out.push(((kept[i] < face_ids.len()).then(|| face_ids[kept[i]]), verts));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use truck_stepio::r#in::alias::{BSplineCurve, NurbsCurve};

    #[test]
    fn placements_compose_and_invert() {
        let a: M = [[0.0, -1.0, 0.0, 2.0], [1.0, 0.0, 0.0, 3.0], [0.0, 0.0, 1.0, 4.0]];
        let i = mul(&a, &inverse_rigid(&a));
        for r in 0..3 {
            for c in 0..4 {
                assert!((i[r][c] - IDENTITY[r][c]).abs() < 1e-12);
            }
        }
        assert_eq!(args("FOO('a,b',(#1,#2),#3)"), vec!["'a,b'", "(#1,#2)", "#3"]);
        assert_eq!(refs("(#12,#3) #7"), vec![12, 3, 7]);
    }

    #[test]
    fn filleted_box_loads_from_curve_3d() {
        let mesh = parse(include_str!("../tests/data/filleted_box.step")).unwrap();
        assert!(mesh.triangles() > 0);
        let (lo, hi) = mesh.bounds();
        for k in 0..3 {
            assert!((lo[k] + 0.5).abs() < 1e-4 && (hi[k] - 0.5).abs() < 1e-4, "{lo:?} {hi:?}");
        }
    }

    #[test]
    fn unclamped_rational_spline_is_clamped_to_its_valid_range() {
        let t = std::f64::consts::TAU / 3.0;
        let knots =
            KnotVec::from(vec![-t, 0.0, 0.0, t, t, 2.0 * t, 2.0 * t, 3.0 * t, 3.0 * t, 4.0 * t]);
        let w = [1.0, 0.5, 1.0, 0.5, 1.0, 0.5, 1.0];
        let pts = [
            (0.8, 0.0),
            (0.45, 0.0),
            (0.63, 0.3),
            (0.8, 0.6),
            (0.97, 0.3),
            (1.15, 0.0),
            (0.8, 0.0),
        ];
        let control =
            pts.iter().zip(w).map(|(&(x, y), w)| Vector4::new(x * w, y * w, 0.0, w)).collect();
        let mut nurbs = NurbsCurve::new(BSplineCurve::new(knots, control));
        assert!(!finite_curve(&Curve3D::NurbsCurve(nurbs.clone())));
        let range = valid_range(nurbs.knot_vec(), nurbs.degree());
        clamp_curve(&mut nurbs, range);
        assert_eq!(nurbs.range_tuple(), (0.0, 3.0 * t));
        assert!(finite_curve(&Curve3D::NurbsCurve(nurbs)));
        let line = Curve3D::Line(Line(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)));
        assert!(finite_curve(&line));
    }

    #[test]
    fn unclamped_surface_is_cut_to_its_valid_range() {
        let knots = KnotVec::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let control = (0..3)
            .map(|i| (0..3).map(|j| Point3::new(i as f64, j as f64, (i * j) as f64)).collect())
            .collect();
        let original = BSplineSurface::new((knots.clone(), knots), control);
        let mut clamped = original.clone();
        clamp_surface(&mut clamped);
        assert_eq!(clamped.range_tuple(), ((2.0, 3.0), (2.0, 3.0)));
        for (u, v) in [(2.0, 2.0), (2.5, 2.25), (3.0, 3.0)] {
            assert!(clamped.subs(u, v).distance(original.subs(u, v)) < 1e-9);
        }
    }
}

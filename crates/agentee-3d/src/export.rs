use crate::Mesh;
use agentee_core::board::Board;
use agentee_core::geom::P;
use agentee_core::layout::{Layout, Placed};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub type Affine = [[f64; 4]; 3];

const IDENTITY: Affine = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

pub fn rotation(deg: [f64; 3]) -> [[f64; 3]; 3] {
    let [x, y, z] = deg.map(|d| (-d).to_radians());
    let rx = [[1.0, 0.0, 0.0], [0.0, x.cos(), -x.sin()], [0.0, x.sin(), x.cos()]];
    let ry = [[y.cos(), 0.0, y.sin()], [0.0, 1.0, 0.0], [-y.sin(), 0.0, y.cos()]];
    let rz = [[z.cos(), -z.sin(), 0.0], [z.sin(), z.cos(), 0.0], [0.0, 0.0, 1.0]];
    let m = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
        let mut o = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        o
    };
    m(m(rz, ry), rx)
}

fn apply(m: &Affine, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
}

fn affine_of(f: impl Fn([f64; 3]) -> [f64; 3]) -> Affine {
    let o = f([0.0; 3]);
    let cols = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|e| {
        let q = f(e);
        [q[0] - o[0], q[1] - o[1], q[2] - o[2]]
    });
    [0, 1, 2].map(|i| [cols[0][i], cols[1][i], cols[2][i], o[i]])
}

fn compose(a: &Affine, b: &Affine) -> Affine {
    affine_of(|p| apply(a, apply(b, p)))
}

fn column(m: &Affine, j: usize) -> [f64; 3] {
    [m[0][j], m[1][j], m[2][j]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-12).then(|| a.map(|v| v / l))
}

fn is_rigid(m: &Affine) -> bool {
    let c = [column(m, 0), column(m, 1), column(m, 2)];
    let ortho = (0..3)
        .all(|i| (0..3).all(|j| (dot(c[i], c[j]) - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6));
    ortho && dot(cross(c[0], c[1]), c[2]) > 0.0
}

pub fn model_frame(fp: &agentee_core::footprint::Footprint) -> Affine {
    let r = rotation(fp.model_rotate);
    let (s, o) = (fp.model_scale, fp.model_offset);
    [0, 1, 2].map(|i| [r[i][0] * s[0], r[i][1] * s[1], r[i][2] * s[2], o[i]])
}

fn board_frame(part: &Placed, top: f64, bot: f64) -> Affine {
    let tf = part.transform();
    affine_of(|q| {
        let b = tf.apply([q[0], -q[1]]);
        let z = if part.bottom { bot - q[2] } else { top + q[2] };
        [b[0], -b[1], z]
    })
}

#[derive(Debug, Default)]
pub struct Report {
    pub file: PathBuf,
    pub parts: usize,
    pub step_models: Vec<String>,
    pub meshes: usize,
    pub boxes: Vec<String>,
    pub missing: Vec<String>,
}

fn real(v: f64) -> String {
    let v = if v.abs() < 1e-9 { 0.0 } else { v };
    let s = format!("{v:.6}");
    s.trim_end_matches('0').to_string()
}

fn text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' })
        .collect::<String>()
        .replace('\\', "_")
        .replace('\'', "''")
}

fn list(ids: &[u64]) -> String {
    ids.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(",")
}

struct Writer {
    out: String,
    next: u64,
    geom: u64,
    product_ctx: u64,
    definition_ctx: u64,
    origin: u64,
    styled: Vec<u64>,
    colours: HashMap<[u16; 3], u64>,
}

impl Writer {
    fn new() -> Self {
        let mut w = Writer {
            out: String::new(),
            next: 1,
            geom: 0,
            product_ctx: 0,
            definition_ctx: 0,
            origin: 0,
            styled: Vec::new(),
            colours: HashMap::new(),
        };
        let app =
            w.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
        w.add(format!(
            "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app})"
        ));
        w.product_ctx = w.add(format!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
        w.definition_ctx =
            w.add(format!("PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"));
        let mm = w.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))");
        let rad = w.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))");
        let sr = w.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())");
        let tol = w.add(format!(
            "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#{mm},'distance_accuracy_value','confusion accuracy')"
        ));
        w.geom = w.add(format!(
            "(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{tol}))GLOBAL_UNIT_ASSIGNED_CONTEXT((#{mm},#{rad},#{sr}))REPRESENTATION_CONTEXT('','3D'))"
        ));
        w.origin = w.axis([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        w
    }

    fn reserve(&mut self) -> u64 {
        self.next += 1;
        self.next - 1
    }

    fn put(&mut self, id: u64, body: impl AsRef<str>) {
        let _ = writeln!(self.out, "#{id}={};", body.as_ref());
    }

    fn add(&mut self, body: impl AsRef<str>) -> u64 {
        let id = self.reserve();
        self.put(id, body);
        id
    }

    fn point(&mut self, p: [f64; 3]) -> u64 {
        self.add(format!("CARTESIAN_POINT('',({},{},{}))", real(p[0]), real(p[1]), real(p[2])))
    }

    fn direction(&mut self, d: [f64; 3]) -> u64 {
        self.add(format!("DIRECTION('',({},{},{}))", real(d[0]), real(d[1]), real(d[2])))
    }

    fn axis(&mut self, at: [f64; 3], z: [f64; 3], x: [f64; 3]) -> u64 {
        let p = self.point(at);
        let a = self.direction(z);
        let b = self.direction(x);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{a},#{b})"))
    }

    fn product(&mut self, name: &str) -> u64 {
        let n = text(name);
        let p = self.add(format!("PRODUCT('{n}','{n}','',(#{}))", self.product_ctx));
        self.add(format!("PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{p}))"));
        let f = self.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{p})"));
        self.add(format!("PRODUCT_DEFINITION('design','',#{f},#{})", self.definition_ctx))
    }

    fn define(&mut self, pd: u64, rep: u64) {
        let s = self.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{pd})"));
        self.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{s},#{rep})"));
    }

    fn style(&mut self, item: u64, colour: [f32; 3]) {
        let key = colour.map(|c| (c.clamp(0.0, 1.0) * 1000.0) as u16);
        let psa = match self.colours.get(&key) {
            Some(&psa) => psa,
            None => {
                let [r, g, b] = colour.map(|c| real(c.clamp(0.0, 1.0) as f64));
                let c = self.add(format!("COLOUR_RGB('',{r},{g},{b})"));
                let fc = self.add(format!("FILL_AREA_STYLE_COLOUR('',#{c})"));
                let fs = self.add(format!("FILL_AREA_STYLE('',(#{fc}))"));
                let sfa = self.add(format!("SURFACE_STYLE_FILL_AREA(#{fs})"));
                let side = self.add(format!("SURFACE_SIDE_STYLE('',(#{sfa}))"));
                let usage = self.add(format!("SURFACE_STYLE_USAGE(.BOTH.,#{side})"));
                let psa = self.add(format!("PRESENTATION_STYLE_ASSIGNMENT((#{usage}))"));
                self.colours.insert(key, psa);
                psa
            }
        };
        let s = self.add(format!("STYLED_ITEM('color',(#{psa}),#{item})"));
        self.styled.push(s);
    }

    fn instance(
        &mut self,
        parent: (u64, u64),
        child: (u64, u64),
        name: &str,
        m: &Affine,
        items: &mut Vec<u64>,
    ) {
        let ax = self.axis(column(m, 3), column(m, 2), column(m, 0));
        items.push(ax);
        let n = text(name);
        let nauo = self.add(format!(
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE('{n}','{n}','',#{},#{},$)",
            parent.0, child.0
        ));
        let pds = self.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{nauo})"));
        let idt = self.add(format!("ITEM_DEFINED_TRANSFORMATION('','',#{},#{ax})", self.origin));
        let rel = self.add(format!(
            "(REPRESENTATION_RELATIONSHIP('','',#{},#{})REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{idt})SHAPE_REPRESENTATION_RELATIONSHIP())",
            child.1, parent.1
        ));
        self.add(format!("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{rel},#{pds})"));
    }
}

#[derive(Default)]
struct Points(HashMap<[i64; 3], u64>);

impl Points {
    fn get(&mut self, w: &mut Writer, p: [f64; 3]) -> u64 {
        let key = p.map(|v| (v * 1e6).round() as i64);
        *self.0.entry(key).or_insert_with(|| w.point(p))
    }
}

fn newell(ring: &[[f64; 3]]) -> Option<[f64; 3]> {
    let mut n = [0.0; 3];
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    unit(n)
}

fn face(w: &mut Writer, pts: &mut Points, rings: &[Vec<[f64; 3]>]) -> Option<u64> {
    let outer = rings.first()?;
    let normal = newell(outer)?;
    let edge = sub(outer[1], outer[0]);
    let x = unit(sub(edge, normal.map(|v| v * dot(edge, normal))))?;
    let mut bounds = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        let ids: Vec<u64> = ring.iter().map(|p| pts.get(w, *p)).collect();
        let lp = w.add(format!("POLY_LOOP('',({}))", list(&ids)));
        let kind = if i == 0 { "FACE_OUTER_BOUND" } else { "FACE_BOUND" };
        bounds.push(w.add(format!("{kind}('',#{lp},.T.)")));
    }
    let ax = w.axis(outer[0], normal, x);
    let plane = w.add(format!("PLANE('',#{ax})"));
    Some(w.add(format!("FACE_SURFACE('',({}),#{plane},.T.)", list(&bounds))))
}

type Triangles = Vec<([f32; 3], Vec<[[f64; 3]; 3]>)>;

fn mesh_triangles(mesh: &Mesh, m: &Affine) -> Triangles {
    mesh.parts
        .iter()
        .map(|part| {
            let tris = part
                .positions
                .chunks_exact(3)
                .map(|t| [0, 1, 2].map(|k| apply(m, t[k].map(|v| v as f64))))
                .collect();
            (part.colour, tris)
        })
        .collect()
}

fn surface_rep(w: &mut Writer, name: &str, groups: &Triangles) -> Option<u64> {
    let mut items = Vec::new();
    for (colour, tris) in groups {
        let mut pts = Points::default();
        let faces: Vec<u64> =
            tris.iter().filter_map(|t| face(w, &mut pts, &[t.to_vec()])).collect();
        if faces.is_empty() {
            continue;
        }
        let shell = w.add(format!("OPEN_SHELL('',({}))", list(&faces)));
        let model = w.add(format!("SHELL_BASED_SURFACE_MODEL('',(#{shell}))"));
        w.style(model, *colour);
        items.push(model);
    }
    if items.is_empty() {
        return None;
    }
    items.push(w.origin);
    Some(w.add(format!(
        "MANIFOLD_SURFACE_SHAPE_REPRESENTATION('{}',({}),#{})",
        text(name),
        list(&items),
        w.geom
    )))
}

fn signed_area(r: &[P]) -> f64 {
    (0..r.len())
        .map(|i| {
            let (p, q) = (r[i], r[(i + 1) % r.len()]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        / 2.0
}

fn inside(p: P, ring: &[P]) -> bool {
    let mut odd = false;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            odd = !odd;
        }
    }
    odd
}

fn disc(c: P, r: f64) -> Vec<P> {
    let n = ((r * 40.0) as usize).clamp(16, 64);
    (0..n)
        .map(|k| {
            let t = std::f64::consts::TAU * k as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

fn slot(c: P, d: [f64; 2], rot: f64) -> Vec<P> {
    let (w, h) = (d[0], d[1]);
    if (w - h).abs() < 1e-6 {
        return disc(c, w / 2.0);
    }
    let (r, half) = (w.min(h) / 2.0, (w.max(h) - w.min(h)) / 2.0);
    let ang = rot.to_radians() + if h > w { std::f64::consts::FRAC_PI_2 } else { 0.0 };
    let (dx, dy) = (half * ang.cos(), -half * ang.sin());
    let (a, b) = ([c[0] - dx, c[1] - dy], [c[0] + dx, c[1] + dy]);
    let n = 16;
    let heading = (b[1] - a[1]).atan2(b[0] - a[0]);
    let mut out = Vec::new();
    for (e, start) in
        [(b, heading - std::f64::consts::FRAC_PI_2), (a, heading + std::f64::consts::FRAC_PI_2)]
    {
        for k in 0..=n {
            let t = start + std::f64::consts::PI * k as f64 / n as f64;
            out.push([e[0] + r * t.cos(), e[1] + r * t.sin()]);
        }
    }
    out
}

fn tidy(ring: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for &p in ring {
        if out.last().is_none_or(|q: &P| (q[0] - p[0]).hypot(q[1] - p[1]) > 1e-6) {
            out.push(p);
        }
    }
    while out.len() > 2
        && (out[0][0] - out[out.len() - 1][0]).hypot(out[0][1] - out[out.len() - 1][1]) < 1e-6
    {
        out.pop();
    }
    out
}

fn board_holes(l: &Layout, outline: &[P]) -> Vec<Vec<P>> {
    let mut rings: Vec<Vec<P>> = Vec::new();
    for part in &l.parts {
        for pad in &part.pads {
            if let Some((c, d, rot)) = pad.drill {
                rings.push(slot(c, d, rot));
            }
        }
    }
    rings.extend(l.board_cutouts.iter().filter(|c| c.len() >= 3).cloned());
    let mut kept: Vec<Vec<P>> = Vec::new();
    for ring in rings {
        let ring: Vec<P> = tidy(&ring).into_iter().map(|p| [p[0], -p[1]]).collect();
        if ring.len() < 3 || !ring.iter().all(|&p| inside(p, outline)) {
            continue;
        }
        if kept
            .iter()
            .any(|k| ring.iter().any(|&p| inside(p, k)) || k.iter().any(|&p| inside(p, &ring)))
        {
            continue;
        }
        kept.push(ring);
    }
    for ring in &mut kept {
        if signed_area(ring) > 0.0 {
            ring.reverse();
        }
    }
    kept
}

fn board_rep(w: &mut Writer, l: &Layout, board: &Board, t: f64) -> Option<u64> {
    let mut outline: Vec<P> = tidy(&l.outline).into_iter().map(|p| [p[0], -p[1]]).collect();
    if outline.len() < 3 {
        return None;
    }
    if signed_area(&outline) < 0.0 {
        outline.reverse();
    }
    let holes = board_holes(l, &outline);
    let lift = |r: &[P], z: f64| -> Vec<[f64; 3]> { r.iter().map(|p| [p[0], p[1], z]).collect() };
    let mut pts = Points::default();
    let mut faces = Vec::new();
    let mut top = vec![lift(&outline, t)];
    top.extend(holes.iter().map(|h| lift(h, t)));
    faces.extend(face(w, &mut pts, &top));
    let bottom: Vec<Vec<[f64; 3]>> =
        top.iter().map(|r| r.iter().rev().map(|p| [p[0], p[1], 0.0]).collect()).collect();
    faces.extend(face(w, &mut pts, &bottom));
    for ring in std::iter::once(&outline).chain(&holes) {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let quad = vec![[a[0], a[1], 0.0], [b[0], b[1], 0.0], [b[0], b[1], t], [a[0], a[1], t]];
            faces.extend(face(w, &mut pts, &[quad]));
        }
    }
    let shell = w.add(format!("CLOSED_SHELL('',({}))", list(&faces)));
    let brep = w.add(format!("FACETED_BREP('',#{shell})"));
    w.style(brep, mask_rgb(&board.stackup.mask_color));
    Some(w.add(format!(
        "FACETED_BREP_SHAPE_REPRESENTATION('{}',(#{brep},#{}),#{})",
        text(&l.name),
        w.origin,
        w.geom
    )))
}

fn mask_rgb(name: &str) -> [f32; 3] {
    let c: [u8; 3] = match name.to_lowercase().as_str() {
        "black" => [22, 24, 24],
        "blue" => [20, 50, 120],
        "red" => [140, 24, 24],
        "white" => [225, 225, 220],
        "yellow" => [200, 170, 30],
        "purple" => [80, 30, 110],
        "matte black" => [28, 28, 28],
        _ => [24, 90, 44],
    };
    c.map(|v| v as f32 / 255.0)
}

struct Record<'a> {
    id: u64,
    body: &'a str,
}

fn records(data: &str) -> Vec<Record<'_>> {
    let b = data.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b[i..].starts_with(b"/*") {
            i = data[i..].find("*/").map_or(b.len(), |e| i + e + 2);
            continue;
        }
        if i >= b.len() || b[i] != b'#' {
            break;
        }
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let Ok(id) = data[start..i].parse::<u64>() else { break };
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'=') {
            i += 1;
        }
        let body_start = i;
        let mut quoted = false;
        while i < b.len() && (quoted || b[i] != b';') {
            if b[i] == b'\'' {
                quoted = !quoted;
            }
            i += 1;
        }
        out.push(Record { id, body: data[body_start..i].trim() });
        i += 1;
    }
    out
}

fn shift(body: &str, base: u64) -> String {
    let b = body.as_bytes();
    let mut out = String::with_capacity(body.len() + 16);
    let (mut i, mut quoted, mut from) = (0, false, 0);
    while i < b.len() {
        if b[i] == b'\'' {
            quoted = !quoted;
        } else if !quoted && b[i] == b'#' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if let Ok(n) = body[i + 1..j].parse::<u64>() {
                out.push_str(&body[from..i]);
                let _ = write!(out, "#{}", n + base);
                from = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&body[from..]);
    out.replace(['\r', '\n'], " ")
}

fn kind(body: &str) -> &str {
    body.split('(').next().unwrap_or("").trim()
}

fn args(body: &str) -> Vec<&str> {
    let Some(open) = body.find('(') else { return Vec::new() };
    let b = body.as_bytes();
    let (mut depth, mut quoted, mut start) = (0usize, false, open + 1);
    let mut out = Vec::new();
    for i in open..b.len() {
        match b[i] {
            b'\'' => quoted = !quoted,
            _ if quoted => {}
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    out.push(body[start..i].trim());
                    break;
                }
            }
            b',' if depth == 1 => {
                out.push(body[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

fn reference(arg: Option<&&str>) -> Option<u64> {
    arg?.trim().strip_prefix('#')?.parse().ok()
}

fn embed(w: &mut Writer, path: &Path) -> Result<Vec<(u64, u64)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let header_end = text.find("ENDSEC;").ok_or("no header")?;
    let data_at = text[header_end..].find("DATA;").ok_or("no DATA section")? + header_end + 5;
    let recs = records(&text[data_at..]);
    let max = recs.iter().map(|r| r.id).max().ok_or("empty DATA section")?;
    let base = w.next;
    let bodies: HashMap<u64, &str> = recs.iter().map(|r| (r.id, r.body)).collect();
    let mut definitions = Vec::new();
    let mut used = std::collections::HashSet::new();
    let mut rep_of = HashMap::new();
    for r in &recs {
        let k = kind(r.body);
        let a = args(r.body);
        if k == "PRODUCT_DEFINITION" {
            definitions.push(r.id);
        } else if k.ends_with("ASSEMBLY_USAGE_OCCURRENCE") {
            used.extend(reference(a.get(4)));
        } else if k == "SHAPE_DEFINITION_REPRESENTATION" {
            let pd = reference(a.first())
                .and_then(|pds| bodies.get(&pds))
                .and_then(|pds| reference(args(pds).get(2)));
            if let (Some(pd), Some(rep)) = (pd, reference(a.get(1))) {
                rep_of.insert(pd, rep);
            }
        }
    }
    let roots: Vec<(u64, u64)> = definitions
        .into_iter()
        .filter(|d| !used.contains(d))
        .filter_map(|d| rep_of.get(&d).map(|&r| (d + base, r + base)))
        .collect();
    if roots.is_empty() {
        return Err(format!("{}: no root product with a shape", path.display()));
    }
    w.next = base + max + 1;
    for r in &recs {
        let _ = writeln!(w.out, "#{}={};", r.id + base, shift(r.body, base));
    }
    Ok(roots)
}

fn box_mesh(part: &Placed) -> Option<Mesh> {
    let fp = &part.footprint;
    let h = fp.height.unwrap_or(1.0);
    if h <= 0.0 {
        return None;
    }
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    for g in fp.graphics.iter().filter(|g| g.layer == "F.Fab") {
        if !matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
            b.union(&g.bounds());
        }
    }
    if b.is_empty() {
        b = fp.bounds();
    }
    if b.is_empty() {
        return None;
    }
    let (x0, x1, y0, y1) = (b.min[0] as f32, b.max[0] as f32, -b.max[1] as f32, -b.min[1] as f32);
    let h = h as f32;
    let c = |i: usize| -> [f32; 3] {
        [
            if i & 1 == 0 { x0 } else { x1 },
            if i & 2 == 0 { y0 } else { y1 },
            if i & 4 == 0 { 0.0 } else { h },
        ]
    };
    let quads =
        [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut mb = crate::MeshBuilder::default();
    for q in quads {
        let colour = [0.15, 0.15, 0.16];
        mb.push_flat(colour, [c(q[0]), c(q[1]), c(q[2])]);
        mb.push_flat(colour, [c(q[0]), c(q[2]), c(q[3])]);
    }
    Some(mb.finish())
}

enum Model {
    Step(PathBuf),
    Mesh(Mesh, bool),
    Nothing,
    Missing(String),
}

fn resolve(part: &Placed, root: &Path) -> Model {
    let fp = &part.footprint;
    let own = fp.model.as_deref().filter(|m| crate::in_project(m, root).is_some());
    if own.is_none()
        && let Some(mesh) = crate::parametric::generate(fp)
    {
        return if mesh.parts.is_empty() { Model::Nothing } else { Model::Mesh(mesh, false) };
    }
    let Some(model) = &fp.model else { return Model::Missing(String::new()) };
    let path = match crate::locate(model, root).map_or_else(|| crate::fetch(model), Ok) {
        Ok(p) => p,
        Err(e) => return Model::Missing(e),
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "step" || ext == "stp" {
        return Model::Step(path);
    }
    match crate::load(&path) {
        Ok(mesh) => Model::Mesh(mesh, true),
        Err(e) => Model::Missing(e),
    }
}

pub fn step(l: &Layout, board: &Board, root: &Path, out: &Path) -> Result<Report, String> {
    let t = board.stackup.thickness().to_mm().max(0.4);
    let mut w = Writer::new();
    let mut report = Report { file: out.to_path_buf(), ..Default::default() };
    let top_pd = w.product(&l.name);
    let top_rep = w.reserve();
    let mut items = vec![w.origin];
    let pcb_pd = w.product(&format!("{} PCB", l.name));
    let pcb_rep = board_rep(&mut w, l, board, t).ok_or("the layout has no board outline")?;
    w.define(pcb_pd, pcb_rep);
    w.instance((top_pd, top_rep), (pcb_pd, pcb_rep), "PCB", &IDENTITY, &mut items);
    let mut embedded: HashMap<PathBuf, Option<Vec<(u64, u64)>>> = HashMap::new();
    let mut meshes: HashMap<String, Option<(u64, u64)>> = HashMap::new();
    for part in &l.parts {
        let place = board_frame(part, t, 0.0);
        let fp = &part.footprint;
        let model = match resolve(part, root) {
            Model::Step(path) => {
                let whole = compose(&place, &model_frame(fp));
                let roots = embedded
                    .entry(path.clone())
                    .or_insert_with(|| embed(&mut w, &path).ok())
                    .clone()
                    .filter(|_| is_rigid(&whole));
                if let Some(roots) = roots {
                    for child in roots {
                        w.instance((top_pd, top_rep), child, &part.reference, &whole, &mut items);
                    }
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    if let Some(name) = name.filter(|n| !report.step_models.contains(n)) {
                        report.step_models.push(name);
                    }
                    report.parts += 1;
                    continue;
                }
                match crate::load(&path) {
                    Ok(mesh) => Model::Mesh(mesh, true),
                    Err(e) => Model::Missing(e),
                }
            }
            other => other,
        };
        let (key, mesh, frame) = match model {
            Model::Nothing | Model::Step(_) => continue,
            Model::Mesh(mesh, frame) => {
                report.meshes += 1;
                (format!("{}:{frame}", fp.name), Some(mesh), frame)
            }
            Model::Missing(e) => {
                if !e.is_empty() && !report.missing.contains(&e) {
                    report.missing.push(e);
                }
                report.boxes.push(part.reference.clone());
                (format!("{}:box", fp.name), box_mesh(part), false)
            }
        };
        let component = meshes.entry(key).or_insert_with(|| {
            let mesh = mesh?;
            let local = if frame { model_frame(fp) } else { IDENTITY };
            let rep = surface_rep(&mut w, &fp.name, &mesh_triangles(&mesh, &local))?;
            let pd = w.product(&fp.name);
            w.define(pd, rep);
            Some((pd, rep))
        });
        if let Some(child) = *component {
            w.instance((top_pd, top_rep), child, &part.reference, &place, &mut items);
            report.parts += 1;
        }
    }
    w.put(
        top_rep,
        format!("SHAPE_REPRESENTATION('{}',({}),#{})", text(&l.name), list(&items), w.geom),
    );
    w.define(top_pd, top_rep);
    if !w.styled.is_empty() {
        let styled = std::mem::take(&mut w.styled);
        w.add(format!(
            "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',({}),#{})",
            list(&styled),
            w.geom
        ));
    }
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let file = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('{}'),'2;1');\nFILE_NAME('{}','',(''),(''),'agentee','agentee','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;\n{}ENDSEC;\nEND-ISO-10303-21;\n",
        text(&l.name),
        text(&name),
        w.out
    );
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, file).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renumbering_skips_strings_and_shifts_every_reference() {
        assert_eq!(shift("FOO('#1 it''s',#2,(#30,#4))", 100), "FOO('#1 it''s',#102,(#130,#104))");
    }

    #[test]
    fn records_split_on_semicolons_outside_strings() {
        let r = records("#1=A('x;y');\n/* c */ #2 = B(#1) ;\nENDSEC;");
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].id, r[0].body), (1, "A('x;y')"));
        assert_eq!((r[1].id, r[1].body), (2, "B(#1)"));
    }

    #[test]
    fn arguments_split_at_the_top_level() {
        assert_eq!(args("NAUO('a,b','',(#1,#2),#3)"), vec!["'a,b'", "''", "(#1,#2)", "#3"]);
    }

    const PART_STEP: &str = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n\
        #1=PRODUCT('blob','blob','',(#9));\n#2=PRODUCT_DEFINITION_FORMATION('','',#1);\n\
        #3=PRODUCT_DEFINITION('design','',#2,#9);\n#4=PRODUCT_DEFINITION_SHAPE('','',#3);\n\
        #5=SHAPE_DEFINITION_REPRESENTATION(#4,#6);\n#6=SHAPE_REPRESENTATION('',(#7),#9);\n\
        #7=AXIS2_PLACEMENT_3D('',#8,$,$);\n#8=CARTESIAN_POINT('',(0.,0.,0.));\n#9=APPLICATION_CONTEXT('x');\n\
        ENDSEC;\nEND-ISO-10303-21;\n";

    fn part_files(dir: &Path, name: &str, model: &str) {
        std::fs::write(
            dir.join(format!("symbols/{name}.sym.toml")),
            format!("name = \"{name}\"\nreference = \"U\"\nfootprint = \"{name}\"\n[[pins]]\nnumber = \"1\"\nat = [0, 0]\nside = \"left\"\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("footprints/{name}.fp.toml")),
            format!("name = \"{name}\"\n{model}height = 2.0\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"rect\"\nat = [0, 0]\nsize = [1, 1]\nlayers = [\"F.Cu\", \"F.Mask\"]\n[[graphics]]\nkind = \"rect\"\nlayer = \"F.Fab\"\nstart = [-1, -1]\nend = [1, 1]\nwidth = 0.1\n"),
        )
        .unwrap();
    }

    #[test]
    fn a_layout_exports_as_one_assembly_whose_references_all_resolve() {
        let dir = std::env::temp_dir().join(format!("agentee-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["symbols", "footprints", "3dmodels"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("3dmodels/blob.step"), PART_STEP).unwrap();
        part_files(&dir, "Blob", "model = \"3dmodels/blob.step\"\n");
        part_files(&dir, "Lump", "");
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\n[outline]\nsize = [20, 10]\n[[outline.cutouts]]\norigin = [14, 3]\nsize = [4, 4]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.sch.toml"),
            "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"U1\"\nsymbol = \"Blob\"\nat = [0, 0]\n[[parts]]\nref = \"U2\"\nsymbol = \"Lump\"\nat = [12.7, 0]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"U1\"\nat = [5, 4]\n[[footprints]]\nref = \"U2\"\nat = [10, 5]\nside = \"bottom\"\n",
        )
        .unwrap();
        let p = agentee_core::Project::load(&dir).unwrap();
        let (l, board) = (&p.layouts[0].item, &p.boards[0].item);
        let out = dir.join("t.step");
        let r = step(l, board, &dir, &out).unwrap();
        let t = board.stackup.thickness().to_mm();
        let file = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(file.starts_with("ISO-10303-21;"));
        assert_eq!(r.step_models, vec!["blob.step"]);
        assert_eq!((r.meshes, r.boxes.len()), (1, 0));
        assert_eq!(r.parts, 2);
        let data = &file[file.find("DATA;").unwrap() + 5..];
        let recs = records(data);
        let ids: std::collections::HashSet<u64> = recs.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), recs.len(), "an id is defined twice");
        for rec in &recs {
            for word in shift(rec.body, 0).split(|c: char| !(c == '#' || c.is_ascii_digit())) {
                if let Some(n) = word.strip_prefix('#').and_then(|n| n.parse::<u64>().ok()) {
                    assert!(ids.contains(&n), "#{} names #{n}, which is not defined", rec.id);
                }
            }
        }
        let count = |k: &str| recs.iter().filter(|r| kind(r.body) == k).count();
        assert_eq!(count("NEXT_ASSEMBLY_USAGE_OCCURRENCE"), 3);
        assert_eq!(count("FACETED_BREP"), 1);
        assert!(file.contains(&format!("CARTESIAN_POINT('',(5.,-4.,{}))", real(t))));
        assert!(file.contains("CARTESIAN_POINT('',(10.,-5.,0.))"));
    }

    #[test]
    fn model_rotation_about_x_stands_a_model_up_rigidly() {
        let m: Affine = {
            let r = rotation([-90.0, 0.0, 0.0]);
            [0, 1, 2].map(|i| [r[i][0], r[i][1], r[i][2], 0.0])
        };
        assert!(is_rigid(&m));
        let p = apply(&m, [0.0, 1.0, -2.0]);
        assert!((p[0]).abs() < 1e-9 && (p[1] - 2.0).abs() < 1e-9 && (p[2] - 1.0).abs() < 1e-9);
    }
}

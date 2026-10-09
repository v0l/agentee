use crate::raster::{Canvas, Textures};
use agentee_3d::export::rotation;
use agentee_3d::{Fetch, Status};
use agentee_core::board::Board;
use agentee_core::footprint::PadKind;
use agentee_core::geom::P;
use agentee_core::layout::{Layout, Placed};
use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};
use egui_bench::viewer3d::{
    self, Camera, Image, Look, Material, Orbit, Shading, Style, V3, Viewer, cross, norm, sub,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Finish {
    #[default]
    Board,
    Metal,
}

impl Material for Finish {
    fn shading(&self) -> Shading {
        match self {
            Finish::Board => Shading { gloss: 0.06, sharpness: 24.0, ..Shading::default() },
            Finish::Metal => {
                Shading { gloss: 0.46, sharpness: 81.6, metal: 0.8, ..Shading::default() }
            }
        }
    }
}

pub type Surface = viewer3d::Surface<Finish>;

pub const BOARD: usize = 0;
pub const PARTS: usize = 1;

pub struct Scene {
    pub mesh: Arc<viewer3d::Scene<Finish>>,
    pub pending: usize,
    pub missing: Vec<String>,
}

pub const STYLE: Style = Style {
    background: viewer3d::BACKGROUND,
    ambient: 0.30,
    key: 0.80,
    fill: 0.25,
    cap: Color32::from_rgb(237, 140, 46),
    selected: Color32::from_rgb(245, 166, 59),
    selected_tint: 0.35,
};

pub type SoftCache = Option<((u64, Camera, [u32; 2], bool), TextureHandle)>;

pub struct State {
    pub camera: Camera,
    pub orbit: Orbit,
    pub region: Option<(V3, f32)>,
    soft: SoftCache,
}

fn home() -> Camera {
    Camera { yaw: 0.35, pitch: 0.85, ortho: false, ..Camera::default() }
}

impl Default for State {
    fn default() -> Self {
        State { camera: home(), orbit: Orbit::default(), region: None, soft: None }
    }
}

fn mask_rgb(name: &str) -> Color32 {
    match name.to_lowercase().as_str() {
        "black" => Color32::from_rgb(22, 24, 24),
        "blue" => Color32::from_rgb(20, 50, 120),
        "red" => Color32::from_rgb(140, 24, 24),
        "white" => Color32::from_rgb(225, 225, 220),
        "yellow" => Color32::from_rgb(200, 170, 30),
        "purple" => Color32::from_rgb(80, 30, 110),
        "matte black" => Color32::from_rgb(28, 28, 28),
        _ => Color32::from_rgb(24, 90, 44),
    }
}

fn silk_rgb(name: &str) -> Color32 {
    match name.to_lowercase().as_str() {
        "black" => Color32::from_rgb(20, 20, 20),
        "yellow" => Color32::from_rgb(230, 200, 40),
        _ => Color32::from_rgb(236, 236, 232),
    }
}

fn finish_rgb(name: &str) -> Color32 {
    let n = name.to_lowercase();
    if n.contains("enig") || n.contains("gold") {
        Color32::from_rgb(214, 172, 86)
    } else if n.contains("osp") {
        Color32::from_rgb(196, 118, 80)
    } else {
        Color32::from_rgb(196, 198, 200)
    }
}

fn lift(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| ((v as f32 * k).min(255.0)) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

fn rgb(c: Color32) -> [f32; 3] {
    [c.r(), c.g(), c.b()].map(|v| v as f32 / 255.0)
}

fn capsule(a: P, b: P, r: f64) -> Vec<P> {
    let n = 12;
    let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
    let mut out = Vec::new();
    for (c, start) in
        [(b, ang - std::f64::consts::FRAC_PI_2), (a, ang + std::f64::consts::FRAC_PI_2)]
    {
        for k in 0..=n {
            let t = start + std::f64::consts::PI * k as f64 / n as f64;
            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        }
    }
    out
}

fn disc(c: P, r: f64, n: usize) -> Vec<P> {
    (0..n)
        .map(|k| {
            let t = std::f64::consts::TAU * k as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

type Decal = Vec<([P; 3], Color32)>;

fn graphic_areas(part: &Placed, layer: &str) -> Vec<Vec<P>> {
    let tf = part.transform();
    let mut out = Vec::new();
    for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == layer) {
        if matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
            continue;
        }
        let path: Vec<P> =
            agentee_core::footprint::graphic_path(g).into_iter().map(|q| tf.apply(q)).collect();
        if g.fill == agentee_core::graphic::Fill::Solid && path.len() >= 3 {
            out.push(path.clone());
        }
        let w = g.width.to_mm() / 2.0;
        if w > 0.0 {
            out.extend(path.windows(2).map(|s| capsule(s[0], s[1], w)));
        }
    }
    out
}

fn overlaps(a: &[P], b: &[P]) -> bool {
    let bounds = |r: &[P]| {
        let mut x = agentee_core::graphic::Bounds::EMPTY;
        r.iter().for_each(|q| x.add(*q));
        x
    };
    bounds(a).overlaps(&bounds(b))
}

fn flat(polys: &[Vec<P>], color: Color32, out: &mut Decal) {
    for t in agentee_core::contour::triangles(polys) {
        out.push((t, color));
    }
}

fn body_height(name: &str) -> (f32, Color32) {
    let n = name.to_uppercase();
    let dark = Color32::from_rgb(38, 38, 40);
    if n.starts_with("C_0402") || n.starts_with("C_0201") {
        (0.5, Color32::from_rgb(170, 140, 100))
    } else if n.starts_with("C_") {
        (0.8, Color32::from_rgb(170, 140, 100))
    } else if n.starts_with("R_") {
        (0.35, dark)
    } else if n.starts_with("L_") {
        (0.8, Color32::from_rgb(90, 80, 70))
    } else if n.contains("BCR") {
        (4.06, Color32::from_rgb(60, 60, 64))
    } else if n.starts_with("LED") {
        (0.6, Color32::from_rgb(120, 220, 140))
    } else if n.starts_with("SOT") || n.starts_with("SOIC") || n.starts_with("QFN") {
        (1.5, dark)
    } else if n.starts_with("D_") {
        (0.9, dark)
    } else if n.contains("SMA") {
        (4.0, Color32::from_rgb(214, 180, 96))
    } else if n.starts_with("PINHEADER") {
        (2.5, dark)
    } else if n.starts_with("MOUNTINGHOLE") {
        (0.0, dark)
    } else {
        (1.0, dark)
    }
}

struct Hole {
    ring: Vec<P>,
    plated: bool,
    z: (f32, f32),
}

impl Hole {
    fn opens(&self, z: f32) -> bool {
        z <= self.z.0 + 1e-4 && z >= self.z.1 - 1e-4
    }
}

fn via_z(v: &agentee_core::layout::Via, l: &Layout, board: &Board, z: (f32, f32)) -> (f32, f32) {
    let Some((a, b)) = v.span_of(&l.copper) else { return z };
    let depth = |k: usize| board.stackup.copper_z(&l.copper[k]).unwrap_or(0.0) as f32;
    let first = if a == 0 { z.0 } else { z.0 - depth(a) };
    let last = if b + 1 == l.copper.len() { z.1 } else { z.0 - depth(b) };
    (first, last)
}

fn backdrilled(
    v: &agentee_core::layout::Via,
    l: &Layout,
    board: &Board,
    z: (f32, f32),
) -> Option<(Hole, (f32, f32))> {
    let bd = v.backdrill.as_ref()?;
    let side = l.copper.iter().position(|c| *c == bd.from)?;
    let stop = l.copper.iter().position(|c| *c == bd.to)?;
    let barrel = via_z(v, l, board, z);
    let stop_z = z.0 - board.stackup.copper_z(&bd.to)? as f32;
    let stub = bd.max_stub.to_mm() as f32;
    let ring = disc(v.at, bd.diameter.to_mm() / 2.0, 16);
    if side < stop {
        let floor = (stop_z + stub).min(z.0);
        Some((Hole { ring, plated: false, z: (z.0, floor) }, (floor, barrel.1)))
    } else {
        let floor = (stop_z - stub).max(z.1);
        Some((Hole { ring, plated: false, z: (floor, z.1) }, (barrel.0, floor)))
    }
}

const DRILL_POINT: f64 = 0.6;

fn drill_point(
    v: &agentee_core::layout::Via,
    l: &Layout,
    board: &Board,
    z: (f32, f32),
) -> Option<(f32, f32)> {
    if v.drill_kind != agentee_core::board::DrillKind::ControlledDepth || v.backdrill.is_some() {
        return None;
    }
    let (a, _) = v.span_of(&l.copper)?;
    let barrel = via_z(v, l, board, z);
    let h = (v.drill / 2.0 * DRILL_POINT) as f32;
    Some(if a == 0 { (barrel.1, barrel.1 - h) } else { (barrel.0, barrel.0 + h) })
}

fn cone(ring: &[P], c: P, base: f32, apex: f32, s: &mut Surface) {
    let up = if base > apex { 1.0 } else { -1.0 };
    let tip = [c[0] as f32, -c[1] as f32, apex];
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let pa = [a[0] as f32, -a[1] as f32, base];
        let pb = [b[0] as f32, -b[1] as f32, base];
        let (u, w) = (
            [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]],
            [tip[0] - pa[0], tip[1] - pa[1], tip[2] - pa[2]],
        );
        let mut n =
            [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        let flip = if n[2] * up < 0.0 { -1.0 } else { 1.0 };
        n = n.map(|x| x * flip / len);
        for p in [pa, pb, tip] {
            s.positions.push(p);
            s.normals.push(n);
        }
    }
}

fn holes(l: &Layout, board: &Board, z: (f32, f32)) -> Vec<Hole> {
    let mut out: Vec<Hole> = Vec::new();
    for v in &l.vias {
        let ring = disc(v.at, v.drill / 2.0, 12);
        match backdrilled(v, l, board, z) {
            Some((hole, barrel)) => {
                out.push(hole);
                out.push(Hole { ring, plated: true, z: barrel });
            }
            None => out.push(Hole { ring, plated: true, z: via_z(v, l, board, z) }),
        }
    }
    for part in &l.parts {
        for pad in &part.pads {
            let Some((c, d, rot)) = pad.drill else { continue };
            let plated = pad.kind != PadKind::Npth;
            let (w, h) = (d[0], d[1]);
            let ring = if (w - h).abs() < 1e-6 {
                disc(c, w / 2.0, if w > 1.0 { 32 } else { 16 })
            } else {
                let (r, half) = (w.min(h) / 2.0, (w.max(h) - w.min(h)) / 2.0);
                let a = rot.to_radians() + if h > w { std::f64::consts::FRAC_PI_2 } else { 0.0 };
                let (dx, dy) = (half * a.cos(), -half * a.sin());
                capsule([c[0] - dx, c[1] - dy], [c[0] + dx, c[1] + dy], r)
            };
            out.push(Hole { ring, plated, z });
        }
    }
    out.extend(l.board_cutouts.iter().filter(|c| c.len() >= 3).map(|c| Hole {
        ring: c.clone(),
        plated: false,
        z,
    }));
    out
}

fn signed_area(r: &[P]) -> f64 {
    let mut a = 0.0;
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a / 2.0
}

fn walls(ring: &[P], z0: f32, z1: f32, outward: bool, s: &mut Surface) {
    let ccw = signed_area(ring) > 0.0;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            continue;
        }
        let mut n = [dy / len, -dx / len];
        if ccw != outward {
            n = [-n[0], -n[1]];
        }
        let nw = [n[0] as f32, -n[1] as f32, 0.0];
        let pa = |z: f32| [a[0] as f32, -a[1] as f32, z];
        let pb = |z: f32| [b[0] as f32, -b[1] as f32, z];
        for p in [pa(z1), pb(z1), pb(z0), pa(z1), pb(z0), pa(z0)] {
            s.positions.push(p);
            s.normals.push(nw);
        }
    }
}

fn faces(
    outline: &[P],
    holes: &[Hole],
    z: f32,
    up: bool,
    bounds: [f64; 4],
    texture: usize,
) -> Surface {
    let mut coords: Vec<f64> = outline.iter().flat_map(|p| [p[0], p[1]]).collect();
    let mut starts = Vec::new();
    for h in holes.iter().filter(|h| h.opens(z)) {
        starts.push(coords.len() / 2);
        coords.extend(h.ring.iter().flat_map(|p| [p[0], p[1]]));
    }
    let idx = earcutr::earcut(&coords, &starts, 2).unwrap_or_default();
    let mut s = Surface { texture: Some(texture), ..Default::default() };
    let [x0, y0, w, h] = bounds;
    for t in idx.chunks_exact(3) {
        let order = if up { [t[0], t[1], t[2]] } else { [t[0], t[2], t[1]] };
        let pts: Vec<P> = order.iter().map(|&i| [coords[2 * i], coords[2 * i + 1]]).collect();
        let area = (pts[1][0] - pts[0][0]) * (pts[2][1] - pts[0][1])
            - (pts[1][1] - pts[0][1]) * (pts[2][0] - pts[0][0]);
        let fix = (area > 0.0) == up;
        let pts = if fix { vec![pts[0], pts[2], pts[1]] } else { pts };
        for p in pts {
            s.positions.push([p[0] as f32, -p[1] as f32, z]);
            s.normals.push([0.0, 0.0, if up { 1.0 } else { -1.0 }]);
            s.uvs.push([((p[0] - x0) / w) as f32, ((p[1] - y0) / h) as f32]);
        }
    }
    s
}

fn decals(l: &Layout, board: &Board, side: usize) -> Decal {
    let mask = mask_rgb(&board.stackup.mask_color);
    let silk = silk_rgb(&board.stackup.silk_color);
    let gold = finish_rgb(&board.stackup.finish);
    let cu = if side == 0 { "F.Cu" } else { "B.Cu" };
    let mask_layer = if side == 0 { "F.Mask" } else { "B.Mask" };
    let silk_layer = if side == 0 { "F.SilkS" } else { "B.SilkS" };
    let under = lift(mask, 1.9);
    let mut d = Decal::new();
    for zf in l.zones.iter().filter(|zf| zf.layer == cu) {
        for tr in &zf.triangles {
            d.push((*tr, under));
        }
    }
    for tr in l.tracks.iter().filter(|tr| tr.layer == cu) {
        for w in tr.points.windows(2) {
            flat(&[capsule(w[0], w[1], tr.width / 2.0)], under, &mut d);
        }
    }
    for v in l.vias.iter().filter(|v| v.layers.iter().any(|c| c == cu)) {
        flat(&[disc(v.at, v.diameter / 2.0, 20)], under, &mut d);
    }
    for part in &l.parts {
        for pad in part.pads.iter().filter(|p| p.copper.iter().any(|c| c == cu)) {
            if !pad.mask.iter().any(|m| m == mask_layer) {
                flat(&pad.outlines, under, &mut d);
            }
        }
        let bare = graphic_areas(part, mask_layer);
        for area in graphic_areas(part, cu) {
            let open = bare.iter().any(|m| overlaps(&area, m));
            flat(&[area], if open { gold } else { under }, &mut d);
        }
    }
    let pen_for = |size: f64| agentee_core::font::default_thickness(size).max(0.15);
    let mut texts: Vec<_> = l.parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
    texts.extend(l.board_texts());
    for tx in texts.iter().filter(|tx| tx.layer == silk_layer) {
        let pen = pen_for(tx.size) / 2.0;
        for st in
            agentee_core::font::strokes(&tx.text, tx.at, tx.size, tx.rotation, tx.anchor, side == 1)
        {
            for w in st.windows(2) {
                flat(&[capsule(w[0], w[1], pen)], silk, &mut d);
            }
        }
    }
    for part in &l.parts {
        let tf = part.transform();
        for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == silk_layer)
        {
            if matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
                continue;
            }
            let path: Vec<P> =
                agentee_core::footprint::graphic_path(g).into_iter().map(|q| tf.apply(q)).collect();
            let pen = (g.width.to_mm() / 2.0).max(0.06);
            for w in path.windows(2) {
                flat(&[capsule(w[0], w[1], pen)], silk, &mut d);
            }
        }
    }
    for a in l.artwork.iter().filter(|a| a.layer == silk_layer) {
        flat(&a.polygons, silk, &mut d);
    }
    for part in &l.parts {
        for pad in part.pads.iter().filter(|p| p.copper.iter().any(|c| c == cu)) {
            if pad.mask.iter().any(|m| m == mask_layer) {
                flat(&pad.outlines, gold, &mut d);
            }
        }
    }
    d
}

fn rasterize(decal: &Decal, base: Color32, bounds: [f64; 4], ppm: f64) -> Image {
    let [x0, y0, w, h] = bounds;
    let (pw, ph) = ((w * ppm).ceil() as usize, (h * ppm).ceil() as usize);
    let mut mesh = egui::Mesh::default();
    for (t, c) in decal {
        let base_index = mesh.vertices.len() as u32;
        for p in t {
            mesh.colored_vertex(
                Pos2::new(((p[0] - x0) * ppm) as f32, ((p[1] - y0) * ppm) as f32),
                *c,
            );
        }
        mesh.add_triangle(base_index, base_index + 1, base_index + 2);
    }
    let mut canvas = Canvas::new(pw, ph);
    canvas.rgba.fill(base.to_array().map(|v| v as f32 / 255.0));
    let prim = ClippedPrimitive {
        clip_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(pw as f32, ph as f32)),
        primitive: Primitive::Mesh(mesh),
    };
    canvas.draw(&[prim], &Textures::default(), 1.0);
    Image { width: pw, height: ph, rgba: canvas.to_rgba8() }
}

fn is_metal(c: [f32; 3]) -> bool {
    let (mx, mn) = (c.iter().cloned().fold(0.0, f32::max), c.iter().cloned().fold(1.0, f32::min));
    let grey = mx > 0.6 && mx - mn < 0.12;
    let gold = c[0] > 0.7 && c[1] > 0.55 && c[2] < 0.6 && c[0] - c[2] > 0.2;
    grey || gold
}

fn place_model(
    part: &Placed,
    mesh: &agentee_3d::Mesh,
    model_frame: bool,
    top: f32,
    bot: f32,
    groups: &mut HashMap<[u16; 3], Surface>,
) {
    let fp = &part.footprint;
    let (r, s, o) = if model_frame {
        (rotation(fp.model_rotate), fp.model_scale, fp.model_offset)
    } else {
        (rotation([0.0; 3]), [1.0; 3], [0.0; 3])
    };
    let tf = part.transform();
    let rot = |v: [f64; 3]| [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2]);
    for p in &mesh.parts {
        let key = p.colour.map(|c| (c.clamp(0.0, 1.0) * 1000.0) as u16);
        let g = groups.entry(key).or_insert_with(|| Surface {
            colour: p.colour,
            material: if is_metal(p.colour) { Finish::Metal } else { Finish::Board },
            group: PARTS,
            ..Default::default()
        });
        for (v, n) in p.positions.iter().zip(&p.normals) {
            let q = rot([v[0] as f64 * s[0], v[1] as f64 * s[1], v[2] as f64 * s[2]]);
            let q = [q[0] + o[0], q[1] + o[1], q[2] + o[2]];
            let b = tf.apply([q[0], -q[1]]);
            let z = if part.bottom { bot - q[2] as f32 } else { top + q[2] as f32 };
            g.positions.push([b[0] as f32, -b[1] as f32, z]);
            let qn = rot([n[0] as f64, n[1] as f64, n[2] as f64]);
            let d = tf.direction([qn[0], -qn[1]]);
            let nz = if part.bottom { -qn[2] } else { qn[2] };
            g.normals.push(norm([d[0] as f32, -d[1] as f32, nz as f32]));
        }
    }
}

fn place_box(part: &Placed, top: f32, bot: f32, out: &mut Vec<Surface>) {
    let (guess, color) = body_height(&part.footprint_name);
    let h = part.footprint.height.map(|v| v as f32).unwrap_or(guess);
    if h <= 0.0 {
        return;
    }
    let fab = format!("{}.Fab", if part.bottom { "B" } else { "F" });
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == fab) {
        if !matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
            b.union(&g.bounds());
        }
    }
    if b.is_empty() {
        return;
    }
    let tf = part.transform();
    let ring: Vec<P> = [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
        .into_iter()
        .map(|q| tf.apply(q))
        .collect();
    let (z0, z1) = if part.bottom { (bot - h, bot) } else { (top, top + h) };
    let mut s = Surface { colour: rgb(color), group: PARTS, ..Default::default() };
    walls(&ring, z0, z1, true, &mut s);
    let zc = if part.bottom { z0 } else { z1 };
    let nz = if part.bottom { -1.0 } else { 1.0 };
    let c: Vec<V3> = ring.iter().map(|p| [p[0] as f32, -p[1] as f32, zc]).collect();
    let up = cross(sub(c[1], c[0]), sub(c[2], c[0]))[2] * nz > 0.0;
    let tris = if up { [[0, 1, 2], [0, 2, 3]] } else { [[0, 2, 1], [0, 3, 2]] };
    for t in tris {
        for k in t {
            s.positions.push(c[k]);
            s.normals.push([0.0, 0.0, nz]);
        }
    }
    out.push(s);
}

pub fn build(l: &Layout, board: &Board, root: &Path, fetch: Fetch) -> Scene {
    let mut s = viewer3d::Scene::<Finish>::default();
    let (mut pending, mut missing) = (0, Vec::new());
    let t = board.stackup.thickness().to_mm().max(0.4) as f32;
    let (top, bot) = (t / 2.0, -t / 2.0);
    let mask = mask_rgb(&board.stackup.mask_color);
    let gold = finish_rgb(&board.stackup.finish);
    let fr4 = Color32::from_rgb(170, 160, 120);
    let outline: Vec<P> = l.outline.clone();
    let mut bb = agentee_core::graphic::Bounds::EMPTY;
    outline.iter().for_each(|p| bb.add(*p));
    if bb.is_empty() {
        return Scene { mesh: Arc::new(s), pending, missing };
    }
    let size = bb.size();
    let bounds = [bb.min[0], bb.min[1], size[0].max(1e-3), size[1].max(1e-3)];
    let ppm = (40.0f64).min(4096.0 / size[0].max(size[1]));
    let hs = holes(l, board, (top, bot));
    for side in 0..2 {
        s.images.push(rasterize(&decals(l, board, side), mask, bounds, ppm));
    }
    s.surfaces.push(faces(&outline, &hs, top, true, bounds, 0));
    s.surfaces.push(faces(&outline, &hs, bot, false, bounds, 1));
    let mut edge = Surface { colour: rgb(fr4), ..Default::default() };
    walls(&outline, bot, top, true, &mut edge);
    let mut plated = Surface { colour: rgb(gold), material: Finish::Metal, ..Default::default() };
    for h in &hs {
        walls(&h.ring, h.z.1, h.z.0, false, if h.plated { &mut plated } else { &mut edge });
    }
    for v in &l.vias {
        if let Some((base, apex)) = drill_point(v, l, board, (top, bot)) {
            cone(&disc(v.at, v.drill / 2.0, 12), v.at, base, apex, &mut plated);
        }
    }
    s.surfaces.push(edge);
    s.surfaces.push(plated);
    let mut groups: HashMap<[u16; 3], Surface> = HashMap::new();
    let mut boxes = Vec::new();
    let mut generated: HashMap<String, Option<agentee_3d::Mesh>> = HashMap::new();
    for part in &l.parts {
        let fp = &part.footprint;
        let own = fp.model.as_deref().filter(|m| agentee_3d::in_project(m, root).is_some());
        if own.is_none() {
            let mesh = generated
                .entry(fp.name.clone())
                .or_insert_with(|| agentee_3d::parametric::generate(fp));
            if let Some(mesh) = mesh {
                place_model(part, mesh, false, top, bot, &mut groups);
                continue;
            }
        }
        let status = match &fp.model {
            Some(m) => agentee_3d::get(m, root, fetch),
            None => Status::Missing(String::new()),
        };
        match status {
            Status::Ready(mesh) => place_model(part, &mesh, true, top, bot, &mut groups),
            Status::Pending => {
                pending += 1;
                place_box(part, top, bot, &mut boxes);
            }
            Status::Missing(e) => {
                if !e.is_empty() && !missing.contains(&e) {
                    missing.push(e);
                }
                place_box(part, top, bot, &mut boxes);
            }
        }
    }
    let mut keys: Vec<_> = groups.keys().copied().collect();
    keys.sort();
    for k in keys {
        s.surfaces.push(groups.remove(&k).unwrap());
    }
    s.surfaces.extend(boxes);
    let c = bb.center();
    s.centre = [c[0] as f32, -c[1] as f32, 0.0];
    s.radius = (size[0].max(size[1]) as f32).max(1.0) * 0.69;
    Scene { mesh: Arc::new(s), pending, missing }
}

pub fn show(ui: &mut Ui, scene: &Arc<Scene>, state: &mut State, interactive: bool, parts: bool) {
    let size = ui.available_size();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let camera = match state.region {
        Some((centre, span)) => state.camera.framed(&scene.mesh, centre, span),
        None => state.camera,
    };
    let looks = [Look::default(), if parts { Look::default() } else { Look::HIDDEN }];
    let mut viewer = Viewer::new(scene.mesh.clone(), camera).looks(looks).style(STYLE);
    if interactive {
        if viewer.navigate(ui, &resp, &mut state.orbit) {
            state.camera = viewer.camera();
            state.region = None;
        }
        if resp.double_clicked() {
            state.camera = home();
            state.region = None;
        }
        viewer.paint(ui, rect);
    } else {
        let ppp = ui.ctx().pixels_per_point();
        let (pw, ph) =
            ((rect.width() * ppp).round() as usize, (rect.height() * ppp).round() as usize);
        let key = (scene.mesh.id, camera, [pw as u32, ph as u32], parts);
        if state.soft.as_ref().map(|s| s.0) != Some(key) && pw > 0 && ph > 0 {
            let img = viewer.render_soft(pw, ph);
            let tex = ui.ctx().load_texture("board3d", img, egui::TextureOptions::LINEAR);
            state.soft = Some((key, tex));
        }
        if let Some((_, tex)) = &state.soft {
            ui.painter_at(rect).image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
    let p = ui.painter_at(rect);
    let mut notes = Vec::new();
    if scene.pending > 0 {
        notes.push(format!("fetching {} 3D models", scene.pending));
    }
    if !scene.missing.is_empty() {
        notes.push(format!("{} models not found", scene.missing.len()));
    }
    if interactive {
        notes
            .push("drag to orbit, shift-drag to pan, scroll to zoom, double-click to reset".into());
    }
    if !notes.is_empty() {
        p.text(
            rect.left_bottom() + Vec2::new(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            notes.join("   "),
            egui::FontId::proportional(11.0),
            Color32::from_gray(140),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_model_rewritten_on_disk_shows_in_the_next_scene() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["symbols", "footprints", "3dmodels"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let lna = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
        std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
        let fp = std::fs::read_to_string(lna.join("footprints/R_0402_1005Metric.fp.toml")).unwrap();
        let fp: String = fp
            .lines()
            .map(|l| if l.starts_with("model") { "model = \"3dmodels/r.wrl\"" } else { l })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join("footprints/R_0402_1005Metric.fp.toml"), fp).unwrap();
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\n[outline]\nsize = [20, 10]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.sch.toml"),
            "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\nfootprint = \"R_0402_1005Metric\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\nat = [10, 5]\n",
        )
        .unwrap();
        let quads = |n: usize| {
            let mut s = String::from("#VRML V2.0 utf8\n");
            for i in 0..n {
                s += &format!(
                    "Shape {{ geometry IndexedFaceSet {{ coord Coordinate {{ point [ 0 0 {i}, 1 0 {i}, 1 1 {i}, 0 1 {i} ] }} coordIndex [ 0 1 2 3 -1 ] }} }}\n"
                );
            }
            s
        };
        let model = dir.join("3dmodels/r.wrl");
        let p = agentee_core::Project::load(&dir).unwrap();
        let part_triangles = || {
            let scene = build(&p.layouts[0].item, &p.boards[0].item, &dir, Fetch::Never);
            scene
                .mesh
                .surfaces
                .iter()
                .filter(|s| s.group == PARTS)
                .map(|s| s.positions.len() / 3)
                .sum::<usize>()
        };
        std::fs::write(&model, quads(1)).unwrap();
        let first = part_triangles();
        std::fs::write(&model, quads(4)).unwrap();
        let generation = agentee_3d::generation();
        assert!(agentee_3d::is_model(&model));
        agentee_3d::models_changed();
        assert!(agentee_3d::generation() > generation, "the scene key would not change");
        let second = part_triangles();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(first > 0 && second > first, "{first} then {second} part triangles");
    }

    use super::*;

    #[test]
    fn a_board_cutout_is_a_hole_through_the_3d_board() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-cutout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\n[outline]\nsize = [20, 10]\n[[outline.cutouts]]\norigin = [8, 3]\nsize = [4, 4]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("t.sch.toml"), "name = \"t\"\nboard = \"t\"\n").unwrap();
        std::fs::write(dir.join("t.pcb.toml"), "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n")
            .unwrap();
        let p = agentee_core::Project::load(&dir).unwrap();
        let scene = build(&p.layouts[0].item, &p.boards[0].item, &dir, Fetch::Blocking);
        let top = &scene.mesh.surfaces[0];
        let mut area = 0.0;
        for t in top.positions.chunks_exact(3) {
            let c = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, -(t[0][1] + t[1][1] + t[2][1]) / 3.0];
            assert!(
                !(c[0] > 8.0 && c[0] < 12.0 && c[1] > 3.0 && c[1] < 7.0),
                "face over the cutout"
            );
            let (u, v) =
                ([t[1][0] - t[0][0], t[1][1] - t[0][1]], [t[2][0] - t[0][0], t[2][1] - t[0][1]]);
            area += ((u[0] * v[1] - u[1] * v[0]) / 2.0).abs() as f64;
        }
        assert!((area - (200.0 - 16.0)).abs() < 1e-3, "top face area {area}");
        let walls = &scene.mesh.surfaces[2];
        assert_eq!(walls.positions.len(), 6 * 8);
    }

    #[test]
    fn a_blind_via_opens_one_face_and_its_barrel_stops_at_its_span() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-vias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\nfab = \"hdi\"\n[outline]\nsize = [20, 10]\n[stackup]\npreset = \"hdi-6l-1n1\"\n[[vias]]\nname = \"uv\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.sch.toml"),
            "name = \"t\"\nboard = \"t\"\n[[nets]]\nname = \"A\"\npins = []\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[vias]]\nnet = \"A\"\nat = [5, 5]\nvia = \"uv\"\n",
        )
        .unwrap();
        let p = agentee_core::Project::load(&dir).unwrap();
        let (l, board) = (&p.layouts[0].item, &p.boards[0].item);
        let scene = build(l, board, &dir, Fetch::Blocking);
        let area = |s: &Surface| {
            s.positions
                .chunks_exact(3)
                .map(|t| {
                    let (u, v) = (
                        [t[1][0] - t[0][0], t[1][1] - t[0][1]],
                        [t[2][0] - t[0][0], t[2][1] - t[0][1]],
                    );
                    ((u[0] * v[1] - u[1] * v[0]) / 2.0).abs() as f64
                })
                .sum::<f64>()
        };
        assert!(area(&scene.mesh.surfaces[0]) < 200.0 - 1e-4);
        assert!((area(&scene.mesh.surfaces[1]) - 200.0).abs() < 1e-4);
        let plated = &scene.mesh.surfaces[3];
        let t = board.stackup.thickness().to_mm() as f32;
        let low = plated.positions.iter().map(|q| q[2]).fold(f32::MAX, f32::min);
        let depth = board.stackup.copper_z("In1.Cu").unwrap() as f32;
        assert!((low - (t / 2.0 - depth)).abs() < 1e-4, "{low}");
    }

    #[test]
    fn a_backdrill_opens_the_drilled_face_wider_and_leaves_its_stub_plated() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-backdrill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\nfab = \"hdi\"\n[outline]\nsize = [20, 10]\n[stackup]\npreset = \"hdi-6l-1n1\"\n[[vias]]\nname = \"bd\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\nbackdrill = { from = \"B.Cu\", to = \"In2.Cu\", max_stub = \"0.1mm\", diameter = \"0.5mm\" }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.sch.toml"),
            "name = \"t\"\nboard = \"t\"\n[[nets]]\nname = \"A\"\npins = []\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[vias]]\nnet = \"A\"\nat = [5, 5]\nvia = \"bd\"\n",
        )
        .unwrap();
        let p = agentee_core::Project::load(&dir).unwrap();
        let (l, board) = (&p.layouts[0].item, &p.boards[0].item);
        let scene = build(l, board, &dir, Fetch::Blocking);
        let area = |s: &Surface| {
            s.positions
                .chunks_exact(3)
                .map(|t| {
                    let (u, v) = (
                        [t[1][0] - t[0][0], t[1][1] - t[0][1]],
                        [t[2][0] - t[0][0], t[2][1] - t[0][1]],
                    );
                    ((u[0] * v[1] - u[1] * v[0]) / 2.0).abs() as f64
                })
                .sum::<f64>()
        };
        let ring_area =
            |r: f64, n: usize| 0.5 * n as f64 * r * r * (std::f64::consts::TAU / n as f64).sin();
        assert!((area(&scene.mesh.surfaces[0]) - (200.0 - ring_area(0.15, 12))).abs() < 1e-3);
        assert!((area(&scene.mesh.surfaces[1]) - (200.0 - ring_area(0.25, 16))).abs() < 1e-3);
        let t = board.stackup.thickness().to_mm() as f32;
        let floor = t / 2.0 - board.stackup.copper_z("In2.Cu").unwrap() as f32 - 0.1;
        let plated = &scene.mesh.surfaces[3];
        let low = plated.positions.iter().map(|q| q[2]).fold(f32::MAX, f32::min);
        assert!((low - floor).abs() < 1e-4, "{low} {floor}");
        let wide: Vec<f32> = scene.mesh.surfaces[2]
            .positions
            .iter()
            .filter(|q| ((q[0] - 5.0).powi(2) + (q[1] + 5.0).powi(2)).sqrt() < 0.3)
            .map(|q| q[2])
            .collect();
        let top = wide.iter().cloned().fold(f32::MIN, f32::max);
        assert!(!wide.is_empty() && (top - floor).abs() < 1e-4, "{top} {floor}");
    }

    #[test]
    fn a_controlled_depth_via_ends_in_a_drill_point() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let board = "name = \"t\"\nfab = \"hdi\"\n[outline]\nsize = [20, 10]\n[stackup]\npreset = \"hdi-6l-1n1\"\n[[vias]]\nname = \"cd\"\ndrill = \"0.15mm\"\ndiameter = \"0.45mm\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\ndrill_kind = \"controlled_depth\"\n";
        std::fs::write(
            dir.join("t.sch.toml"),
            "name = \"t\"\nboard = \"t\"\n[[nets]]\nname = \"A\"\npins = []\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[vias]]\nnet = \"A\"\nat = [5, 5]\nvia = \"cd\"\n",
        )
        .unwrap();
        let lowest = |board: &str| {
            std::fs::write(dir.join("t.board.toml"), board).unwrap();
            let p = agentee_core::Project::load(&dir).unwrap();
            let (l, b) = (&p.layouts[0].item, &p.boards[0].item);
            let scene = build(l, b, &dir, Fetch::Blocking);
            let t = b.stackup.thickness().to_mm() as f32;
            let stop = t / 2.0 - b.stackup.copper_z("In1.Cu").unwrap() as f32;
            let low =
                scene.mesh.surfaces[3].positions.iter().map(|q| q[2]).fold(f32::MAX, f32::min);
            low - stop
        };
        let point = lowest(board);
        assert!((point + 0.075 * 0.6).abs() < 1e-4, "{point}");
        let flat = lowest(&board.replace("drill_kind = \"controlled_depth\"\n", ""));
        assert!(flat.abs() < 1e-4, "{flat}");
    }

    #[test]
    fn model_rotation_follows_kicad_negated_zyx() {
        let r = rotation([0.0, 0.0, 90.0]);
        let v = [0, 1, 2].map(|i| r[i][0]);
        assert!((v[0]).abs() < 1e-12 && (v[1] + 1.0).abs() < 1e-12 && v[2].abs() < 1e-12);
        let r = rotation([90.0, 0.0, 0.0]);
        let v = [0, 1, 2].map(|i| r[i][1]);
        assert!(v[0].abs() < 1e-12 && v[1].abs() < 1e-12 && (v[2] + 1.0).abs() < 1e-12);
    }
}

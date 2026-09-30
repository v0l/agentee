use crate::raster::{Canvas, Textures};
use agentee_3d::{Fetch, Status};
use agentee_core::board::Board;
use agentee_core::footprint::PadKind;
use agentee_core::geom::P;
use agentee_core::layout::{Layout, Placed};
use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub type V3 = [f32; 3];

#[derive(Clone, Default)]
pub struct Surface {
    pub positions: Vec<V3>,
    pub normals: Vec<V3>,
    pub uvs: Vec<[f32; 2]>,
    pub colour: [f32; 3],
    pub texture: Option<usize>,
    pub metal: f32,
    pub part: bool,
}

pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct Scene {
    pub id: u64,
    pub surfaces: Vec<Surface>,
    pub images: Vec<Image>,
    pub centre: V3,
    pub radius: f32,
    pub pending: usize,
    pub missing: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: Vec2,
    pub focus: Option<(V3, f32)>,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 0.35, pitch: 0.85, zoom: 1.0, pan: Vec2::ZERO, focus: None }
    }
}

pub const AMBIENT: f32 = 0.30;
pub const KEY: f32 = 0.80;
pub const FILL: f32 = 0.25;
pub const BACKGROUND: Color32 = Color32::from_rgb(20, 22, 26);

pub struct View {
    pub eye: V3,
    pub target: V3,
    pub right: V3,
    pub up: V3,
    pub forward: V3,
    pub focal: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub key: V3,
    pub fill: V3,
}

pub fn view(scene: &Scene, cam: &Camera, size: Vec2) -> View {
    let (cy, sy, cp, sp) = (cam.yaw.cos(), cam.yaw.sin(), cam.pitch.cos(), cam.pitch.sin());
    let eye_dir = [sy * cp, -cy * cp, sp];
    let dist = scene.radius * 2.4;
    let (centre, span) = cam.focus.unwrap_or((scene.centre, 0.0));
    let zoom = if span > 0.0 { cam.zoom * dist / (1.25 * span * 1.15) } else { cam.zoom };
    let forward = norm([-eye_dir[0], -eye_dir[1], -eye_dir[2]]);
    let right = norm(cross(forward, [0.0, 0.0, 1.0]));
    let right = if dot(right, right) < 0.5 { [1.0, 0.0, 0.0] } else { right };
    let up = cross(right, forward);
    let focal = size.x.min(size.y) * 1.25 * zoom;
    let shift = add(scale(right, -cam.pan.x * dist / focal), scale(up, cam.pan.y * dist / focal));
    let target = add(centre, shift);
    let eye = add(target, scale(eye_dir, dist));
    let back = scale(forward, -1.0);
    let key = norm(add(add(scale(right, -0.35), scale(up, 0.55)), scale(back, 0.75)));
    let fill = norm(add(add(scale(right, 0.55), scale(up, -0.25)), scale(back, 0.8)));
    View {
        eye,
        target,
        right,
        up,
        forward,
        focal,
        fov_y: 2.0 * (size.y * 0.5 / focal).atan(),
        near: dist * 0.02,
        far: dist * 10.0,
        key,
        fill,
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

fn rotation(deg: [f64; 3]) -> [[f64; 3]; 3] {
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
            metal: if is_metal(p.colour) { 0.8 } else { 0.0 },
            part: true,
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
    let mut s = Surface { colour: rgb(color), part: true, ..Default::default() };
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

static SCENE_ID: AtomicU64 = AtomicU64::new(1);

pub fn build(l: &Layout, board: &Board, root: &Path, fetch: Fetch) -> Scene {
    let mut s = Scene { id: SCENE_ID.fetch_add(1, Ordering::Relaxed), ..Default::default() };
    let t = board.stackup.thickness().to_mm().max(0.4) as f32;
    let (top, bot) = (t / 2.0, -t / 2.0);
    let mask = mask_rgb(&board.stackup.mask_color);
    let gold = finish_rgb(&board.stackup.finish);
    let fr4 = Color32::from_rgb(170, 160, 120);
    let outline: Vec<P> = l.outline.clone();
    let mut bb = agentee_core::graphic::Bounds::EMPTY;
    outline.iter().for_each(|p| bb.add(*p));
    if bb.is_empty() {
        return s;
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
    let mut plated = Surface { colour: rgb(gold), metal: 0.8, ..Default::default() };
    for h in &hs {
        walls(&h.ring, h.z.1, h.z.0, false, if h.plated { &mut plated } else { &mut edge });
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
                s.pending += 1;
                place_box(part, top, bot, &mut boxes);
            }
            Status::Missing(e) => {
                if !e.is_empty() && !s.missing.contains(&e) {
                    s.missing.push(e);
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
    s.radius = (size[0].max(size[1]) as f32).max(1.0) * 0.75;
    s
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn shade(albedo: V3, n: V3, v: V3, metal: f32, vw: &View) -> V3 {
    let albedo = albedo.map(to_linear);
    let n = if dot(n, v) < 0.0 { scale(n, -1.0) } else { n };
    let d = AMBIENT + KEY * dot(n, vw.key).max(0.0) + FILL * dot(n, vw.fill).max(0.0);
    let h = norm(add(vw.key, v));
    let ks = 0.06 + 0.5 * metal;
    let sh = 24.0 + 72.0 * metal;
    let sp = ks * dot(n, h).max(0.0).powf(sh);
    [0, 1, 2].map(|k| {
        let tint = 1.0 + (albedo[k] * 1.6 - 1.0) * metal;
        to_srgb(albedo[k] * d + sp * tint)
    })
}

fn sample(img: &Image, u: f32, v: f32) -> V3 {
    let (w, h) = (img.width, img.height);
    let x = (u * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
    let y = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize, k: usize| img.rgba[(y * w + x) * 4 + k] as f32 / 255.0;
    [0, 1, 2].map(|k| {
        let t = px(x0, y0, k) + (px(x1, y0, k) - px(x0, y0, k)) * fx;
        let b = px(x0, y1, k) + (px(x1, y1, k) - px(x0, y1, k)) * fx;
        t + (b - t) * fy
    })
}

pub fn render_soft(
    scene: &Scene,
    cam: &Camera,
    width: usize,
    height: usize,
    parts: bool,
) -> ColorImage {
    const SS: usize = 2;
    let (w, h) = (width * SS, height * SS);
    let vw = view(scene, cam, Vec2::new(width as f32, height as f32));
    let focal = vw.focal * SS as f32;
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let mut depth = vec![f32::INFINITY; w * h];
    let mut color = vec![rgb(BACKGROUND); w * h];
    for surf in scene.surfaces.iter().filter(|s| parts || !s.part) {
        let tex = surf.texture.and_then(|i| scene.images.get(i));
        for t in (0..surf.positions.len() / 3).map(|i| i * 3) {
            let sp = [0, 1, 2].map(|k| {
                let d = sub(surf.positions[t + k], vw.eye);
                let z = dot(d, vw.forward);
                [cx + dot(d, vw.right) / z * focal, cy - dot(d, vw.up) / z * focal, z]
            });
            if sp.iter().any(|p| p[2] < vw.near) {
                continue;
            }
            let area = (sp[1][0] - sp[0][0]) * (sp[2][1] - sp[0][1])
                - (sp[1][1] - sp[0][1]) * (sp[2][0] - sp[0][0]);
            if area.abs() < 1e-9 {
                continue;
            }
            let x0 = sp.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let y0 = sp.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
            let x1 = (sp.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil() as i64).min(w as i64);
            let y1 = (sp.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil() as i64).min(h as i64);
            if x1 <= x0 as i64 || y1 <= y0 as i64 {
                continue;
            }
            let inv_z = sp.map(|p| 1.0 / p[2]);
            for y in y0..y1 as usize {
                let py = y as f32 + 0.5;
                for x in x0..x1 as usize {
                    let px = x as f32 + 0.5;
                    let e = |a: [f32; 3], b: [f32; 3]| {
                        (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
                    };
                    let (w0, w1, w2) =
                        (e(sp[1], sp[2]) / area, e(sp[2], sp[0]) / area, e(sp[0], sp[1]) / area);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let iz = w0 * inv_z[0] + w1 * inv_z[1] + w2 * inv_z[2];
                    let z = 1.0 / iz;
                    let i = y * w + x;
                    if z >= depth[i] {
                        continue;
                    }
                    depth[i] = z;
                    let pw = [w0 * inv_z[0] * z, w1 * inv_z[1] * z, w2 * inv_z[2] * z];
                    let lerp3 = |a: &[V3]| {
                        [0, 1, 2]
                            .map(|k| pw[0] * a[t][k] + pw[1] * a[t + 1][k] + pw[2] * a[t + 2][k])
                    };
                    let pos = lerp3(&surf.positions);
                    let n = norm(lerp3(&surf.normals));
                    let albedo = match tex {
                        Some(img) if surf.uvs.len() > t + 2 => {
                            let uv = &surf.uvs;
                            let u = pw[0] * uv[t][0] + pw[1] * uv[t + 1][0] + pw[2] * uv[t + 2][0];
                            let v = pw[0] * uv[t][1] + pw[1] * uv[t + 1][1] + pw[2] * uv[t + 2][1];
                            sample(img, u, v)
                        }
                        _ => surf.colour,
                    };
                    color[i] = shade(albedo, n, norm(sub(vw.eye, pos)), surf.metal, &vw);
                }
            }
        }
    }
    let mut out = ColorImage::new([width, height], vec![Color32::BLACK; width * height]);
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0.0f32; 3];
            for dy in 0..SS {
                for dx in 0..SS {
                    let c = color[(y * SS + dy) * w + x * SS + dx];
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = (SS * SS) as f32;
            out.pixels[y * width + x] = Color32::from_rgb(
                (acc[0] / n * 255.0 + 0.5) as u8,
                (acc[1] / n * 255.0 + 0.5) as u8,
                (acc[2] / n * 255.0 + 0.5) as u8,
            );
        }
    }
    out
}

pub type SoftCache = Option<((u64, [u32; 4], [u32; 2], bool), TextureHandle)>;

pub fn show(
    ui: &mut Ui,
    scene: &Arc<Scene>,
    cam: &mut Camera,
    interactive: bool,
    parts: bool,
    soft: &mut SoftCache,
) {
    let size = ui.available_size();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if interactive {
        if resp.dragged_by(egui::PointerButton::Primary) && !ui.input(|i| i.modifiers.shift) {
            let d = resp.drag_delta();
            cam.yaw -= d.x * 0.01;
            cam.pitch = (cam.pitch + d.y * 0.01).clamp(-1.55, 1.55);
        } else if resp.dragged() {
            cam.pan += resp.drag_delta();
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            cam.zoom = (cam.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 20.0);
        }
        if resp.double_clicked() {
            *cam = Camera::default();
        }
        crate::gl3d::paint(ui, rect, scene.clone(), *cam, parts);
    } else {
        let ppp = ui.ctx().pixels_per_point();
        let (pw, ph) =
            ((rect.width() * ppp).round() as usize, (rect.height() * ppp).round() as usize);
        let key = (
            scene.id,
            [
                cam.yaw.to_bits(),
                cam.pitch.to_bits(),
                cam.zoom.to_bits(),
                cam.pan.x.to_bits() ^ cam.pan.y.to_bits() ^ cam.focus.map_or(0, |f| f.1.to_bits()),
            ],
            [pw as u32, ph as u32],
            parts,
        );
        if soft.as_ref().map(|s| s.0) != Some(key) && pw > 0 && ph > 0 {
            let img = render_soft(scene, cam, pw, ph, parts);
            let tex = ui.ctx().load_texture("board3d", img, egui::TextureOptions::LINEAR);
            *soft = Some((key, tex));
        }
        if let Some((_, tex)) = soft {
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
        let top = &scene.surfaces[0];
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
        let walls = &scene.surfaces[2];
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
        assert!(area(&scene.surfaces[0]) < 200.0 - 1e-4);
        assert!((area(&scene.surfaces[1]) - 200.0).abs() < 1e-4);
        let plated = &scene.surfaces[3];
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
        assert!((area(&scene.surfaces[0]) - (200.0 - ring_area(0.15, 12))).abs() < 1e-3);
        assert!((area(&scene.surfaces[1]) - (200.0 - ring_area(0.25, 16))).abs() < 1e-3);
        let t = board.stackup.thickness().to_mm() as f32;
        let floor = t / 2.0 - board.stackup.copper_z("In2.Cu").unwrap() as f32 - 0.1;
        let plated = &scene.surfaces[3];
        let low = plated.positions.iter().map(|q| q[2]).fold(f32::MAX, f32::min);
        assert!((low - floor).abs() < 1e-4, "{low} {floor}");
        let wide: Vec<f32> = scene.surfaces[2]
            .positions
            .iter()
            .filter(|q| ((q[0] - 5.0).powi(2) + (q[1] + 5.0).powi(2)).sqrt() < 0.3)
            .map(|q| q[2])
            .collect();
        let top = wide.iter().cloned().fold(f32::MIN, f32::max);
        assert!(!wide.is_empty() && (top - floor).abs() < 1e-4, "{top} {floor}");
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

use agentee_core::board::Board;
use agentee_core::geom::P;
use agentee_core::layout::Layout;
use egui::{Color32, Mesh, Pos2, Rect, Sense, Ui, Vec2};

type V3 = [f32; 3];

#[derive(Clone, Copy)]
struct Tri {
    p: [V3; 3],
    color: Color32,
}

#[derive(Default)]
pub struct Scene {
    walls: Vec<Tri>,
    faces: [Vec<Tri>; 2],
    decals: [Vec<Tri>; 2],
    bodies: Vec<Tri>,
    centre: V3,
    radius: f32,
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 0.35, pitch: 0.85, zoom: 1.0, pan: Vec2::ZERO }
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

fn flat(polys: &[Vec<P>], z: f32, color: Color32, out: &mut Vec<Tri>) {
    for t in agentee_core::contour::triangles(polys) {
        out.push(Tri {
            p: [
                [t[0][0] as f32, t[0][1] as f32, z],
                [t[1][0] as f32, t[1][1] as f32, z],
                [t[2][0] as f32, t[2][1] as f32, z],
            ],
            color,
        });
    }
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

fn disc(c: P, r: f64) -> Vec<P> {
    let n = 20;
    (0..n)
        .map(|k| {
            let t = std::f64::consts::TAU * k as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

fn quad(a: V3, b: V3, c: V3, d: V3, color: Color32, out: &mut Vec<Tri>) {
    out.push(Tri { p: [a, b, c], color });
    out.push(Tri { p: [a, c, d], color });
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

pub fn build(l: &Layout, board: &Board) -> Scene {
    let mut s = Scene::default();
    let t = board.stackup.thickness().to_mm().max(0.4) as f32;
    let (top, bot) = (t / 2.0, -t / 2.0);
    let mask = mask_rgb(&board.stackup.mask_color);
    let silk = silk_rgb(&board.stackup.silk_color);
    let gold = finish_rgb(&board.stackup.finish);
    let fr4 = Color32::from_rgb(170, 160, 120);
    let outline: Vec<P> = l.outline.clone();
    let mut holes: Vec<(P, f64)> = l.vias.iter().map(|v| (v.at, v.drill / 2.0)).collect();
    for part in &l.parts {
        for pad in &part.pads {
            if let Some((c, d, _)) = pad.drill {
                holes.push((c, d[0].min(d[1]) / 2.0));
            }
        }
    }
    for (a, b) in outline.iter().zip(outline.iter().cycle().skip(1)) {
        let (a3, b3) = ([a[0] as f32, a[1] as f32], [b[0] as f32, b[1] as f32]);
        quad(
            [a3[0], a3[1], top],
            [b3[0], b3[1], top],
            [b3[0], b3[1], bot],
            [a3[0], a3[1], bot],
            fr4,
            &mut s.walls,
        );
    }
    for (side, z) in [(0usize, top), (1usize, bot)] {
        flat(std::slice::from_ref(&outline), z, mask, &mut s.faces[side]);
        let cu = if side == 0 { "F.Cu" } else { "B.Cu" };
        let mask_layer = if side == 0 { "F.Mask" } else { "B.Mask" };
        let silk_layer = if side == 0 { "F.SilkS" } else { "B.SilkS" };
        let under = lift(mask, 1.9).gamma_multiply(1.0);
        let d = &mut s.decals[side];
        for zf in l.zones.iter().filter(|zf| zf.layer == cu) {
            for tr in &zf.triangles {
                d.push(Tri { p: tr.map(|q| [q[0] as f32, q[1] as f32, z]), color: under });
            }
        }
        for tr in l.tracks.iter().filter(|tr| tr.layer == cu) {
            for w in tr.points.windows(2) {
                flat(&[capsule(w[0], w[1], tr.width / 2.0)], z, under, d);
            }
        }
        for v in &l.vias {
            flat(&[disc(v.at, v.diameter / 2.0)], z, under, d);
        }
        for part in &l.parts {
            for pad in &part.pads {
                if pad.copper.iter().any(|c| c == cu) {
                    let bare = pad.mask.iter().any(|m| m == mask_layer);
                    flat(&pad.outlines, z, if bare { gold } else { under }, d);
                }
            }
        }
        let pen_for = |size: f64| agentee_core::font::default_thickness(size).max(0.15);
        let mut texts: Vec<_> =
            l.parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
        texts.extend(l.board_texts());
        for tx in texts.iter().filter(|tx| tx.layer == silk_layer) {
            let pen = pen_for(tx.size) / 2.0;
            for st in agentee_core::font::strokes(
                &tx.text,
                tx.at,
                tx.size,
                tx.rotation,
                tx.anchor,
                side == 1,
            ) {
                for w in st.windows(2) {
                    flat(&[capsule(w[0], w[1], pen)], z, silk, d);
                }
            }
        }
        for part in &l.parts {
            let tf = part.transform();
            for g in
                part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == silk_layer)
            {
                if matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
                    continue;
                }
                let path: Vec<P> = agentee_core::footprint::graphic_path(g)
                    .into_iter()
                    .map(|q| tf.apply(q))
                    .collect();
                let pen = (g.width.to_mm() / 2.0).max(0.06);
                for w in path.windows(2) {
                    flat(&[capsule(w[0], w[1], pen)], z, silk, d);
                }
            }
        }
        for a in l.artwork.iter().filter(|a| a.layer == silk_layer) {
            flat(&a.polygons, z, silk, d);
        }
        for (c, r) in &holes {
            flat(&[disc(*c, *r)], z, Color32::from_rgb(8, 8, 8), d);
        }
    }
    for part in &l.parts {
        let (guess, color) = body_height(&part.footprint_name);
        let h = part.footprint.height.map(|v| v as f32).unwrap_or(guess);
        if h <= 0.0 {
            continue;
        }
        let side = if part.bottom { "B" } else { "F" };
        let fab = format!("{side}.Fab");
        let mut b = agentee_core::graphic::Bounds::EMPTY;
        for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == fab) {
            if !matches!(g.shape, agentee_core::graphic::Shape::Text { .. }) {
                b.union(&g.bounds());
            }
        }
        if b.is_empty() {
            continue;
        }
        let tf = part.transform();
        let base = if part.bottom { bot } else { top };
        let dir = if part.bottom { -1.0 } else { 1.0 };
        let corners: Vec<[f32; 2]> = [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
            .into_iter()
            .map(|q| {
                let w = tf.apply(q);
                [w[0] as f32, w[1] as f32]
            })
            .collect();
        let z0 = base + dir * 0.02;
        let z1 = base + dir * h;
        for k in 0..4 {
            let (a, c) = (corners[k], corners[(k + 1) % 4]);
            quad(
                [a[0], a[1], z0],
                [c[0], c[1], z0],
                [c[0], c[1], z1],
                [a[0], a[1], z1],
                color,
                &mut s.bodies,
            );
        }
        let c = &corners;
        quad(
            [c[0][0], c[0][1], z1],
            [c[1][0], c[1][1], z1],
            [c[2][0], c[2][1], z1],
            [c[3][0], c[3][1], z1],
            lift(color, 1.15),
            &mut s.bodies,
        );
    }
    let [f0, f1] = &mut s.faces;
    let [d0, d1] = &mut s.decals;
    for list in [&mut s.walls, &mut s.bodies, f0, f1, d0, d1] {
        for t in list.iter_mut() {
            for v in t.p.iter_mut() {
                v[1] = -v[1];
            }
        }
    }
    let mut bb = agentee_core::graphic::Bounds::EMPTY;
    outline.iter().for_each(|p| bb.add(*p));
    let c = bb.center();
    s.centre = [c[0] as f32, -c[1] as f32, 0.0];
    s.radius = (bb.size()[0].max(bb.size()[1]) as f32).max(1.0) * 0.75;
    s
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

pub fn show(ui: &mut Ui, scene: &Scene, cam: &mut Camera, interactive: bool) {
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
    }
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, Color32::from_rgb(20, 22, 26));
    let (cy, sy, cp, sp) = (cam.yaw.cos(), cam.yaw.sin(), cam.pitch.cos(), cam.pitch.sin());
    let eye_dir = [sy * cp, -cy * cp, sp];
    let dist = scene.radius * 2.4;
    let eye = [
        scene.centre[0] + eye_dir[0] * dist,
        scene.centre[1] + eye_dir[1] * dist,
        scene.centre[2] + eye_dir[2] * dist,
    ];
    let forward = norm(sub(scene.centre, eye));
    let right = norm(cross(forward, [0.0, 0.0, 1.0]));
    let right = if dot(right, right) < 0.5 { [1.0, 0.0, 0.0] } else { right };
    let up = cross(right, forward);
    let focal = rect.height().min(rect.width()) * 1.25 * cam.zoom;
    let centre = rect.center() + cam.pan;
    let project = |v: V3| -> (Pos2, f32) {
        let d = sub(v, eye);
        let z = dot(d, forward).max(1e-3);
        let x = dot(d, right) / z * focal;
        let y = dot(d, up) / z * focal;
        (Pos2::new(centre.x + x, centre.y - y), z)
    };
    let light = norm([0.3, -0.5, 0.8]);
    let shade = |t: &Tri| -> Option<(f32, [Pos2; 3], Color32)> {
        let n = norm(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0])));
        let pts = t.p.map(project);
        let depth = (pts[0].1 + pts[1].1 + pts[2].1) / 3.0;
        let mut n2 = n;
        let to_eye = sub(eye, t.p[0]);
        if dot(n2, to_eye) < 0.0 {
            n2 = [-n2[0], -n2[1], -n2[2]];
        }
        let k = 0.55 + 0.45 * dot(n2, light).max(0.0);
        Some((depth, pts.map(|q| q.0), lift(t.color, k)))
    };
    let mut mesh = Mesh::default();
    let mut push = |pts: [Pos2; 3], color: Color32| {
        let base = mesh.vertices.len() as u32;
        for q in pts {
            mesh.colored_vertex(q, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    };
    let mut walls: Vec<_> = scene.walls.iter().filter_map(shade).collect();
    walls.sort_by(|a, b| b.0.total_cmp(&a.0));
    let visible = if eye_dir[2] >= 0.0 { 0 } else { 1 };
    for (_, pts, c) in walls {
        push(pts, c);
    }
    for t in &scene.faces[visible] {
        if let Some((_, pts, c)) = shade(t) {
            push(pts, c);
        }
    }
    for t in &scene.decals[visible] {
        if let Some((_, pts, c)) = shade(t) {
            push(pts, c);
        }
    }
    let mut bodies: Vec<_> = scene
        .bodies
        .iter()
        .filter(|t| (t.p[0][2] > 0.0) == (visible == 0))
        .filter_map(shade)
        .collect();
    bodies.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, pts, c) in bodies {
        push(pts, c);
    }
    p.add(mesh);
    if interactive {
        p.text(
            rect.left_bottom() + Vec2::new(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "drag to orbit, shift-drag to pan, scroll to zoom, double-click to reset",
            egui::FontId::proportional(11.0),
            Color32::from_gray(140),
        );
    }
    let _ = Rect::NOTHING;
}

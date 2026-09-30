use crate::{Mesh, MeshBuilder};
use agentee_core::footprint::{Footprint, Pad};
use agentee_core::graphic::{Bounds, Shape};

const BLACK: [f32; 3] = [0.12, 0.12, 0.13];
const METAL: [f32; 3] = [0.82, 0.82, 0.84];
const TIN: [f32; 3] = [0.75, 0.75, 0.77];
const GOLD: [f32; 3] = [0.9, 0.78, 0.45];
const CERAMIC: [f32; 3] = [0.78, 0.66, 0.48];
const FERRITE: [f32; 3] = [0.3, 0.3, 0.32];
const LED_BODY: [f32; 3] = [0.95, 0.95, 0.92];
const MARK: [f32; 3] = [0.55, 0.55, 0.57];

type R = [f64; 2];

struct Builder {
    mesh: MeshBuilder,
}

impl Builder {
    fn new() -> Self {
        Builder { mesh: MeshBuilder::default() }
    }

    fn cuboid(&mut self, colour: [f32; 3], lo: R, hi: R, z0: f64, z1: f64) {
        if hi[0] - lo[0] <= 0.0 || hi[1] - lo[1] <= 0.0 || z1 - z0 <= 0.0 {
            return;
        }
        let p = |x: f64, y: f64, z: f64| [x as f32, -y as f32, z as f32];
        let (x0, y0, x1, y1) = (lo[0], lo[1], hi[0], hi[1]);
        let faces: [([[f64; 3]; 4], [f32; 3]); 6] = [
            ([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], [0.0, 0.0, 1.0]),
            ([[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]], [0.0, 0.0, -1.0]),
            ([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, 1.0, 0.0]),
            ([[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]], [0.0, -1.0, 0.0]),
            ([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], [-1.0, 0.0, 0.0]),
            ([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], [1.0, 0.0, 0.0]),
        ];
        for (q, n) in faces {
            let v = q.map(|c| p(c[0], c[1], c[2]));
            self.mesh.push(colour, [(v[0], n), (v[1], n), (v[2], n)]);
            self.mesh.push(colour, [(v[0], n), (v[2], n), (v[3], n)]);
        }
    }

    fn prism(&mut self, colour: [f32; 3], rings: &[Vec<R>], z0: f64, z1: f64) {
        let p = |q: R, z: f64| [q[0] as f32, -q[1] as f32, z as f32];
        for t in agentee_core::contour::triangles(rings) {
            let (up, down) = ([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]);
            self.mesh.push(colour, [(p(t[0], z1), up), (p(t[1], z1), up), (p(t[2], z1), up)]);
            self.mesh.push(colour, [(p(t[0], z0), down), (p(t[2], z0), down), (p(t[1], z0), down)]);
        }
        let solid = |q: R| {
            rings.iter().filter(|r| agentee_core::geom::point_in_polygon(q, r)).count() % 2 == 1
        };
        for r in rings {
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-9 {
                    continue;
                }
                let mut n = [dy / len, -dx / len];
                let mid = [(a[0] + b[0]) / 2.0 + n[0] * 1e-4, (a[1] + b[1]) / 2.0 + n[1] * 1e-4];
                if solid(mid) {
                    n = [-n[0], -n[1]];
                }
                let n = [n[0] as f32, -n[1] as f32, 0.0];
                let v = [p(a, z0), p(b, z0), p(b, z1), p(a, z1)];
                self.mesh.push(colour, [(v[0], n), (v[1], n), (v[2], n)]);
                self.mesh.push(colour, [(v[0], n), (v[2], n), (v[3], n)]);
            }
        }
    }

    fn finish(self) -> Mesh {
        self.mesh.finish()
    }
}

fn square(c: R, half: f64) -> Vec<R> {
    vec![
        [c[0] - half, c[1] - half],
        [c[0] + half, c[1] - half],
        [c[0] + half, c[1] + half],
        [c[0] - half, c[1] + half],
    ]
}

fn chamfered(c: R, half: f64, cut: f64) -> Vec<R> {
    let (h, k) = (half, half - cut);
    [[-k, -h], [k, -h], [h, -k], [h, k], [k, h], [-k, h], [-h, k], [-h, -k]]
        .into_iter()
        .map(|q| [c[0] + q[0], c[1] + q[1]])
        .collect()
}

fn fab_bounds(fp: &Footprint) -> Option<Bounds> {
    let mut b = Bounds::EMPTY;
    for g in fp.graphics.iter().filter(|g| g.layer == "F.Fab") {
        if !matches!(g.shape, Shape::Text { .. }) {
            b.union(&g.bounds());
        }
    }
    (!b.is_empty()).then_some(b)
}

fn pad_box(p: &Pad) -> (R, R) {
    let [x, y] = p.at.to_mm();
    let [w, h] = p.size.to_mm();
    let quarter = ((p.rotation / 90.0).round() as i64).rem_euclid(2) == 1;
    let (w, h) = if quarter { (h, w) } else { (w, h) };
    ([x - w / 2.0, y - h / 2.0], [x + w / 2.0, y + h / 2.0])
}

fn copper_pads(fp: &Footprint) -> Vec<&Pad> {
    fp.pads.iter().filter(|p| p.is_copper() && !p.number.is_empty()).collect()
}

fn named_height(name: &str) -> Option<f64> {
    name.split('_').find_map(|w| w.strip_prefix('h')?.strip_suffix("mm")?.parse().ok())
}

pub fn generate(fp: &Footprint) -> Option<Mesh> {
    let name = fp.name.as_str();
    let upper = name.to_uppercase();
    let has = |s: &str| upper.starts_with(s);
    if has("MOUNTINGHOLE")
        || has("FIDUCIAL")
        || has("TESTPOINT_PAD")
        || has("SOLDERJUMPER")
        || has("TAG-CONNECT")
    {
        return Some(Mesh::default());
    }
    if upper.contains("SHIELD") || fp.description.to_uppercase().contains("SHIELD") {
        return shield(fp);
    }
    if (has("PINHEADER_") || has("PINSOCKET_")) && upper.contains("_VERTICAL") {
        return header(fp, has("PINSOCKET_"));
    }
    if upper.contains("METRIC") && copper_pads(fp).len() == 2 {
        let kind = upper.split('_').next().unwrap_or("");
        if let Some(m) = chip(fp, kind) {
            return Some(m);
        }
    }
    if has("CRYSTAL_SMD") || has("OSCILLATOR_SMD") {
        return can(fp);
    }
    ic(fp, &upper)
}

fn chip(fp: &Footprint, kind: &str) -> Option<Mesh> {
    let pads = copper_pads(fp);
    let body = fab_bounds(fp)?;
    let (a, b) = (pad_box(pads[0]), pad_box(pads[1]));
    let ca = [(a.0[0] + a.1[0]) / 2.0, (a.0[1] + a.1[1]) / 2.0];
    let cb = [(b.0[0] + b.1[0]) / 2.0, (b.0[1] + b.1[1]) / 2.0];
    let along_x = (cb[0] - ca[0]).abs() >= (cb[1] - ca[1]).abs();
    let [bl, bw] = body.size();
    let (length, width) = if along_x { (bl, bw) } else { (bw, bl) };
    let (colour, height, cap) = match kind {
        "C" => (CERAMIC, width, 0.22),
        "R" => (BLACK, (width * 0.7).min(0.55), 0.18),
        "L" => (FERRITE, width, 0.22),
        "LED" => (LED_BODY, width * 0.9, 0.18),
        "D" => (BLACK, width * 0.9, 0.18),
        "FUSE" => (BLACK, width * 0.8, 0.2),
        _ => return None,
    };
    let height = fp.height.unwrap_or(height);
    let end = length * cap;
    let mut m = Builder::new();
    let (lo, hi) = (body.min, body.max);
    if along_x {
        m.cuboid(colour, [lo[0] + end, lo[1]], [hi[0] - end, hi[1]], 0.0, height);
        m.cuboid(METAL, lo, [lo[0] + end, hi[1]], 0.0, height);
        m.cuboid(METAL, [hi[0] - end, lo[1]], hi, 0.0, height);
    } else {
        m.cuboid(colour, [lo[0], lo[1] + end], [hi[0], hi[1] - end], 0.0, height);
        m.cuboid(METAL, lo, [hi[0], lo[1] + end], 0.0, height);
        m.cuboid(METAL, [lo[0], hi[1] - end], hi, 0.0, height);
    }
    Some(m.finish())
}

fn header(fp: &Footprint, socket: bool) -> Option<Mesh> {
    let pitch = if fp.name.contains("P1.27mm") {
        1.27
    } else if fp.name.contains("P2.00mm") {
        2.0
    } else {
        2.54
    };
    let k = pitch / 2.54;
    let (pin, above, tail) = (0.64 * k, 6.0 * k, 3.0 * k);
    let base = fp.height.unwrap_or(if socket { 8.5 * k } else { 2.5 * k });
    let pins: Vec<R> = fp.pads.iter().filter(|p| p.drill.is_some()).map(|p| p.at.to_mm()).collect();
    if pins.is_empty() {
        return None;
    }
    let half = pitch / 2.0;
    let cut = 0.25 * k;
    let mut m = Builder::new();
    for c in &pins {
        let block = chamfered(*c, half, cut);
        let h = pin / 2.0;
        if socket {
            let (hole, depth) = (0.5 * k, 2.0 * k);
            m.prism(BLACK, std::slice::from_ref(&block), 0.0, base - depth);
            m.prism(BLACK, &[block, square(*c, hole)], base - depth, base);
            m.prism(GOLD, &[square(*c, h)], -tail, 0.0);
        } else {
            m.prism(BLACK, &[block], 0.0, base);
            let tip = 0.5 * k;
            m.prism(GOLD, &[square(*c, h)], -tail + tip, base + above - tip);
            m.prism(GOLD, &[square(*c, h * 0.5)], base + above - tip, base + above);
            m.prism(GOLD, &[square(*c, h * 0.5)], -tail, -tail + tip);
        }
    }
    Some(m.finish())
}

fn shield(fp: &Footprint) -> Option<Mesh> {
    let body = fab_bounds(fp)?;
    let height = fp.height.unwrap_or(3.0);
    let (wall, lip) = (0.2, 1.0);
    let (lo, hi) = (body.min, body.max);
    let mut m = Builder::new();
    m.cuboid(METAL, lo, [hi[0], lo[1] + wall], 0.0, height);
    m.cuboid(METAL, [lo[0], hi[1] - wall], hi, 0.0, height);
    m.cuboid(METAL, lo, [lo[0] + wall, hi[1]], 0.0, height);
    m.cuboid(METAL, [hi[0] - wall, lo[1]], hi, 0.0, height);
    let top = height - wall;
    m.cuboid(METAL, [lo[0] + wall, lo[1] + wall], [hi[0] - wall, lo[1] + lip], top, height);
    m.cuboid(METAL, [lo[0] + wall, hi[1] - lip], [hi[0] - wall, hi[1] - wall], top, height);
    m.cuboid(METAL, [lo[0] + wall, lo[1] + lip], [lo[0] + lip, hi[1] - lip], top, height);
    m.cuboid(METAL, [hi[0] - lip, lo[1] + lip], [hi[0] - wall, hi[1] - lip], top, height);
    Some(m.finish())
}

fn can(fp: &Footprint) -> Option<Mesh> {
    let body = fab_bounds(fp)?;
    let height = fp.height.or_else(|| named_height(&fp.name)).unwrap_or(0.8);
    let mut m = Builder::new();
    let base = height * 0.3;
    m.cuboid(CERAMIC, body.min, body.max, 0.0, base);
    let inset = 0.1f64.min(body.size()[0].min(body.size()[1]) / 10.0);
    m.cuboid(
        METAL,
        [body.min[0] + inset, body.min[1] + inset],
        [body.max[0] - inset, body.max[1] - inset],
        base,
        height,
    );
    Some(m.finish())
}

const FAMILIES: &[(&str, f64, bool)] = &[
    ("SOIC", 1.5, true),
    ("SOP", 1.5, true),
    ("SSOP", 1.5, true),
    ("TSSOP", 1.0, true),
    ("MSOP", 1.0, true),
    ("TSOP", 1.0, true),
    ("SOT-23", 1.1, true),
    ("SOT-353", 0.95, true),
    ("SOT-363", 0.95, true),
    ("SC-70", 0.95, true),
    ("SOT-563", 0.55, false),
    ("SOT-89", 1.5, false),
    ("LQFP", 1.5, true),
    ("TQFP", 1.1, true),
    ("QFP", 2.0, true),
    ("D_SOD-123", 1.1, false),
    ("D_SOD-323", 0.9, false),
    ("D_SOD-882", 0.5, false),
    ("D_SMA", 2.3, false),
    ("D_SMB", 2.3, false),
    ("DHVQFN", 0.85, false),
    ("UQFN", 0.55, false),
    ("VQFN", 0.9, false),
    ("WQFN", 0.75, false),
    ("QFN", 0.9, false),
    ("UDFN", 0.55, false),
    ("DFN", 0.6, false),
    ("WSON", 0.75, false),
    ("USON", 0.55, false),
    ("VSON", 0.9, false),
    ("SON", 0.9, false),
    ("TSSLP", 0.32, false),
    ("TSLP", 0.4, false),
    ("TFBGA", 1.2, false),
    ("BGA", 1.4, false),
];

fn ic(fp: &Footprint, upper: &str) -> Option<Mesh> {
    let (height, gull) = FAMILIES
        .iter()
        .filter(|(k, _, _)| upper.starts_with(k) || upper.contains(&format!("_{k}")))
        .max_by_key(|(k, _, _)| k.len())
        .map(|(_, h, g)| (*h, *g))
        .or_else(|| named_height(&fp.name).map(|h| (h, false)))
        .or_else(|| fp.height.map(|h| (h, false)))?;
    let height = fp.height.or_else(|| named_height(&fp.name)).unwrap_or(height);
    let body = fab_bounds(fp)?;
    let bga = upper.contains("BGA");
    let lift = if gull {
        0.1
    } else if bga {
        0.25
    } else {
        0.0
    };
    let mut m = Builder::new();
    m.cuboid(BLACK, body.min, body.max, lift, height);
    for p in copper_pads(fp) {
        let (lo, hi) = pad_box(p);
        let outside = [
            (lo[0] < body.min[0]).then(|| ([lo[0], lo[1]], [body.min[0], hi[1]])),
            (hi[0] > body.max[0]).then(|| ([body.max[0], lo[1]], [hi[0], hi[1]])),
            (lo[1] < body.min[1]).then(|| ([lo[0], lo[1]], [hi[0], body.min[1]])),
            (hi[1] > body.max[1]).then(|| ([lo[0], body.max[1]], [hi[0], hi[1]])),
        ];
        for (a, b) in outside.into_iter().flatten() {
            let narrow = |v0: f64, v1: f64| {
                let c = (v0 + v1) / 2.0;
                let h = (v1 - v0) * 0.35;
                (c - h, c + h)
            };
            let (a, b) = if b[0] - a[0] > b[1] - a[1] {
                let (y0, y1) = narrow(a[1], b[1]);
                ([a[0], y0], [b[0], y1])
            } else {
                let (x0, x1) = narrow(a[0], b[0]);
                ([x0, a[1]], [x1, b[1]])
            };
            let top = if gull { (height * 0.4).min(0.6) } else { 0.15 };
            m.cuboid(TIN, a, b, 0.0, top);
        }
    }
    if let Some(one) = fp.pads.iter().find(|p| p.number == "1" || p.number == "A1") {
        let [x, y] = one.at.to_mm();
        let d = 0.12 * body.size()[0].min(body.size()[1]).min(2.5);
        let cx = x.clamp(body.min[0] + 2.0 * d, body.max[0] - 2.0 * d);
        let cy = y.clamp(body.min[1] + 2.0 * d, body.max[1] - 2.0 * d);
        m.cuboid(
            MARK,
            [cx - d / 2.0, cy - d / 2.0],
            [cx + d / 2.0, cy + d / 2.0],
            height,
            height + 0.01,
        );
    }
    Some(m.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentee_core::footprint::FootprintFile;

    fn footprint(text: &str) -> Footprint {
        let file: FootprintFile = toml::from_str(text).unwrap();
        file.resolve(&mut agentee_core::diag::Diags::new("test"))
    }

    const CAP: &str = r#"
name = "C_0402_1005Metric"
[[pads]]
number = "1"
kind = "smd"
shape = "roundrect"
at = [-0.48, 0.0]
size = [0.56, 0.62]
[[pads]]
number = "2"
kind = "smd"
shape = "roundrect"
at = [0.48, 0.0]
size = [0.56, 0.62]
[[graphics]]
kind = "rect"
layer = "F.Fab"
start = [-0.5, -0.25]
end = [0.5, 0.25]
width = 0.1
"#;

    #[test]
    fn a_chip_cap_fills_its_fab_outline_and_stands_as_tall_as_it_is_wide() {
        let m = generate(&footprint(CAP)).unwrap();
        let (lo, hi) = m.bounds();
        assert!((lo[0] + 0.5).abs() < 1e-6 && (hi[0] - 0.5).abs() < 1e-6);
        assert!((hi[2] - 0.5).abs() < 1e-6);
        assert_eq!(m.parts.len(), 2);
    }

    #[test]
    fn a_shield_frame_leaves_its_middle_open() {
        let text = r#"
name = "Laird_Technologies_BMI-S-230-F_50.8x38.1mm"
height = 5.08
[[graphics]]
kind = "rect"
layer = "F.Fab"
start = [-25.4, -19.05]
end = [25.4, 19.05]
width = 0.1
"#;
        let m = generate(&footprint(text)).unwrap();
        let (lo, hi) = m.bounds();
        assert!((hi[2] - 5.08).abs() < 1e-5 && lo[2].abs() < 1e-6);
        let centre = m
            .parts
            .iter()
            .flat_map(|p| &p.positions)
            .any(|p| p[0].abs() < 20.0 && p[1].abs() < 15.0);
        assert!(!centre);
    }

    #[test]
    fn a_header_has_a_notched_block_and_a_pin_through_the_board_per_pad() {
        let text = r#"
name = "PinHeader_1x02_P2.54mm_Vertical"
[[pads]]
number = "1"
kind = "tht"
shape = "rect"
at = [0, 0]
size = [1.7, 1.7]
drill = 1.0
[[pads]]
number = "2"
kind = "tht"
shape = "oval"
at = [0, 2.54]
size = [1.7, 1.7]
drill = 1.0
"#;
        let m = generate(&footprint(text)).unwrap();
        let (lo, hi) = m.bounds();
        assert!(lo[2] < -2.9 && hi[2] > 8.0);
        let block = m.parts.iter().find(|p| p.colour == BLACK).unwrap();
        let notch = block.positions.iter().any(|p| (p[0].abs() - 1.02).abs() < 1e-3);
        assert!(notch);
    }

    #[test]
    fn mounting_holes_have_no_body() {
        let text = "name = \"MountingHole_3.2mm_M3_Pad_Via\"\n";
        assert!(generate(&footprint(text)).unwrap().parts.is_empty());
    }
}

use crate::footprint::{Footprint, Pad};
use crate::graphic::{Bounds, Shape};

pub const FAMILIES: &[(&str, f64, bool)] = &[
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

pub const SHIELD: f64 = 3.0;
pub const CAN: f64 = 0.8;
pub const SMA_AXIS: f64 = 0.65;
pub const SMA_FLANGE: f64 = 3.175;
pub const HEADER_BASE: f64 = 2.5;
pub const HEADER_PIN: f64 = 6.0;
pub const SOCKET: f64 = 8.5;

pub fn named_height(name: &str) -> Option<f64> {
    name.split('_').find_map(|w| w.strip_prefix('h')?.strip_suffix("mm")?.parse().ok())
}

pub fn family(upper: &str) -> Option<(f64, bool)> {
    FAMILIES
        .iter()
        .filter(|(k, _, _)| upper.starts_with(k) || upper.contains(&format!("_{k}")))
        .max_by_key(|(k, _, _)| k.len())
        .map(|(_, h, g)| (*h, *g))
}

pub fn chip_height(kind: &str, width: f64) -> Option<f64> {
    Some(match kind {
        "C" | "L" => width,
        "R" => (width * 0.7).min(0.55),
        "LED" | "D" => width * 0.9,
        "FUSE" => width * 0.8,
        _ => return None,
    })
}

pub fn header_pitch(name: &str) -> f64 {
    if name.contains("P1.27mm") {
        1.27
    } else if name.contains("P2.00mm") {
        2.0
    } else {
        2.54
    }
}

pub fn fab_bounds(fp: &Footprint) -> Option<Bounds> {
    let mut b = Bounds::EMPTY;
    for g in fp.graphics.iter().filter(|g| g.layer == "F.Fab") {
        if !matches!(g.shape, Shape::Text { .. }) {
            b.union(&g.bounds());
        }
    }
    (!b.is_empty()).then_some(b)
}

pub fn pad_box(p: &Pad) -> ([f64; 2], [f64; 2]) {
    let [x, y] = p.at.to_mm();
    let [w, h] = p.size.to_mm();
    let quarter = ((p.rotation / 90.0).round() as i64).rem_euclid(2) == 1;
    let (w, h) = if quarter { (h, w) } else { (w, h) };
    ([x - w / 2.0, y - h / 2.0], [x + w / 2.0, y + h / 2.0])
}

pub fn copper_pads(fp: &Footprint) -> Vec<&Pad> {
    fp.pads.iter().filter(|p| p.is_copper() && !p.number.is_empty()).collect()
}

pub fn chip_body(fp: &Footprint) -> Option<(Bounds, bool, f64, f64)> {
    let pads = copper_pads(fp);
    if pads.len() != 2 {
        return None;
    }
    let body = fab_bounds(fp)?;
    let (a, b) = (pad_box(pads[0]), pad_box(pads[1]));
    let ca = [(a.0[0] + a.1[0]) / 2.0, (a.0[1] + a.1[1]) / 2.0];
    let cb = [(b.0[0] + b.1[0]) / 2.0, (b.0[1] + b.1[1]) / 2.0];
    let along_x = (cb[0] - ca[0]).abs() >= (cb[1] - ca[1]).abs();
    let [bl, bw] = body.size();
    let (length, width) = if along_x { (bl, bw) } else { (bw, bl) };
    Some((body, along_x, length, width))
}

pub fn body_height(fp: &Footprint) -> Option<f64> {
    if let Some(h) = fp.height {
        return Some(h);
    }
    let upper = fp.name.to_uppercase();
    let has = |s: &str| upper.starts_with(s);
    if has("MOUNTINGHOLE")
        || has("FIDUCIAL")
        || has("TESTPOINT_PAD")
        || has("SOLDERJUMPER")
        || has("TAG-CONNECT")
    {
        return None;
    }
    if upper.contains("SHIELD") || fp.description.to_uppercase().contains("SHIELD") {
        return fab_bounds(fp).map(|_| SHIELD);
    }
    if upper.contains("SMA") && upper.contains("EDGEMOUNT") {
        return fab_bounds(fp).map(|_| SMA_AXIS + SMA_FLANGE);
    }
    if (has("PINHEADER_") || has("PINSOCKET_")) && upper.contains("_VERTICAL") {
        let k = header_pitch(&fp.name) / 2.54;
        return Some(if has("PINSOCKET_") { SOCKET * k } else { (HEADER_BASE + HEADER_PIN) * k });
    }
    if upper.contains("METRIC")
        && let Some((_, _, _, width)) = chip_body(fp)
        && let Some(h) = chip_height(upper.split('_').next().unwrap_or(""), width)
    {
        return Some(h);
    }
    if has("CRYSTAL_SMD") || has("OSCILLATOR_SMD") {
        return fab_bounds(fp).map(|_| named_height(&fp.name).unwrap_or(CAN));
    }
    fab_bounds(fp)?;
    named_height(&fp.name).or_else(|| family(&upper).map(|f| f.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::FootprintFile;

    fn footprint(name: &str, extra: &str) -> Footprint {
        let text = format!(
            "name = \"{name}\"\n{extra}\n[[pads]]\nnumber = \"1\"\nkind = \"tht\"\nshape = \"circle\"\nat = [0, 0]\nsize = [1.7, 1.7]\ndrill = 1.0\n\n[[pads]]\nnumber = \"2\"\nkind = \"tht\"\nshape = \"circle\"\nat = [0, 2.54]\nsize = [1.7, 1.7]\ndrill = 1.0\n\n[[graphics]]\nkind = \"rect\"\nlayer = \"F.Fab\"\nstart = [-1.27, -1.27]\nend = [1.27, 3.81]\n"
        );
        let file: FootprintFile = toml::from_str(&text).unwrap();
        file.resolve(&mut crate::diag::Diags::new("test"))
    }

    #[test]
    fn body_height_falls_back_to_the_3d_view_defaults() {
        let h = |name: &str| body_height(&footprint(name, ""));
        assert_eq!(h("PinHeader_1x02_P2.54mm_Vertical"), Some(8.5));
        assert_eq!(h("PinSocket_1x02_P1.27mm_Vertical"), Some(4.25));
        assert_eq!(h("SOIC-8_3.9x4.9mm_P1.27mm"), Some(1.5));
        assert_eq!(h("L_Bourns_SRR1260_h6.0mm"), Some(6.0));
        assert_eq!(h("MountingHole_3.2mm_M3"), None);
        assert_eq!(h("Weird_Part"), None);
        assert_eq!(body_height(&footprint("SOIC-8", "height = \"4mm\"")), Some(4.0));
    }
}

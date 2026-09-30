use super::Rule;
use crate::footprint::{PadKind, PadShape};
use crate::geom;
use crate::layout::Placed;

pub static RULES: &[Rule] = &[];

pub fn bga_pitch(p: &Placed) -> Option<(f64, f64)> {
    let balls: Vec<&crate::footprint::Pad> = p
        .footprint
        .pads
        .iter()
        .filter(|q| q.kind == PadKind::Smd && q.shape == PadShape::Circle)
        .collect();
    if balls.len() < 16 {
        return None;
    }
    let mut pitch = f64::MAX;
    for (i, a) in balls.iter().enumerate() {
        for b in &balls[i + 1..] {
            let d = geom::dist(a.at.to_mm(), b.at.to_mm());
            if d > 1e-6 {
                pitch = pitch.min(d);
            }
        }
    }
    let pad = balls.iter().map(|q| q.size.to_mm()[0]).fold(f64::MAX, f64::min);
    (pitch < f64::MAX).then_some((pitch, pad))
}

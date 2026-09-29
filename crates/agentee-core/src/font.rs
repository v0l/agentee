use crate::geom::{self, P};
use crate::graphic::Anchor;

include!("font_data.rs");

pub fn default_thickness(size: f64) -> f64 {
    size * THICKNESS
}

pub fn strokes(
    text: &str,
    at: P,
    size: f64,
    rotation: f64,
    anchor: Anchor,
    mirror: bool,
) -> Vec<Vec<P>> {
    let s = size / CAP;
    let mut cursor = 0.0;
    let mut out: Vec<Vec<P>> = Vec::new();
    for c in text.chars() {
        let i = (c as u32).wrapping_sub(32) as usize;
        let (adv, glyph) = GLYPHS.get(i).copied().unwrap_or(GLYPHS[('?' as usize) - 32]);
        for st in glyph {
            out.push(
                st.iter()
                    .map(|(x, y)| [(cursor + *x as f64) * s, (CAP / 2.0 - *y as f64) * s])
                    .collect(),
            );
        }
        cursor += adv as f64;
    }
    let (lo, hi) =
        out.iter().flatten().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    if lo > hi {
        return out;
    }
    let shift = match anchor {
        Anchor::Left => -lo,
        Anchor::Center => -(lo + hi) / 2.0,
        Anchor::Right => -hi,
    };
    for st in out.iter_mut() {
        for p in st.iter_mut() {
            let x = p[0] + shift;
            let q = [if mirror { -x } else { x }, p[1]];
            let [rx, ry] = geom::rotate(q, rotation);
            *p = [rx + at[0], ry + at[1]];
        }
    }
    out
}

pub fn ink_width(text: &str, size: f64) -> f64 {
    let pts: Vec<P> =
        strokes(text, [0.0, 0.0], size, 0.0, Anchor::Left, false).into_iter().flatten().collect();
    pts.iter().map(|p| p[0]).fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capital_h_is_as_tall_as_the_size() {
        let st = strokes("H", [0.0, 0.0], 1.0, 0.0, Anchor::Center, false);
        let ys: Vec<f64> = st.iter().flatten().map(|p| p[1]).collect();
        let (lo, hi) = ys.iter().fold((f64::MAX, f64::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
        assert!((hi - lo - 1.0).abs() < 1e-9);
        assert!((hi + lo).abs() < 1e-9);
        assert!(ink_width("RF OUT", 1.0) > ink_width("RF", 1.0));
    }
}

use super::{Rule, edge_distance};
use crate::geom;
use crate::layout::{PlacedPad, Via};

pub static RULES: &[Rule] = &[];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViaOnPad {
    Inside,
    Cuts { centre_inside: bool, edge: f64 },
}

pub fn via_on_pad(v: &Via, q: &PlacedPad) -> Option<ViaOnPad> {
    let r = v.diameter / 2.0;
    let centre_inside = q.outlines.iter().any(|o| geom::point_in_polygon(v.at, o));
    let edge = q.outlines.iter().map(|o| edge_distance(o, v.at)).fold(f64::MAX, f64::min);
    if !centre_inside && edge > r + 1e-6 {
        return None;
    }
    let probe = (r - 0.002).max(0.0);
    let inside = centre_inside
        && geom::circle(v.at, probe, 32)
            .into_iter()
            .all(|c| q.outlines.iter().any(|o| geom::point_in_polygon(c, o)));
    Some(if inside { ViaOnPad::Inside } else { ViaOnPad::Cuts { centre_inside, edge } })
}

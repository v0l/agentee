use crate::gpu::gpu;
use agentee_core::board::{Board, LayerKind};

pub use sims_quasistatic::xsection::*;

pub fn stack(board: &Board, layer: &str, mask: bool) -> Result<Stack, String> {
    let layers = &board.stackup.layers;
    let i = layers.iter().position(|l| l.name == layer && l.kind == LayerKind::Copper).ok_or_else(
        || {
            format!(
                "`{layer}` is not a copper layer of {} ({})",
                board.name,
                board.stackup.copper_names().join(", ")
            )
        },
    )?;
    let collect = |range: Box<dyn Iterator<Item = usize>>| {
        let mut v = Vec::new();
        let mut plane = false;
        let mut mask_layer = None;
        for k in range {
            let l = &layers[k];
            match l.kind {
                LayerKind::Copper => {
                    plane = true;
                    break;
                }
                LayerKind::Core | LayerKind::Prepreg => {
                    v.push((l.thickness.to_mm(), l.er, l.loss_tangent))
                }
                LayerKind::Mask => {
                    mask_layer = Some((l.thickness.to_mm().max(0.005), l.er, l.loss_tangent))
                }
                _ => {}
            }
        }
        (v, plane, mask_layer)
    };
    let (above, plane_above, mask_above) = collect(Box::new((0..i).rev()));
    let (below, plane_below, mask_below) = collect(Box::new(i + 1..layers.len()));
    if above.is_empty() && below.is_empty() {
        return Err(format!("`{layer}` has no dielectric next to it"));
    }
    let fill_er = above.first().or(below.first()).map(|x| x.1).unwrap_or(1.0);
    let outer_mask = if above.is_empty() { mask_above } else { mask_below };
    Ok(Stack {
        above,
        below,
        plane_above,
        plane_below,
        copper: layers[i].thickness.to_mm(),
        fill_er,
        mask: if mask && (!plane_above || !plane_below) { outer_mask } else { None },
    })
}

pub fn line(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
) -> Result<Line, String> {
    let stack = stack(board, layer, mask)?;
    let (g, diff) = build(&stack, trace, if fine { &Resolution::FINE } else { &Resolution::FAST })?;
    Ok(solve(g, diff, gpu(), if fine { 1e-8 } else { 1e-7 }))
}

pub fn pair(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
) -> Result<Pair, String> {
    if trace.diff_gap.is_none() {
        return Err("a pair needs a gap".into());
    }
    let stack = stack(board, layer, mask)?;
    let res = if fine { &Resolution::FINE } else { &Resolution::FAST };
    let tol = if fine { 1e-8 } else { 1e-7 };
    let (go, _) = build_mode(&stack, trace, res, false)?;
    let (ge, _) = build_mode(&stack, trace, res, true)?;
    Ok(pair_of(&solve(go, true, gpu(), tol), &solve(ge, true, gpu(), tol)))
}

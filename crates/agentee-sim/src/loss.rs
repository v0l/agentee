use crate::gpu::gpu;
use crate::xsection;
use agentee_core::board::Board;

pub use sims_quasistatic::loss::*;
use sims_quasistatic::xsection::{Resolution, Trace};

pub fn line(
    board: &Board,
    layer: &str,
    trace: &Trace,
    mask: bool,
    fine: bool,
    model: &Model,
    freqs: &[f64],
) -> Result<Vec<Sweep>, String> {
    let stack = xsection::stack(board, layer, mask)?;
    let res = if fine { &Resolution::FINE } else { &Resolution::FAST };
    let tol = if fine { 1e-8 } else { 1e-7 };
    let area = trace.width * stack.copper;
    let modes: &[(bool, &str)] = if trace.diff_gap.is_some() {
        &[(false, "odd"), (true, "even")]
    } else {
        &[(false, "single")]
    };
    modes
        .iter()
        .map(|(even, name)| {
            let (g, _) = xsection::build_mode(&stack, trace, res, *even)?;
            let geom = wheeler(&stack, trace, res, *even, gpu(), tol)?;
            Ok(sweep(&g, trace.diff_gap.is_some(), area, geom, model, freqs, name, gpu(), tol))
        })
        .collect()
}

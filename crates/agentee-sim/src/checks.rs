use crate::xsection::{self, Line, Trace};
use agentee_core::board::{Board, Solver};
use agentee_core::diag::{Diagnostic, Severity};
use agentee_core::project::Project;
use agentee_core::units::Length;

pub fn width_for(
    board: &Board,
    layer: &str,
    trace: &Trace,
    target: f64,
    mask: bool,
) -> Result<(f64, Line), String> {
    let z = |w: f64| -> Result<Line, String> {
        xsection::line(board, layer, &Trace { width: w, ..trace.clone() }, mask, false)
    };
    let (mut w0, mut w1) = (trace.width, trace.width * 1.15);
    let (mut z0, mut z1) = (z(w0)?.z0, z(w1)?.z0);
    for _ in 0..12 {
        if (z1 - target).abs() / target < 0.002 || (z1 - z0).abs() < 1e-9 {
            break;
        }
        let w2 = (w1 + (target - z1) * (w1 - w0) / (z1 - z0))
            .clamp(trace.width / 4.0, trace.width * 4.0);
        (w0, z0) = (w1, z1);
        w1 = w2;
        z1 = z(w1)?.z0;
    }
    let fine = xsection::line(board, layer, &Trace { width: w1, ..trace.clone() }, mask, true)?;
    Ok((w1, fine))
}

pub fn board(board: &Board) -> Vec<(Severity, String, String)> {
    let mut out = Vec::new();
    for n in board.netclasses.iter().filter(|n| n.solver == Solver::Field) {
        let Some(target) = n.impedance else {
            out.push((
                Severity::Warning,
                format!("netclass {}", n.name),
                "`solver = \"field\"` without an `impedance` target".into(),
            ));
            continue;
        };
        for layer in &n.layers {
            let at = format!("netclass {} on {layer}", n.name);
            let trace = Trace {
                width: n.width_on(layer).to_mm(),
                diff_gap: n.diff_gap.map(Length::to_mm),
                coplanar_gap: n.coplanar_gap.map(Length::to_mm),
            };
            let solved = match xsection::line(board, layer, &trace, true, false) {
                Ok(r) => r,
                Err(e) => {
                    out.push((Severity::Error, at, e));
                    continue;
                }
            };
            let tol = target.0 * n.impedance_tolerance.0 / 100.0;
            if (solved.z0 - target.0).abs() <= tol {
                out.push((
                    Severity::Info,
                    at,
                    format!(
                        "field solver: {:.1} ohm with solder mask, eeff {:.3} ({})",
                        solved.z0, solved.eeff, solved.device
                    ),
                ));
                continue;
            }
            let hint = match width_for(board, layer, &trace, target.0, true) {
                Ok((w, fine)) => {
                    let key = if n.widths.contains_key(layer) {
                        format!("widths.\"{layer}\"")
                    } else {
                        "track_width".to_string()
                    };
                    format!(", use {key} = \"{}\" ({:.1} ohm)", Length::mm(w), fine.z0)
                }
                Err(_) => String::new(),
            };
            out.push((
                Severity::Error,
                at,
                format!(
                    "field solver gives {:.1} ohm with solder mask, outside {} +/- {}{hint}",
                    solved.z0, target, n.impedance_tolerance
                ),
            ));
        }
    }
    out
}

pub fn apply(project: &mut Project) {
    for e in &mut project.boards {
        for (severity, at, message) in board(&e.item) {
            e.diags.push(Diagnostic {
                severity,
                file: Some(e.path.clone()),
                item: e.name.clone(),
                at,
                message,
                rule: None,
            });
        }
    }
}

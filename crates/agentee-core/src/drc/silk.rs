use super::{Category, Ctx, Report, Rule, every, recorded};
use crate::diag::Severity;
use crate::graphic::{Fill, Shape};

pub static RULES: &[Rule] = &[
    Rule {
        id: "silk-text",
        category: Category::Silk,
        severity: Severity::Error,
        summary: "silk text that crowds other text, sits on pads, prints over vias, crosses a part's silk outline or runs off the board; a reference gets a clear spot to move to",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "silk-hidden",
        category: Category::Silk,
        severity: Severity::Warning,
        summary: "silk text that is only hidden under the body of another part",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "silk-text-height",
        category: Category::Silk,
        severity: Severity::Warning,
        summary: "silk text shorter than min_silk_text_height",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "silk-artwork",
        category: Category::Silk,
        severity: Severity::Error,
        summary: "silk artwork that sits on pads, overlaps silk text or runs off the board",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "watermark",
        category: Category::Silk,
        severity: Severity::Error,
        summary: "the agentee version watermark has no clear spot on the silk, or the [watermark] spot is not clear; fab refuses without it",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "silk-width",
        category: Category::Silk,
        severity: Severity::Warning,
        summary: "board silk lines of the layout thinner than min_silk_width (footprint silk is checked with the footprint)",
        when: "every board",
        applies: every,
        check: silk_width,
    },
];

fn silk_width(cx: &Ctx, r: &mut Report) {
    let min = cx.board.rules.min_silk_width;
    let thin: Vec<_> = cx
        .graphics
        .iter()
        .filter(|g| g.layer.ends_with(".SilkS") && !matches!(g.shape, Shape::Text { .. }))
        .filter(|g| g.width < min && !(g.fill != Fill::None && g.width.to_mm() <= 0.0))
        .collect();
    let Some(worst) = thin.iter().min_by_key(|g| g.width) else { return };
    let at = crate::footprint::graphic_path(worst)
        .first()
        .map(|p| format!(", at [{:.3}, {:.3}] on {}", p[0], p[1], worst.layer))
        .unwrap_or_default();
    r.emit(
        "graphics",
        format!(
            "{} board silk lines under the fab minimum width {min}, thinnest {}{at}",
            thin.len(),
            worst.width
        ),
    );
}

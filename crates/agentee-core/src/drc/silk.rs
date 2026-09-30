use super::{Category, Rule, every, recorded};
use crate::diag::Severity;

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
];

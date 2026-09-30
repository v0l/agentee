use super::{Category, Rule, Setup, every, recorded};
use crate::diag::Severity;

pub static RULES: &[Rule] = &[
    Rule {
        id: "zone-overlap",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "the fills of two zones of different nets on one layer overlap, a short",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-to-zone",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "the fills of two zones of different nets closer than their clearance",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-clearance",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "a fill that covers or comes closer than the clearance to copper of another net",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-tips",
        category: Category::Zone,
        severity: Severity::Warning,
        summary: "sharp fill tips under 30 degrees, which etch unevenly and can lift",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-islands",
        category: Category::Zone,
        severity: Severity::Info,
        summary: "counts the fill islands that reach nothing of the zone's net and were removed",
        when: "every board",
        applies: every,
        check: recorded,
    },
];

fn with_zones(s: &Setup) -> bool {
    s.zones
}

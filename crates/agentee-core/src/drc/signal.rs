use super::{Category, Rule, Setup, recorded};
use crate::diag::Severity;

pub static RULES: &[Rule] = &[
    Rule {
        id: "pair-skew",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a pair whose length skew is over its `max_skew` or the class max_skew, with the net to lengthen",
        when: "pairs",
        applies: with_pairs,
        check: recorded,
    },
    Rule {
        id: "pair-skew-info",
        category: Category::Signal,
        severity: Severity::Info,
        summary: "the length and delay skew of each pair within its limit",
        when: "pairs",
        applies: with_pairs,
        check: recorded,
    },
    Rule {
        id: "pair-gap",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a pair run side by side at another gap than the class diff_gap, beyond the class max_uncoupled",
        when: "pairs",
        applies: with_pairs,
        check: recorded,
    },
    Rule {
        id: "pair-coupling",
        category: Category::Signal,
        severity: Severity::Warning,
        summary: "a pair of which less than 80% of the longer net runs side by side at the pair gap",
        when: "pairs",
        applies: with_pairs,
        check: recorded,
    },
    Rule {
        id: "match-length",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a net of a match group off the group target by more than its tolerance",
        when: "match groups",
        applies: with_match_groups,
        check: recorded,
    },
    Rule {
        id: "interface-pair",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a net of a differential interface with no pair partner",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-impedance",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "an interface net whose class has no impedance target, one outside the interface's window, or no pair gap on a differential interface",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-skew",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a pair of an interface skewed more than the interface's max_skew",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-bus-skew",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "the data signals of an interface spread more than max_bus_skew in delay",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-clock-window",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a data signal arriving outside the interface's clock_window from the clock",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-vias",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a lane with more vias than the interface's max_vias",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-stub",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a via that leaves a stub longer than the interface's max_stub",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-return-via",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a signal via with no reference net via within the interface's return_via distance",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-length",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a lane longer than the interface's max_length",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
    Rule {
        id: "interface-reference",
        category: Category::Signal,
        severity: Severity::Error,
        summary: "a lane running longer than max_unreferenced with no reference plane next to it",
        when: "interfaces",
        applies: with_interfaces,
        check: recorded,
    },
];

fn with_pairs(s: &Setup) -> bool {
    s.pairs
}

fn with_match_groups(s: &Setup) -> bool {
    s.match_groups
}

fn with_interfaces(s: &Setup) -> bool {
    s.interfaces
}

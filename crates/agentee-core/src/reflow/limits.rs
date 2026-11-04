use super::defaults::{self, Basis, Family};
use super::inputs::PartInput;
use super::paste::Paste;
use super::{Capable, Confidence, Finding, LimitsFile, ReflowFile, Termination};
use crate::diag::Severity;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Design,
    Library,
    PackageDefault,
}

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub max_peak: f64,
    pub max_time_at_peak: f64,
    pub max_time_above_liquidus: f64,
    pub max_ramp_up: f64,
    pub max_ramp_down: f64,
    pub msl: String,
    pub floor_life_hours: f64,
    pub max_passes: u32,
    pub capable: Capable,
    pub termination: Option<Termination>,
    pub last_pass: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Resolved {
    pub reference: String,
    pub mpn: Option<String>,
    pub footprint: String,
    pub family: Family,
    pub origin: Origin,
    pub limits: Limits,
    pub source: Option<String>,
    pub page: Option<String>,
    pub confidence: Confidence,
    pub assumed_fields: Vec<String>,
    pub note: String,
    #[serde(skip)]
    pub mass_g: Option<f64>,
    #[serde(skip)]
    pub local_copper_fill: Option<f64>,
}

fn same_mpn(a: &str, b: &str) -> bool {
    let n = |s: &str| {
        super::inputs::last_word(s)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    !n(a).is_empty() && n(a) == n(b)
}

pub fn entry_for<'a>(file: &'a ReflowFile, p: &PartInput) -> Option<&'a LimitsFile> {
    file.parts.iter().find(|e| e.refs.iter().any(|r| r == &p.reference)).or_else(|| {
        let m = p.mpn.as_deref()?;
        file.parts
            .iter()
            .find(|e| e.refs.is_empty() && e.mpn.as_deref().is_some_and(|x| same_mpn(x, m)))
    })
}

pub fn termination_of(l: &LimitsFile) -> Option<Termination> {
    l.termination.or_else(|| {
        l.ball_alloy.as_deref().map(|a| {
            if a.to_lowercase().contains("pb") {
                Termination::TinLead
            } else {
                Termination::LeadFree
            }
        })
    })
}

fn merge(top: &LimitsFile, below: &LimitsFile) -> LimitsFile {
    macro_rules! pick {
        ($f:ident) => {
            top.$f.clone().or_else(|| below.$f.clone())
        };
    }
    LimitsFile {
        refs: top.refs.clone(),
        mpn: pick!(mpn),
        max_peak: pick!(max_peak),
        max_time_at_peak: pick!(max_time_at_peak),
        max_time_above_liquidus: pick!(max_time_above_liquidus),
        max_ramp_up: pick!(max_ramp_up),
        max_ramp_down: pick!(max_ramp_down),
        msl: pick!(msl),
        floor_life_hours: pick!(floor_life_hours),
        max_passes: pick!(max_passes),
        capable: pick!(capable),
        termination: pick!(termination),
        ball_alloy: pick!(ball_alloy),
        mass_g: pick!(mass_g),
        last_pass: top.last_pass || below.last_pass,
        local_copper_fill: pick!(local_copper_fill),
        source: pick!(source),
        page: pick!(page),
        confidence: pick!(confidence),
        note: if top.note.is_empty() { below.note.clone() } else { top.note.clone() },
    }
}

pub fn resolve_part(
    file: &ReflowFile,
    p: &PartInput,
    paste: &Paste,
    out: &mut Vec<Finding>,
) -> Resolved {
    let design = entry_for(file, p);
    let library = p.library.as_ref();
    let family = defaults::classify(&p.footprint, &p.reference, p.mount);
    let basis = Basis { family, body: p.body, lead_free: paste.lead_free };
    let fallback = defaults::package_default(&basis, p.mount);
    let (top, origin) = match (design, library) {
        (Some(d), Some(l)) => (merge(d, l), Origin::Design),
        (Some(d), None) => (d.clone(), Origin::Design),
        (None, Some(l)) => (l.clone(), Origin::Library),
        (None, None) => (fallback.clone(), Origin::PackageDefault),
    };
    let mut assumed_fields = Vec::new();
    macro_rules! field {
        ($f:ident) => {
            match top.$f.clone() {
                Some(v) => v,
                None => {
                    assumed_fields.push(stringify!($f).to_string());
                    fallback.$f.clone().expect("package default sets every limit")
                }
            }
        };
    }
    let max_peak = field!(max_peak);
    let max_time_at_peak = field!(max_time_at_peak);
    let max_time_above_liquidus = field!(max_time_above_liquidus);
    let max_ramp_up = field!(max_ramp_up);
    let max_ramp_down = field!(max_ramp_down);
    let msl = field!(msl);
    let max_passes = field!(max_passes);
    let capable = field!(capable);
    let at = p.reference.clone();
    let floor_life_hours = match top.floor_life_hours {
        Some(v) => v,
        None => match defaults::floor_life_hours(&msl) {
            Some(v) => v,
            None => {
                out.push(Finding::new(
                    Severity::Error,
                    "reflow-msl",
                    &at,
                    format!("moisture sensitivity level `{msl}` is not 1, 2, 2a, 3, 4, 5, 5a or 6"),
                ));
                0.0
            }
        },
    };
    let termination = termination_of(&top).or_else(|| termination_of(&fallback));
    let mut confidence = top.confidence.unwrap_or(Confidence::Assumed);
    if origin == Origin::PackageDefault {
        confidence = Confidence::Assumed;
        out.push(Finding::new(
            Severity::Warning,
            "reflow-default",
            &at,
            format!(
                "{} has no sourced reflow limits: using the {} default ({} C peak, {} s at peak, MSL {}), assumed",
                p.mpn.as_deref().unwrap_or(&p.value),
                family.label(),
                max_peak,
                max_time_at_peak,
                msl
            ),
        ));
    } else {
        if top.source.is_none() {
            if confidence != Confidence::Assumed {
                out.push(Finding::new(
                    Severity::Warning,
                    "reflow-unsourced",
                    &at,
                    "limits name no source: marked assumed",
                ));
            } else {
                out.push(Finding::new(
                    Severity::Warning,
                    "reflow-unsourced",
                    &at,
                    "limits have no source: treated as assumed",
                ));
            }
            confidence = Confidence::Assumed;
        } else if confidence == Confidence::Datasheet && top.page.is_none() {
            out.push(Finding::new(
                Severity::Info,
                "reflow-no-page",
                &at,
                "datasheet limits give no page",
            ));
        }
        if !assumed_fields.is_empty() {
            out.push(Finding::new(
                Severity::Info,
                "reflow-assumed-field",
                &at,
                format!("{} not given: package default used, assumed", assumed_fields.join(", ")),
            ));
        }
    }
    for (name, v) in [
        ("max_peak", max_peak),
        ("max_time_at_peak", max_time_at_peak),
        ("max_time_above_liquidus", max_time_above_liquidus),
        ("max_ramp_up", max_ramp_up),
        ("max_ramp_down", max_ramp_down),
    ] {
        if v <= 0.0 {
            out.push(Finding::new(
                Severity::Error,
                "reflow-range",
                &at,
                format!("{name} is {v}, it must be above zero"),
            ));
        }
    }
    if max_peak > 0.0 && max_peak < super::AMBIENT_C + 50.0 {
        out.push(Finding::new(
            Severity::Error,
            "reflow-range",
            &at,
            format!("max_peak {max_peak} C is not a plausible soldering limit"),
        ));
    }
    Resolved {
        reference: p.reference.clone(),
        mpn: p.mpn.clone(),
        footprint: p.footprint.clone(),
        family,
        origin,
        limits: Limits {
            max_peak,
            max_time_at_peak,
            max_time_above_liquidus,
            max_ramp_up,
            max_ramp_down,
            msl,
            floor_life_hours,
            max_passes,
            capable,
            termination,
            last_pass: top.last_pass,
        },
        source: top.source.clone(),
        page: top.page.clone(),
        confidence,
        assumed_fields,
        note: top.note.clone(),
        mass_g: top.mass_g,
        local_copper_fill: top.local_copper_fill,
    }
}

pub fn check_design(file: &ReflowFile, parts: &[PartInput], out: &mut Vec<Finding>) {
    let mut seen: Vec<&str> = Vec::new();
    for e in &file.parts {
        if e.refs.is_empty() && e.mpn.is_none() {
            out.push(Finding::new(
                Severity::Error,
                "reflow-entry",
                "parts",
                "a [[parts]] entry needs refs or an mpn",
            ));
        }
        for r in &e.refs {
            if !parts.iter().any(|p| &p.reference == r) {
                out.push(Finding::new(
                    Severity::Error,
                    "reflow-unknown-ref",
                    r,
                    format!("`{r}` is not a part on layout `{}`", file.layout),
                ));
            }
            if seen.contains(&r.as_str()) {
                out.push(Finding::new(
                    Severity::Error,
                    "reflow-duplicate",
                    r,
                    format!("`{r}` has limits in two [[parts]] entries"),
                ));
            }
            seen.push(r);
        }
        if e.refs.is_empty()
            && let Some(m) = &e.mpn
            && !parts.iter().any(|p| p.mpn.as_deref().is_some_and(|x| same_mpn(x, m)))
        {
            out.push(Finding::new(
                Severity::Warning,
                "reflow-unknown-mpn",
                m,
                format!("no part on layout `{}` has mpn `{m}`", file.layout),
            ));
        }
    }
}

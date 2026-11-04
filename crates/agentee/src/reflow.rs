use crate::ops;
use agentee_core::project::Project;
use agentee_core::reflow::inputs::{self, PartInput};
use agentee_core::reflow::solve::{self, Board3, Report};
use agentee_core::reflow::{self, Finding, LimitsFile, ReflowFile, export};
use agentee_core::schematic::Schematic;
use agentee_core::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn find_file(root: &Path, name: &str) -> Result<PathBuf, String> {
    let direct = Path::new(name);
    if name.ends_with(reflow::EXT) && direct.is_file() {
        return Ok(direct.to_path_buf());
    }
    let want = format!("{name}{}", reflow::EXT);
    let mut stack = vec![root.to_path_buf()];
    let mut all = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if p.is_dir() {
                if !n.starts_with('.') && n != "target" {
                    stack.push(p);
                }
            } else if n.ends_with(reflow::EXT) {
                if n == want {
                    return Ok(p);
                }
                all.push(n.trim_end_matches(reflow::EXT).to_string());
            }
        }
    }
    all.sort();
    Err(format!(
        "no {want} under {}{}",
        root.display(),
        if all.is_empty() { String::new() } else { format!(" (found: {})", all.join(", ")) }
    ))
}

fn board_info(p: &Project, board: &str) -> Board3 {
    match p.boards.iter().find(|b| b.name == board) {
        Some(b) => {
            let st = &b.item.stackup;
            let copper = st.copper().map(|(_, l)| l.thickness.to_mm()).sum();
            let mut thickness = st.thickness().to_mm();
            if thickness <= 0.0 {
                thickness = 1.6;
            }
            Board3 { thickness_mm: thickness, copper_mm: copper }
        }
        None => Board3 { thickness_mm: 1.6, copper_mm: 0.07 },
    }
}

fn library_lookup<'a>(
    lib: &'a agentee_parts::library::Library,
) -> impl Fn(&str, &str) -> Option<LimitsFile> + 'a {
    move |mpn: &str, _reference: &str| {
        if mpn.is_empty() {
            return None;
        }
        let last = inputs::last_word(mpn);
        lib.get(mpn).or_else(|| lib.get(last)).and_then(|e| e.part.reflow.clone())
    }
}

pub struct Run {
    pub report: Report,
    pub file: PathBuf,
}

pub fn analyse(root: &Path, name: &str) -> Result<Run, String> {
    let path = find_file(root, name)?;
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = ReflowFile::parse(&src).map_err(|e| format!("{}: {e}", path.display()))?;
    let project = ops::load(root)?;
    let layout = project
        .layouts
        .iter()
        .find(|l| l.name == file.layout)
        .ok_or_else(|| format!("{}: no layout named `{}`", path.display(), file.layout))?;
    let schematic: Option<&Schematic> =
        project.schematics.iter().find(|s| s.name == layout.item.schematic).map(|s| &s.item);
    let lib = crate::library::open(Some(&project.root));
    let lookup = library_lookup(&lib);
    let parts: Vec<PartInput> = inputs::gather(&layout.item, schematic, &lookup);
    let info = board_info(&project, &layout.item.board);
    let report = solve::run(&file, &parts, &info, Vec::new());
    Ok(Run { report, file: path })
}

pub struct Options {
    pub csv: Option<PathBuf>,
    pub plot: Option<PathBuf>,
}

pub fn run(root: &Path, name: &str, o: &Options) -> Result<(Report, Value), String> {
    let r = analyse(root, name)?;
    let mut written = Vec::new();
    if let Some(p) = &o.csv {
        let text = export::csv(&r.report).ok_or("no profile to export: the limits conflict")?;
        std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p.display().to_string());
    }
    if let Some(p) = &o.plot {
        let text = export::svg(&r.report).ok_or("no profile to plot: the limits conflict")?;
        std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p.display().to_string());
    }
    let mut v = serde_json::to_value(&r.report).map_err(|e| e.to_string())?;
    v["file"] = json!(r.file);
    v["written"] = json!(written);
    Ok((r.report, v))
}

pub fn diagnostics(r: &Run) -> Vec<Diagnostic> {
    r.report
        .findings
        .iter()
        .map(|f: &Finding| Diagnostic {
            severity: f.severity,
            file: Some(r.file.clone()),
            item: r.report.reflow.clone(),
            at: f.at.clone(),
            message: f.message.clone(),
            rule: Some(f.rule.clone()),
        })
        .collect()
}

pub fn render(r: &Report) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "reflow {} on layout {}", r.reflow, r.layout);
    let _ = writeln!(
        s,
        "paste {} ({}), liquidus {:.0} C, peak {:.0}-{:.0} C [{}]",
        r.paste.name,
        if r.paste.lead_free { "lead free" } else { "leaded" },
        r.paste.liquidus,
        r.paste.peak[0],
        r.paste.peak[1],
        match r.paste.confidence {
            reflow::Confidence::Datasheet => "datasheet",
            reflow::Confidence::Inferred => "partly inferred",
            reflow::Confidence::Assumed => "assumed",
        }
    );
    let _ = writeln!(s, "\nlimits per part:");
    for p in &r.parts {
        let _ = writeln!(
            s,
            "  {:<5} {:<14} peak {:>3.0} C for {:>2.0} s, MSL {:<2}, {} pass, {:?}  [{:?}{}]",
            p.reference,
            p.mpn.as_deref().map(inputs::last_word).unwrap_or(&p.footprint),
            p.limits.max_peak,
            p.limits.max_time_at_peak,
            p.limits.msl,
            p.limits.max_passes,
            p.limits.capable,
            p.confidence,
            p.source.as_deref().map(|x| format!(", {x}")).unwrap_or_default()
        );
    }
    if let Some(p) = &r.profile {
        let _ =
            writeln!(s, "\nprofile ({}):", if p.feasible { "feasible" } else { "NOT feasible" });
        for st in &p.stages {
            let _ = writeln!(
                s,
                "  {:<13} {:>5.0} s -> {:>5.0} s   {:>5.0} -> {:>5.0} C   {:+.2} C/s",
                st.name, st.start_s, st.end_s, st.start_c, st.end_c, st.rate_c_s
            );
        }
        let _ = writeln!(
            s,
            "  limiter {} (max {:.0} C, margin {:.1} C); coldest joint {} peaks {:.1} C, needs {:.0} C (margin {:.1} C); hot to cold spread {:.1} C; tolerance +/-{:.1} C",
            p.limiter,
            p.limiter_limit_c,
            p.limiter_margin_c,
            p.coldest_joint,
            p.coldest_joint_peak_c,
            p.required_joint_peak_c,
            p.coldest_margin_c,
            p.hot_cold_spread_c,
            p.tolerance_c
        );
        let _ = writeln!(s, "  per part:");
        for x in &p.parts {
            let _ = writeln!(
                s,
                "    {:<5} peak {:>5.1} C (max {:>3.0}, hot margin {:>5.1}, cold margin {:>5.1}), above liquidus {:>4.0} s, within 5 C of peak {:>3.0} s, lag {:>4.1} s",
                x.reference,
                x.peak_c,
                x.limit_peak_c,
                x.hot_margin_c,
                x.cold_margin_c,
                x.time_above_liquidus_s,
                x.time_at_peak_s,
                x.tau_s
            );
        }
    }
    if let Some(c) = &r.conflict {
        let _ = writeln!(s, "\nCONFLICT: {}", c.message);
        let _ = writeln!(s, "  conflicting parts: {}", c.parts.join(", "));
        for o in &c.options {
            let _ = writeln!(s, "  option, {}: {}", o.action, o.detail);
        }
    }
    let pl = &r.passes;
    let _ = writeln!(
        s,
        "\npasses: {}{}",
        pl.passes,
        match pl.first_side {
            Some(f) if pl.double_sided => format!(
                ", double sided, {} side first{}",
                f.label(),
                if pl.chosen_automatically { " (chosen by the checker)" } else { "" }
            ),
            _ => ", single sided".to_string(),
        }
    );
    for e in &pl.exposures {
        let _ = writeln!(
            s,
            "  {} MSL {} pass {}: floor life {} h{}",
            e.reference,
            e.msl,
            e.pass,
            e.floor_life_hours,
            e.remaining_hours.map(|h| format!(", {h:.0} h left")).unwrap_or_default()
        );
    }
    for a in &pl.after_reflow {
        let _ = writeln!(s, "  {a}");
    }
    if let Some(o) = &r.oven {
        let _ = writeln!(s, "\noven ({:?}) setpoints:", o.kind);
        for p in &o.setpoints {
            let _ = writeln!(s, "  t {:>4.0} s  {:>4.0} C  {}", p.at_s, p.temperature_c, p.action);
        }
        for z in &o.zones {
            let _ = writeln!(
                s,
                "  zone {} {:.0} mm: {:.0} C, {:.0} s",
                z.zone, z.length_mm, z.temperature_c, z.dwell_s
            );
        }
        if let Some(c) = o.conveyor_mm_min {
            let _ = writeln!(s, "  conveyor {c:.0} mm/min");
        }
        let _ = writeln!(s, "  tolerance: {}", o.tolerance.note);
        for a in &o.advice {
            let _ = writeln!(s, "  {a}");
        }
    }
    if !r.thermocouples.is_empty() {
        let _ = writeln!(s, "\nthermocouples:");
        for t in &r.thermocouples {
            let _ = writeln!(s, "  {}: {}", t.reference, t.why);
        }
    }
    let _ = writeln!(s, "\nfindings:");
    if r.findings.is_empty() {
        let _ = writeln!(s, "  none");
    }
    for f in &r.findings {
        let sev = match f.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let _ = writeln!(s, "  {sev} [{}] {}: {}", f.rule, f.at, f.message);
    }
    let _ = writeln!(s, "\n{}", if r.ok { "ok" } else { "failed" });
    s
}

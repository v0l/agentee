use agentee_core::diag::Severity;
use agentee_core::footprint::Mount;
use agentee_core::reflow::inputs::{PartInput, Side};
use agentee_core::reflow::solve::{self, Board3, Report};
use agentee_core::reflow::{Capable, Confidence, LimitsFile, ReflowFile, export};

fn part(reference: &str, footprint: &str, side: Side, body: [f64; 3], pad: f64) -> PartInput {
    PartInput {
        reference: reference.into(),
        value: "x".into(),
        mpn: None,
        footprint: footprint.into(),
        mount: Mount::Smd,
        side,
        body: Some(body),
        footprint_mm: [body[0], body[1]],
        pad_area_mm2: pad,
        library: None,
    }
}

fn small(r: &str) -> PartInput {
    part(r, "R_0402_1005Metric", Side::Top, [1.0, 0.5, 0.35], 0.5)
}

fn bga(r: &str) -> PartInput {
    part(r, "BGA-256_17x17mm", Side::Top, [17.0, 17.0, 1.4], 60.0)
}

fn file(extra: &str) -> ReflowFile {
    file_peak(230.0, extra)
}

fn file_peak(low: f64, extra: &str) -> ReflowFile {
    ReflowFile::parse(&format!("name = \"t\"\nlayout = \"t\"\n[paste]\nalloy = \"SAC305\"\nsource = \"paste.pdf\"\npeak = [{low}, 249]\npreheat = [150, 200]\nsoak_time = [60, 90]\nmax_ramp_up = 2.0\ntime_above_liquidus = [30, 60]\ncooling = [2, 4]\nflux_activation = [150, 200]\n{extra}")).unwrap()
}

fn board() -> Board3 {
    Board3 { thickness_mm: 1.6, copper_mm: 0.07 }
}

fn go(f: &ReflowFile, parts: &[PartInput]) -> Report {
    solve::run(f, parts, &board(), Vec::new())
}

fn entry(refs: &str, peak: f64, extra: &str) -> String {
    format!(
        "[[parts]]\nrefs = [{refs}]\nmax_peak = {peak}\nmax_time_at_peak = 30\nmax_passes = 3\nmsl = 1\ncapable = \"reflow\"\nsource = \"ds.pdf\"\npage = \"4\"\nconfidence = \"datasheet\"\n{extra}\n"
    )
}

fn has(r: &Report, rule: &str) -> bool {
    r.findings.iter().any(|f| f.rule == rule)
}

#[test]
fn two_parts_with_different_peaks_give_the_lower_one_as_limiter() {
    let f = file(&(entry("\"R1\"", 260.0, "") + &entry("\"R2\"", 245.0, "")));
    let r = go(&f, &[small("R1"), small("R2")]);
    let p = r.profile.expect("profile");
    assert!(p.feasible, "{:?}", r.findings);
    assert_eq!(p.limiter, "R2");
    assert_eq!(p.limiter_limit_c, 245.0);
    assert!(p.limiter_margin_c >= 0.0 && p.limiter_margin_c < 10.0);
    assert!(p.coldest_joint_peak_c >= r.paste.peak[0]);
}

#[test]
fn peak_clears_the_liquidus_and_the_time_above_liquidus_is_inside_the_window() {
    let f = file(&entry("\"R1\"", 260.0, ""));
    let r = go(&f, &[small("R1")]);
    let p = r.profile.expect("profile");
    assert!(p.coldest_joint_peak_c >= r.paste.liquidus + 10.0);
    for x in &p.parts {
        assert!(x.time_above_liquidus_s >= 30.0 && x.time_above_liquidus_s <= 60.0, "{x:?}");
        assert!(x.max_ramp_up_c_s <= 3.0 + 1e-6);
    }
}

#[test]
fn a_part_ramp_limit_slows_the_whole_profile() {
    let f = file(&entry("\"R1\"", 260.0, "max_ramp_up = 1.0\nmax_ramp_down = 3.0"));
    let r = go(&f, &[small("R1")]);
    let p = r.profile.expect("profile");
    assert!((p.ramp_up_c_s - 1.0).abs() < 1e-9);
    assert!((p.ramp_down_c_s - 3.0).abs() < 1e-9);
    assert!(p.parts[0].max_ramp_up_c_s <= 1.0 + 1e-6);
}

#[test]
fn an_empty_intersection_names_the_conflicting_parts_and_offers_options() {
    let f = file_peak(238.0, &(entry("\"U1\"", 235.0, "") + &entry("\"R1\"", 260.0, "")));
    let r = go(&f, &[bga("U1"), small("R1")]);
    let c = r.conflict.clone().expect("conflict");
    assert!(c.parts.iter().any(|p| p == "U1"), "{c:?}");
    assert!(c.message.contains("no profile meets every limit"));
    assert!(c.options.iter().any(|o| o.action.contains("low temperature")));
    assert!(c.options.iter().any(|o| o.action.contains("swap U1")));
    assert!(
        !c.options.iter().any(|o| o.action.contains("hand or selective solder U1")),
        "a BGA cannot be hand soldered"
    );
    assert!(!r.ok);
    assert!(has(&r, "reflow-conflict"));
}

#[test]
fn low_temperature_paste_resolves_the_235c_conflict() {
    let src = "name = \"t\"\nlayout = \"t\"\n[paste]\nalloy = \"Sn42Bi58\"\nsource = \"bi.pdf\"\npeak = [160, 190]\npreheat = [90, 120]\nsoak_time = [60, 90]\nmax_ramp_up = 2.0\ntime_above_liquidus = [30, 90]\ncooling = [2, 4]\nflux_activation = [90, 120]\n".to_string() + &entry("\"U1\"", 235.0, "") + &entry("\"R1\"", 260.0, "");
    let r = go(&ReflowFile::parse(&src).unwrap(), &[bga("U1"), small("R1")]);
    assert!(r.profile.expect("profile").feasible, "{:?}", r.findings);
}

#[test]
fn a_large_bga_and_a_0402_reach_different_temperatures_and_both_stay_inside() {
    let f = file(&(entry("\"U1\"", 250.0, "") + &entry("\"R1\"", 260.0, "")));
    let r = go(&f, &[bga("U1"), small("R1")]);
    let p = r.profile.expect("profile");
    let u1 = p.parts.iter().find(|x| x.reference == "U1").unwrap();
    let r1 = p.parts.iter().find(|x| x.reference == "R1").unwrap();
    assert!(u1.tau_s > r1.tau_s * 5.0, "{u1:?} {r1:?}");
    assert!(r1.peak_c > u1.peak_c + 2.0, "small part runs hotter: {u1:?} {r1:?}");
    assert!(u1.peak_c >= p.required_joint_peak_c, "cold spot reaches the minimum");
    assert!(r1.peak_c <= 260.0 && r1.peak_c <= r.paste.peak[1], "hot spot under its max");
    assert_eq!(p.coldest_joint, "U1");
    assert!(p.hot_cold_spread_c > 2.0);
}

#[test]
fn heavy_part_on_the_first_side_hangs_so_the_checker_puts_it_on_the_second_pass() {
    let heavy = part("U1", "BGA-256_17x17mm", Side::Bottom, [17.0, 17.0, 1.4], 10.0);
    let r1 = small("R1");
    let f = file(&(entry("\"U1\"", 260.0, "") + &entry("\"R1\"", 260.0, "")));
    let r = go(&f, &[heavy, r1]);
    assert!(r.passes.double_sided);
    assert_eq!(r.passes.first_side, Some(Side::Top), "{:?}", r.passes);
    assert!(r.passes.chosen_automatically);
    assert!(!has(&r, "reflow-hangs"), "{:?}", r.findings);
}

#[test]
fn a_heavy_part_forced_onto_the_first_side_is_an_error() {
    let heavy = part("U1", "BGA-256_17x17mm", Side::Bottom, [17.0, 17.0, 1.4], 10.0);
    let f = file(
        &("[assembly]\nfirst_side = \"bottom\"\n".to_string()
            + &entry("\"U1\"", 260.0, "")
            + &entry("\"R1\"", 260.0, "")),
    );
    let r = go(&f, &[heavy, small("R1")]);
    assert!(has(&r, "reflow-hangs"));
    assert!(!r.ok);
}

#[test]
fn exceeding_a_part_pass_count_is_an_error() {
    let a = part("U1", "SOIC-8", Side::Top, [4.9, 3.9, 1.5], 5.0);
    let b = part("U2", "SOIC-8", Side::Bottom, [4.9, 3.9, 1.5], 5.0);
    let f = file(
        &("[assembly]\nfirst_side = \"top\"\n".to_string()
            + &entry("\"U1\"", 260.0, "").replace("max_passes = 3", "max_passes = 1")
            + &entry("\"U2\"", 260.0, "")),
    );
    let r = go(&f, &[a, b]);
    assert!(has(&r, "reflow-passes"));
    assert!(!r.ok);
}

#[test]
fn a_last_pass_part_on_the_first_side_is_an_error() {
    let a = part("U1", "SOIC-8", Side::Top, [4.9, 3.9, 1.5], 5.0);
    let b = part("U2", "SOIC-8", Side::Bottom, [4.9, 3.9, 1.5], 5.0);
    let f = file(
        &("[assembly]\nfirst_side = \"top\"\n".to_string()
            + &entry("\"U1\"", 260.0, "last_pass = true")
            + &entry("\"U2\"", 260.0, "")),
    );
    let r = go(&f, &[a, b]);
    assert!(has(&r, "reflow-last-pass"));
}

#[test]
fn floor_life_exceeded_between_passes_is_an_error() {
    let a = part("U1", "SOIC-8", Side::Top, [4.9, 3.9, 1.5], 5.0);
    let b = part("U2", "SOIC-8", Side::Bottom, [4.9, 3.9, 1.5], 5.0);
    let src = "[assembly]\nfirst_side = \"top\"\nhours_before_first_pass = 2\nhours_between_passes = 200\n".to_string() + &entry("\"U1\"", 260.0, "").replace("msl = 1", "msl = 3") + &entry("\"U2\"", 260.0, "").replace("msl = 1", "msl = 3");
    let r = go(&file(&src), &[a, b]);
    assert!(has(&r, "reflow-floor-life"), "{:?}", r.findings);
    assert!(r.passes.exposures.iter().any(|e| !e.ok));
}

#[test]
fn a_limit_with_no_source_is_flagged_assumed() {
    let f = file(
        "[[parts]]\nrefs = [\"R1\"]\nmax_peak = 260\nmax_time_at_peak = 30\nmax_passes = 3\nmsl = 1\ncapable = \"reflow\"\nconfidence = \"datasheet\"\n",
    );
    let r = go(&f, &[small("R1")]);
    assert!(has(&r, "reflow-unsourced"));
    assert_eq!(r.parts[0].confidence, Confidence::Assumed);
}

#[test]
fn an_unknown_part_falls_back_to_the_package_default_with_a_warning() {
    let r = go(&file(""), &[small("R1"), part("U1", "SOIC-8", Side::Top, [4.9, 3.9, 1.5], 5.0)]);
    let w: Vec<_> = r
        .findings
        .iter()
        .filter(|f| f.rule == "reflow-default" && f.severity == Severity::Warning)
        .collect();
    assert_eq!(w.len(), 2);
    assert!(r.parts.iter().all(|p| p.confidence == Confidence::Assumed));
    assert!(r.parts.iter().find(|p| p.reference == "U1").unwrap().limits.msl == "3");
}

#[test]
fn package_defaults_follow_jstd020_thickness_and_volume() {
    use agentee_core::reflow::defaults::pbfree_classification_c as c;
    assert_eq!(c(1.0, 100.0), 260.0);
    assert_eq!(c(2.0, 1000.0), 250.0);
    assert_eq!(c(3.0, 3000.0), 245.0);
}

#[test]
fn through_hole_parts_are_left_out_of_the_oven_profile() {
    let mut j = small("J3");
    j.mount = Mount::Tht;
    j.footprint = "PinHeader_1x02_P2.54mm_Vertical".into();
    let r = go(&file(&entry("\"R1\"", 260.0, "")), &[small("R1"), j]);
    let p = r.parts.iter().find(|p| p.reference == "J3").unwrap();
    assert_eq!(p.limits.capable, Capable::Selective);
    assert!(r.passes.after_reflow.iter().any(|a| a.starts_with("J3")));
    assert_eq!(r.profile.unwrap().parts.len(), 1);
}

#[test]
fn leaded_parts_on_lead_free_paste_and_the_reverse_are_flagged() {
    let mut u = bga("U1");
    u.library = None;
    let f = file(&entry("\"U1\"", 260.0, "termination = \"tin_lead\""));
    let r = go(&f, &[u]);
    assert!(
        r.findings.iter().any(|f| f.rule == "reflow-alloy-mix" && f.severity == Severity::Error)
    );
}

#[test]
fn unknown_refs_and_alloys_are_errors() {
    let r = go(&file(&entry("\"Z9\"", 260.0, "")), &[small("R1")]);
    assert!(has(&r, "reflow-unknown-ref"));
    let bad =
        ReflowFile::parse("name = \"t\"\nlayout = \"t\"\n[paste]\nalloy = \"mystery\"\n").unwrap();
    let r = go(&bad, &[small("R1")]);
    assert!(has(&r, "reflow-paste") && !r.ok);
}

#[test]
fn csv_and_plot_carry_the_limiter_label() {
    let f = file(&(entry("\"R1\"", 245.0, "") + &entry("\"R2\"", 260.0, "")));
    let r = go(&f, &[small("R1"), small("R2")]);
    let csv = export::csv(&r).unwrap();
    assert!(csv.contains("time_s,oven_target_c") && csv.contains("limiter R1"));
    assert!(csv.lines().count() > 100);
    let svg = export::svg(&r).unwrap();
    assert!(
        svg.starts_with("<svg") && svg.contains("limiter R1") && svg.contains("liquidus 220 C")
    );
}

#[test]
fn an_oven_below_the_needed_air_temperature_is_an_error() {
    let f = file(&("[oven]\nmax_temp = 200\n".to_string() + &entry("\"R1\"", 260.0, "")));
    let r = go(&f, &[small("R1")]);
    assert!(has(&r, "reflow-oven-max"));
}

#[test]
fn zones_turn_the_curve_into_zone_temperatures_and_belt_speed() {
    let f = file(&("[oven]\nkind = \"zones\"\nconveyor_speed = 300\nzones = [{ name = \"z1\", length = 300 }, { name = \"z2\", length = 300 }, { name = \"z3\", length = 300 }, { name = \"z4\", length = 300 }]\n".to_string() + &entry("\"R1\"", 260.0, "")));
    let r = go(&f, &[small("R1")]);
    let o = r.oven.unwrap();
    assert_eq!(o.zones.len(), 4);
    let hottest = o.zones.iter().map(|z| z.temperature_c).fold(0.0, f64::max);
    assert!(o.zones[0].temperature_c < hottest && hottest > 200.0, "{:?}", o.zones);
    assert!(o.conveyor_mm_min.is_some());
}

#[test]
fn library_limits_are_used_and_the_design_overrides_them() {
    let lib = LimitsFile {
        max_peak: Some(245.0),
        max_time_at_peak: Some(20.0),
        max_ramp_up: Some(3.0),
        max_ramp_down: Some(6.0),
        max_time_above_liquidus: Some(100.0),
        msl: Some("1".into()),
        max_passes: Some(2),
        capable: Some(Capable::Reflow),
        source: Some("lib.pdf".into()),
        confidence: Some(Confidence::Datasheet),
        ..Default::default()
    };
    let mut p = small("R1");
    p.library = Some(lib);
    let r = go(&file(""), &[p.clone()]);
    assert_eq!(r.parts[0].limits.max_peak, 245.0);
    assert!(!has(&r, "reflow-default"));
    let r = go(&file(&entry("\"R1\"", 255.0, "")), &[p]);
    assert_eq!(r.parts[0].limits.max_peak, 255.0);
}

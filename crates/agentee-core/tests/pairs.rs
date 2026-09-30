use agentee_core::Project;
use agentee_core::diag::Severity;
use std::path::Path;

fn project(extra_pcb: &str, tracks: &str) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-pairs-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    std::fs::copy(
        lna.join("footprints/R_0402_1005Metric.fp.toml"),
        dir.join("footprints/R_0402_1005Metric.fp.toml"),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        r#"name = "t"
fab = "jlcpcb"
[outline]
size = [30, 10]
[stackup]
preset = "jlcpcb-4l-1.6mm-7628"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.15mm"
[[netclasses]]
name = "USB"
track_width = "0.15mm"
clearance = "0.15mm"
diff_gap = "0.15mm"
impedance = "90ohm"
max_skew = "0.1mm"
layers = ["F.Cu"]
"#,
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    for (i, r) in ["R1", "R2", "R3", "R4"].iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"0\"\nat = [{}, 20]\n",
            10.16 * (i + 1) as f64
        );
    }
    sch += "\n[[nets]]\nname = \"USB_DP\"\nclass = \"USB\"\npins = [\"R1.1\", \"R3.1\"]\n";
    sch += "\n[[nets]]\nname = \"USB_DN\"\nclass = \"USB\"\npins = [\"R2.1\", \"R4.1\"]\n";
    sch += "\n[[nets]]\nname = \"A\"\npins = [\"R1.2\"]\n\n[[nets]]\nname = \"B\"\npins = [\"R2.2\"]\n";
    sch += "\n[[nets]]\nname = \"C\"\npins = [\"R3.2\"]\n\n[[nets]]\nname = \"D\"\npins = [\"R4.2\"]\n";
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    let pcb = format!(
        r#"name = "t"
board = "t"
schematic = "t"

[[footprints]]
ref = "R1"
at = [3, 4]

[[footprints]]
ref = "R2"
at = [3, 6]

[[footprints]]
ref = "R3"
at = [27, 4]

[[footprints]]
ref = "R4"
at = [27, 6]
{tracks}
{extra_pcb}
"#
    );
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    Project::load(&dir).unwrap()
}

fn messages(p: &Project) -> Vec<(Severity, String)> {
    p.layouts[0].diags.iter().map(|d| (d.severity, d.message.clone())).collect()
}

#[test]
fn a_well_routed_pair_passes_and_a_skewed_one_is_flagged() {
    let good = r#"
[[tracks]]
net = "USB_DP"
layer = "F.Cu"
points = [[2.49, 4], [2.49, 4.85], [27.49, 4.85], [27.49, 4]]

[[tracks]]
net = "USB_DN"
layer = "F.Cu"
points = [[2.49, 6], [2.49, 5.15], [27.49, 5.15], [27.49, 6]]
"#;
    let p = project("", good);
    let m = messages(&p);
    assert!(!m.iter().any(|(s, t)| *s == Severity::Error && t.contains("skew")), "{m:?}");
    assert!(!m.iter().any(|(_, t)| t.contains("gap, the class")), "{m:?}");
    assert_eq!(p.layouts[0].item.pairs.len(), 1);
    let pair = &p.layouts[0].item.pairs[0];
    assert!(pair.coupled_mm > 24.9, "{}", pair.coupled_mm);

    let skewed = good.replace(
        "[27.49, 5.15], [27.49, 6]",
        "[27.49, 5.15], [27.49, 5.5], [27.2, 5.5], [27.2, 6], [27.49, 6]",
    );
    let p = project("", &skewed);
    let m = messages(&p);
    assert!(
        m.iter().any(|(s, t)| *s == Severity::Error
            && t.contains("skew")
            && t.contains("lengthen USB_DP")),
        "{m:?}"
    );

    let wide = good.replace("5.15", "5.35");
    let p = project("", &wide);
    assert!(
        messages(&p).iter().any(|(s, t)| *s == Severity::Error && t.contains("0.350 mm gap")),
        "{:?}",
        messages(&p)
    );
}

#[test]
fn match_groups_report_what_to_add() {
    let tracks = r#"
[[tracks]]
net = "USB_DP"
layer = "F.Cu"
points = [[2.49, 4], [2.49, 4.85], [27.49, 4.85], [27.49, 4]]

[[tracks]]
net = "USB_DN"
layer = "F.Cu"
points = [[2.49, 6], [2.49, 5.15], [27.49, 5.15], [27.49, 6]]
"#;
    let group = r#"
[[match_groups]]
name = "usb"
nets = ["USB_D?"]
tolerance = "0.05mm"
target = "27mm"
"#;
    let p = project(group, tracks);
    let m = messages(&p);
    assert!(m.iter().any(|(_, t)| t.contains("USB_DP is 26.700 mm, 0.300 mm short of the 27.000 mm target")), "{m:?}");
}

const PAIR: &str = r#"
[[tracks]]
net = "USB_DP"
layer = "F.Cu"
points = [[2.49, 4], [2.49, 4.85], [27.49, 4.85], [27.49, 4]]

[[tracks]]
net = "USB_DN"
layer = "F.Cu"
points = [[2.49, 6], [2.49, 5.15], [27.49, 5.15], [27.49, 6]]
"#;

fn interface_errors(p: &Project) -> Vec<String> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error && d.at.starts_with("interface"))
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn an_interface_wants_a_plane_under_the_pair() {
    let spec = r#"
[[interfaces]]
name = "hs"
preset = "usb2-hs"
nets = ["USB_D?"]
"#;
    let e = interface_errors(&project(spec, PAIR));
    assert!(e.iter().any(|t| t.contains("USB_DP runs") && t.contains("no GND plane")), "{e:?}");
}

#[test]
fn an_interface_checks_pair_skew_in_time() {
    let spec = r#"
[[interfaces]]
name = "hs"
nets = ["USB_D?"]
differential = true
max_skew = "0.5ps"
"#;
    let skewed = PAIR.replace(
        "[27.49, 5.15], [27.49, 6]",
        "[27.49, 5.15], [27.49, 5.5], [26.0, 5.5], [26.0, 6], [27.49, 6]",
    );
    let e = interface_errors(&project(spec, &skewed));
    assert!(e.iter().any(|t| t.contains("skew") && t.contains("0.50 ps")), "{e:?}");
    assert!(interface_errors(&project(spec, PAIR)).is_empty());
    let p = project(spec, &skewed);
    let iface = &p.layouts[0].item.interfaces[0];
    assert_eq!(iface.lanes.len(), 2);
    assert!((iface.pairs[0].2 + 2.98).abs() < 0.01, "{:?}", iface.pairs);
}

#[test]
fn a_bus_line_outside_its_clock_window_is_flagged() {
    let spec = r#"
[[interfaces]]
name = "bus"
nets = ["USB_D?"]
differential = false
clock = "USB_DP"
clock_window = ["-1ps", "1ps"]
"#;
    let late = PAIR.replace(
        "[27.49, 5.15], [27.49, 6]",
        "[27.49, 5.15], [27.49, 5.5], [26.0, 5.5], [26.0, 6], [27.49, 6]",
    );
    let e = interface_errors(&project(spec, &late));
    assert!(
        e.iter().any(|t| t.contains("USB_DN arrives") && t.contains("after the clock")),
        "{e:?}"
    );
    assert!(interface_errors(&project(spec, PAIR)).is_empty());
}

#[test]
fn same_net_tracks_on_top_of_each_other_are_an_error() {
    let doubled = format!(
        "{PAIR}\n[[tracks]]\nnet = \"USB_DP\"\nlayer = \"F.Cu\"\npoints = [[5.0, 4.85], [12.0, 4.85]]\n"
    );
    let m = messages(&project("", &doubled));
    assert!(m.iter().any(|(s, t)| *s == Severity::Error && t.contains("runs on top of")), "{m:?}");
    let m = messages(&project("", PAIR));
    assert!(!m.iter().any(|(_, t)| t.contains("runs on top of")), "{m:?}");
    let folded = PAIR.replace(
        "[[2.49, 4], [2.49, 4.85], [27.49, 4.85], [27.49, 4]]",
        "[[2.49, 4], [2.49, 4.85], [20.0, 4.85], [15.0, 4.7], [27.49, 4.7], [27.49, 4]]",
    );
    let m = messages(&project("", &folded));
    assert!(m.iter().any(|(_, t)| t.contains("turns back")), "{m:?}");
}

#[test]
fn tuning_a_pair_leg_bumps_where_the_pair_is_uncoupled() {
    let legs = [
        ("USB_DP", "[[2.49, 4], [2.49, 1.5], [7, 1.5], [7, 4.85], [27.49, 4.85], [27.49, 4]]"),
        ("USB_DN", "[[2.49, 6], [2.49, 5.15], [24, 5.15], [24, 8.65], [27.49, 8.65], [27.49, 6]]"),
    ];
    let tracks = |pts: &[String]| {
        legs.iter()
            .zip(pts)
            .map(|((net, _), p)| {
                format!("\n[[tracks]]\nnet = \"{net}\"\nlayer = \"F.Cu\"\npoints = {p}\n")
            })
            .collect::<String>()
    };
    let before: Vec<String> = legs.iter().map(|l| l.1.to_string()).collect();
    let p = project("", &tracks(&before));
    let m = messages(&p);
    assert!(m.iter().any(|(s, t)| *s == Severity::Error && t.contains("lengthen USB_DP")), "{m:?}");
    let opts = agentee_core::tune::TuneOptions::default();
    let r = agentee_core::tune::tune(&p.layouts[0].item, &p.boards[0].item, &opts).unwrap();
    assert!(r.failed.is_empty() && r.tuned.len() == 1, "{:?}", r.failed);
    let mut after = before.clone();
    for e in &r.edits {
        let pts: Vec<String> = e.points.iter().map(|q| format!("[{}, {}]", q[0], q[1])).collect();
        after[e.track] = format!("[{}]", pts.join(", "));
    }
    assert!(after[1] == before[1], "the long leg stays");
    let added: Vec<&[f64; 2]> = r.edits[0]
        .points
        .iter()
        .filter(|q| !before[0].contains(&format!("[{}, {}]", q[0], q[1])))
        .collect();
    assert!(!added.is_empty() && added.iter().all(|q| q[0] <= 7.0 + 1e-9), "{added:?}");
    let p = project("", &tracks(&after));
    let m = messages(&p);
    assert!(!m.iter().any(|(_, t)| t.contains("gap, the class")), "{m:?}");
    assert!(!m.iter().any(|(s, t)| *s == Severity::Error && t.contains("skew")), "{m:?}");
}

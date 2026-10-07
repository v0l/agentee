use agentee_core::Project;
use agentee_core::diag::Severity;

fn symbol(name: &str, pins: usize) -> String {
    let mut s = format!("name = \"{name}\"\nreference = \"U\"\n");
    for i in 1..=pins {
        s += &format!(
            "\n[[pins]]\nnumber = \"{i}\"\ntype = \"passive\"\nat = [0.0, {}]\nside = \"left\"\nlength = 1.27\nunit = 1\n",
            i as f64 * 2.54
        );
    }
    s
}

fn row(name: &str, xs: &[f64], extra: &str) -> String {
    let mut s = format!("name = \"{name}\"\nmount = \"smd\"\n{extra}");
    for (i, x) in xs.iter().enumerate() {
        s += &format!(
            "\n[[pads]]\nnumber = \"{}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [{x}, 0.0]\nsize = [1.0, 1.0]\n",
            i + 1
        );
    }
    s
}

struct Board<'a> {
    rules: &'a str,
    cutouts: &'a str,
    footprints: &'a [(&'a str, String)],
    parts: &'a [(&'a str, &'a str, usize)],
    nets: &'a [(&'a str, &'a str, &'a [&'a str])],
    pcb: &'a str,
}

fn load(b: &Board) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-iso-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    for (name, text) in b.footprints {
        std::fs::write(dir.join(format!("footprints/{name}.fp.toml")), text).unwrap();
    }
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    for (i, (r, fp, pins)) in b.parts.iter().enumerate() {
        let sym = format!("S{pins}");
        std::fs::write(dir.join(format!("symbols/{sym}.sym.toml")), symbol(&sym, *pins)).unwrap();
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"{sym}\"\nvalue = \"x\"\nfootprint = \"{fp}\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
    }
    for (name, class, pins) in b.nets {
        let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
        sch += &format!(
            "\n[[nets]]\nname = \"{name}\"\nclass = \"{class}\"\npins = [{}]\n",
            pins.join(", ")
        );
    }
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            r#"name = "t"
fab = "jlcpcb"
[outline]
size = [40, 20]
{}
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.15mm"
via = "std"
[[netclasses]]
name = "HV"
track_width = "0.5mm"
clearance = "1.5mm"
via = "std"
{}
"#,
            b.cutouts, b.rules
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        format!("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n{}", b.pcb),
    )
    .unwrap();
    Project::load(&dir).unwrap()
}

fn hits(p: &Project, rule: &str) -> Vec<String> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some(rule) && d.severity >= Severity::Warning)
        .map(|d| d.message.clone())
        .collect()
}

const DOMAINS: &str = r#"
[[domains]]
name = "primary"
nets = ["P*"]
[[domains]]
name = "secondary"
nets = ["S*"]
"#;

fn barrier(clearance: &str, creepage: &str) -> String {
    let mut s = format!("{DOMAINS}\n[[barriers]]\nbetween = [\"primary\", \"secondary\"]\n");
    if !clearance.is_empty() {
        s += &format!("clearance = \"{clearance}\"\n");
    }
    if !creepage.is_empty() {
        s += &format!("creepage = \"{creepage}\"\n");
    }
    s
}

fn at(r: &str, x: f64, y: f64) -> String {
    format!("\n[[footprints]]\nref = \"{r}\"\nat = [{x}, {y}]\n")
}

#[test]
fn pads_of_one_footprint_keep_the_clearance_of_an_isolated_class() {
    let fps = [("TRI", row("TRI", &[-2.0, 0.0, 2.0], ""))];
    let nets: &[(&str, &str, &[&str])] = &[
        ("P_DRAIN", "HV", &["Q1.1"]),
        ("P_GATE", "HV", &["Q1.2"]),
        ("P_SRC", "Default", &["Q1.3"]),
    ];
    let pcb = at("Q1", 10.0, 10.0);
    let mut b = Board {
        rules: "",
        cutouts: "",
        footprints: &fps,
        parts: &[("Q1", "TRI", 3)],
        nets,
        pcb: &pcb,
    };
    assert!(hits(&load(&b), "clearance").is_empty());
    b.rules = DOMAINS;
    let e = hits(&load(&b), "clearance");
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(e.iter().any(|m| m.contains("Q1.1 is 1mm from Q1.2, needs 1.5mm")), "{e:?}");
}

#[test]
fn a_net_in_two_domains_is_an_error_and_a_net_in_none_a_warning() {
    let fps = [("TWO", row("TWO", &[-2.0, 2.0], ""))];
    let nets: &[(&str, &str, &[&str])] =
        &[("PS_A", "Default", &["R1.1"]), ("X", "Default", &["R1.2"])];
    let pcb = at("R1", 10.0, 10.0);
    let rules = DOMAINS.replace("nets = [\"S*\"]", "nets = [\"S*\", \"*_A\"]");
    let b = Board {
        rules: &rules,
        cutouts: "",
        footprints: &fps,
        parts: &[("R1", "TWO", 2)],
        nets,
        pcb: &pcb,
    };
    let p = load(&b);
    let e = hits(&p, "isolation-domain");
    assert!(
        e.len() == 1 && e[0].contains("PS_A (class Default) is in domains primary and secondary"),
        "{e:?}"
    );
    let w = hits(&p, "isolation-unassigned");
    assert!(w.len() == 1 && w[0].contains("1 nets") && w[0].contains("X"), "{w:?}");
}

fn across(gap: f64) -> (Vec<(&'static str, String)>, String) {
    let fps = vec![("ONE", row("ONE", &[0.0], ""))];
    let pcb = format!("{}{}", at("U1", 10.0, 10.0), at("U2", 10.0 + 1.0 + gap, 10.0));
    (fps, pcb)
}

const ACROSS: &[(&str, &str, &[&str])] =
    &[("P_HOT", "Default", &["U1.1"]), ("S_COLD", "Default", &["U2.1"])];

#[test]
fn a_barrier_holds_its_clearance_between_domains() {
    let (fps, pcb) = across(2.0);
    let rules = barrier("3mm", "");
    let b = Board {
        rules: &rules,
        cutouts: "",
        footprints: &fps,
        parts: &[("U1", "ONE", 1), ("U2", "ONE", 1)],
        nets: ACROSS,
        pcb: &pcb,
    };
    let e = hits(&load(&b), "isolation-clearance");
    assert!(
        e.len() == 1
            && e[0].contains("U1.1 is 2mm from U2.1, the primary-secondary barrier needs 3mm"),
        "{e:?}"
    );
    let (fps, pcb) = across(3.2);
    let b = Board { footprints: &fps, pcb: &pcb, ..b };
    assert!(hits(&load(&b), "isolation-clearance").is_empty());
}

#[test]
fn creepage_goes_around_a_slot_and_bridges_a_narrow_groove() {
    let (fps, pcb) = across(4.0);
    let rules = barrier("", "6mm");
    let parts: &[(&str, &str, usize)] = &[("U1", "ONE", 1), ("U2", "ONE", 1)];
    let plain =
        Board { rules: &rules, cutouts: "", footprints: &fps, parts, nets: ACROSS, pcb: &pcb };
    let e = hits(&load(&plain), "creepage");
    assert!(e.len() == 1 && e[0].contains("U1.1 is 4mm from U2.1 along the surface"), "{e:?}");
    let slot = "[[outline.cutouts]]\norigin = [12.5, 6.0]\nsize = [2.0, 8.0]\n";
    let b = Board { cutouts: slot, ..plain };
    assert!(hits(&load(&b), "creepage").is_empty(), "{:?}", hits(&load(&b), "creepage"));
    let short = "[[outline.cutouts]]\norigin = [12.5, 9.0]\nsize = [2.0, 2.0]\n";
    let b = Board { cutouts: short, ..plain };
    let e = hits(&load(&b), "creepage");
    assert!(e.len() == 1 && e[0].contains("around a slot (4mm straight)"), "{e:?}");
    let groove = "[[outline.cutouts]]\norigin = [13.25, 6.0]\nsize = [0.5, 8.0]\n";
    let b = Board { cutouts: groove, ..plain };
    let e = hits(&load(&b), "creepage");
    assert!(e.len() == 1 && e[0].contains("U1.1 is 4mm from U2.1 along the surface,"), "{e:?}");
}

#[test]
fn a_pour_keeps_the_barrier_from_the_other_domain() {
    let (fps, pcb) = across(8.0);
    let pour = format!("{pcb}\n[[zones]]\nnet = \"S_COLD\"\nlayers = [\"F.Cu\"]\n");
    let rules = barrier("2mm", "4mm");
    let b = Board {
        rules: &rules,
        cutouts: "",
        footprints: &fps,
        parts: &[("U1", "ONE", 1), ("U2", "ONE", 1)],
        nets: ACROSS,
        pcb: &pour,
    };
    let p = load(&b);
    assert!(hits(&p, "isolation-clearance").is_empty(), "{:?}", hits(&p, "isolation-clearance"));
    assert!(hits(&p, "creepage").is_empty(), "{:?}", hits(&p, "creepage"));
    let l = &p.layouts[0].item;
    let z = l.zones.iter().find(|z| z.layer == "F.Cu").unwrap();
    assert!(!z.filled([13.0, 10.0]) && z.filled([16.0, 10.0]));
}

fn spark(gap: f64, declared: &str, mask: bool) -> String {
    let half = 0.5 + gap / 2.0;
    let opening = if mask {
        format!(
            "\n[[graphics]]\nkind = \"rect\"\nlayer = \"F.Mask\"\nstart = [-{half}, -0.5]\nend = [{half}, 0.5]\nfill = \"solid\"\nwidth = 0\n"
        )
    } else {
        String::new()
    };
    let extra = format!("\n[[spark_gaps]]\npads = [\"1\", \"2\"]\ngap = \"{declared}\"\n{opening}");
    row("SPARK", &[-half, half], &extra)
}

#[test]
fn a_spark_gap_is_exempt_from_clearance_and_checked_as_drawn() {
    let nets: &[(&str, &str, &[&str])] =
        &[("P_LINE", "HV", &["SG1.1"]), ("P_EARTH", "HV", &["SG1.2"])];
    let pcb = at("SG1", 10.0, 10.0);
    let check = |fp: String| {
        let fps = [("SPARK", fp)];
        let b = Board {
            rules: DOMAINS,
            cutouts: "",
            footprints: &fps,
            parts: &[("SG1", "SPARK", 2)],
            nets,
            pcb: &pcb,
        };
        let p = load(&b);
        assert!(hits(&p, "clearance").is_empty(), "{:?}", hits(&p, "clearance"));
        assert!(hits(&p, "mask-web").is_empty(), "{:?}", hits(&p, "mask-web"));
        hits(&p, "spark-gap")
    };
    assert!(check(spark(0.3, "0.3mm", true)).is_empty());
    let e = check(spark(0.3, "0.5mm", true));
    assert!(
        e.len() == 1 && e[0].contains("0.3mm apart on F.Cu, the footprint declares 0.5mm"),
        "{e:?}"
    );
    let e = check(spark(0.3, "0.3mm", false));
    assert!(e.len() == 1 && e[0].contains("solder mask covers the gap on F.Mask"), "{e:?}");
    let e = check(spark(0.05, "0.05mm", true));
    assert!(e.iter().any(|m| m.contains("under the")), "{e:?}");
}

fn under(r: &str, x: f64, y: f64) -> String {
    format!("\n[[footprints]]\nref = \"{r}\"\nat = [{x}, {y}]\nside = \"bottom\"\n")
}

fn round_cutout(c: [f64; 2], r: f64) -> String {
    let pts: Vec<String> = (0..24)
        .map(|k| {
            let a = k as f64 * std::f64::consts::TAU / 24.0;
            format!("[{:.4}, {:.4}]", c[0] + r * a.cos(), c[1] + r * a.sin())
        })
        .collect();
    format!("[[outline.cutouts]]\npoints = [{}]\n", pts.join(", "))
}

#[test]
fn creepage_reaches_the_other_side_through_a_cutout_or_round_the_edge() {
    let fps = vec![("ONE", row("ONE", &[0.0], ""))];
    let rules = barrier("", "6mm");
    let parts: &[(&str, &str, usize)] = &[("U1", "ONE", 1), ("U2", "ONE", 1)];
    let apart = format!("{}{}", at("U1", 10.0, 10.0), under("U2", 14.0, 10.0));
    let b =
        Board { rules: &rules, cutouts: "", footprints: &fps, parts, nets: ACROSS, pcb: &apart };
    assert!(hits(&load(&b), "creepage").is_empty(), "{:?}", hits(&load(&b), "creepage"));
    let hole = round_cutout([12.0, 10.0], 0.5);
    let b = Board { cutouts: &hole, ..b };
    let e = hits(&load(&b), "creepage");
    assert!(e.len() == 1 && e[0].contains("on B.Cu through a board cutout"), "{e:?}");
    let edge = format!("{}{}", at("U1", 2.0, 10.0), under("U2", 2.0, 10.0));
    let b = Board { cutouts: "", pcb: &edge, ..b };
    let e = hits(&load(&b), "creepage");
    assert!(e.len() == 1 && e[0].contains("on B.Cu round the board edge"), "{e:?}");
    assert!(
        e[0].contains("is 4.6104mm"),
        "1.5 mm to the edge, the 1.6104 mm board, 1.5 mm back: {e:?}"
    );
}

const MAINS: &str = "[[netclasses]]\nname = \"Mains\"\ntrack_width = \"0.5mm\"\nvia = \"std\"\nvoltage = \"230VAC\"\n";

const HOT_COLD: &[(&str, &str, &[&str])] =
    &[("L", "Mains", &["U1.1"]), ("GND", "Default", &["U2.1"])];

fn mains(gap: f64, rules: &str) -> Project {
    let (fps, pcb) = across(gap);
    load(&Board {
        rules,
        cutouts: "",
        footprints: &fps,
        parts: &[("U1", "ONE", 1), ("U2", "ONE", 1)],
        nets: HOT_COLD,
        pcb: &pcb,
    })
}

#[test]
fn a_mains_class_is_kept_reinforced_from_low_voltage_with_no_domains_written() {
    let p = mains(2.0, MAINS);
    let b = &p.boards[0].item;
    let class = b.netclass("Mains").unwrap();
    assert_eq!(class.clearance.to_mm(), 2.5, "IPC-2221B B2 for 230VAC");
    let barrier =
        b.barriers.iter().find(|x| x.description.contains("reinforced")).expect("barrier");
    assert!(barrier.clearance.unwrap().to_mm() >= 3.0);
    assert!(barrier.creepage.unwrap().to_mm() >= 5.0);
    let e = hits(&p, "isolation-clearance");
    assert!(e.iter().any(|m| m.contains("U1.1 is 2mm from U2.1")), "{e:?}");
    let p = mains(4.0, MAINS);
    assert!(hits(&p, "isolation-clearance").is_empty());
    let e = hits(&p, "creepage");
    assert!(e.iter().any(|m| m.contains("U1.1 is 4mm from U2.1 along the surface")), "{e:?}");
    let p = mains(5.5, MAINS);
    assert!(hits(&p, "isolation-clearance").is_empty() && hits(&p, "creepage").is_empty());
    assert!(hits(&p, "isolation-unassigned").is_empty(), "implicit domains are not a user's");
}

#[test]
fn low_voltage_classes_make_no_barriers() {
    let low = MAINS.replace("230VAC", "12VDC");
    let p = mains(2.0, &low);
    let b = &p.boards[0].item;
    assert!(b.barriers.is_empty() && b.domains.is_empty(), "{:?}", b.barriers);
    assert_eq!(b.netclass("Mains").unwrap().clearance, b.rules.min_clearance);
}

#[test]
fn a_class_clearance_under_its_voltage_is_an_error() {
    let tight = MAINS.replace("via = \"std\"\n", "via = \"std\"\nclearance = \"0.3mm\"\n");
    let p = mains(6.0, &tight);
    let e: Vec<_> = p.boards[0]
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(e.iter().any(|m| m.contains("carries 230VAC") && m.contains("2.5mm")), "{e:?}");
}

#[test]
fn a_hand_written_barrier_wins_over_the_class_voltage() {
    let rules = format!(
        "{MAINS}\n[[domains]]\nname = \"hot\"\nclasses = [\"Mains\"]\n[[domains]]\nname = \"cold\"\nclasses = [\"Default\"]\n[[barriers]]\nbetween = [\"hot\", \"cold\"]\nclearance = \"8mm\"\n"
    );
    let p = mains(6.0, &rules);
    let b = &p.boards[0].item;
    let hot_cold = |x: &&agentee_core::board::Barrier| x.between == [0, 1] || x.between == [1, 0];
    assert_eq!(b.barriers.iter().filter(hot_cold).count(), 1, "{:?}", b.barriers);
    let e = hits(&p, "isolation-clearance");
    assert!(e.iter().any(|m| m.contains("needs 8mm")), "{e:?}");
}

#[test]
fn spacing_answers_the_same_gap_the_barrier_checks() {
    let p = mains(6.0, MAINS);
    let (b, l) = (&p.boards[0].item, &p.layouts[0].item);
    let s = agentee_core::rules::Spacings::new(b, &l.nets, l.copper.len());
    let net = |n: &str| l.nets.iter().position(|x| x.name == n);
    let (hot, cold) = (net("L"), net("GND"));
    let barrier = s.isolation.barrier(b, hot.unwrap(), cold.unwrap()).expect("a barrier");
    let creepage = barrier.creepage.unwrap().to_mm();
    let clearance = barrier.clearance.unwrap().to_mm();
    assert_eq!(s.gap(hot, cold, 0), creepage.max(clearance), "outer layers keep creepage");
    assert_eq!(s.gap(hot, hot, 0), 0.0);
    assert_eq!(s.gap(hot, None, 0), b.netclass("Mains").unwrap().clearance.to_mm());
    assert!(s.reach(hot) >= creepage);
}

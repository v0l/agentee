use agentee_core::Project;
use agentee_core::diag::Severity;
use std::path::Path;

struct Fixture<'a> {
    preset: &'a str,
    board: &'a str,
    footprints: &'a [(&'a str, &'a str)],
    parts: &'a [(&'a str, &'a str, [f64; 2])],
    nets: &'a [(&'a str, &'a [&'a str])],
    pcb: &'a str,
}

const TWO_PADS: &str = r#"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [-1.0, 0]
size = [1.0, 1.0]
count = 2
pitch = [2.0, 0]
"#;

impl Default for Fixture<'_> {
    fn default() -> Self {
        Fixture {
            preset: "jlcpcb-2l-1.6mm",
            board: "",
            footprints: &[("TWO", TWO_PADS)],
            parts: &[("R1", "TWO", [5.0, 5.0])],
            nets: &[("A", &["R1.1"]), ("B", &["R1.2"])],
            pcb: "",
        }
    }
}

fn load(f: &Fixture) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-drc-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    for (name, body) in f.footprints {
        std::fs::write(
            dir.join(format!("footprints/{name}.fp.toml")),
            format!("name = \"{name}\"\n{body}"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            r#"name = "t"
fab = "jlcpcb"
[outline]
size = [30, 20]
[stackup]
preset = "{}"
finish = "ENIG"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.15mm"
via = "std"
{}
"#,
            f.preset, f.board
        ),
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, fp, at)) in f.parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"0\"\nfootprint = \"{fp}\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!("\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\n", at[0], at[1]);
    }
    for (n, pins) in f.nets {
        let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
        sch += &format!("\n[[nets]]\nname = \"{n}\"\npins = [{}]\n", pins.join(", "));
    }
    pcb += f.pcb;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    Project::load(&dir).unwrap()
}

fn hits(p: &Project, rule: &str) -> Vec<(Severity, String)> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some(rule))
        .map(|d| (d.severity, d.message.clone()))
        .collect()
}

fn via(net: &str, at: [f64; 2]) -> String {
    format!("\n[[vias]]\nnet = \"{net}\"\nat = [{}, {}]\n", at[0], at[1])
}

#[test]
fn a_via_across_a_pad_edge_cuts_it_and_one_inside_is_via_in_pad() {
    let pcb = via("A", [4.0, 5.0]);
    let p = load(&Fixture { pcb: &pcb, ..Default::default() });
    assert!(hits(&p, "via-cuts-pad").is_empty());
    let info = hits(&p, "via-in-pad");
    assert!(info[0].0 == Severity::Info && info[0].1.starts_with("1 vias sit in SMD pads (R1.1)"));
    assert_eq!(
        agentee_core::drc::vias_in_pads(&p.layouts[0].item.parts, &p.layouts[0].item.vias).len(),
        1
    );

    let pcb = via("A", [4.45, 5.0]);
    let e = hits(&load(&Fixture { pcb: &pcb, ..Default::default() }), "via-cuts-pad");
    assert!(e.len() == 1 && e[0].0 == Severity::Error && e[0].1.contains("drill crosses"), "{e:?}");

    let pcb = via("A", [4.8, 5.0]);
    let e = hits(&load(&Fixture { pcb: &pcb, ..Default::default() }), "via-cuts-pad");
    assert!(e.len() == 1 && e[0].1.contains("touches the edge of R1.1"), "{e:?}");

    let pcb = via("A", [4.3, 5.0]);
    let p = load(&Fixture { pcb: &pcb, ..Default::default() });
    assert!(hits(&p, "via-cuts-pad").is_empty());
    let w = hits(&p, "via-annulus-past-pad");
    assert!(w.len() == 1 && w[0].0 == Severity::Warning, "{w:?}");
    assert_eq!(hits(&p, "via-in-pad").len(), 1);

    let pcb = via("A", [4.0, 5.82]);
    let w = hits(&load(&Fixture { pcb: &pcb, ..Default::default() }), "hole-to-smd-pad");
    assert!(w.len() == 1 && w[0].1.contains("closest 0.17mm"), "{w:?}");
}

#[test]
fn rules_can_be_disabled_or_change_severity() {
    let pcb = via("A", [4.5, 5.0]);
    let p = load(&Fixture {
        pcb: &pcb,
        board: "[drc]\ndisable = [\"via-cuts-pad\"]\n",
        ..Default::default()
    });
    assert!(hits(&p, "via-cuts-pad").is_empty());
    let p = load(&Fixture {
        pcb: &pcb,
        board: "[drc]\nseverity = { \"via-cuts-pad\" = \"warning\" }\n",
        ..Default::default()
    });
    let w = hits(&p, "via-cuts-pad");
    assert!(w.len() == 1 && w[0].0 == Severity::Warning, "{w:?}");
    let shown = p.layouts[0].diags.iter().find(|d| d.rule.is_some()).unwrap().to_string();
    assert!(shown.contains(": [via-cuts-pad] "), "{shown}");
}

#[test]
fn a_via_in_pad_wider_than_the_fab_fills_is_an_error() {
    let board = "[[vias]]\nname = \"big\"\ndrill = \"0.6mm\"\ndiameter = \"0.9mm\"\n";
    let pcb = "\n[[vias]]\nnet = \"A\"\nat = [4.0, 5.0]\nvia = \"big\"\n";
    let e = hits(&load(&Fixture { pcb, board, ..Default::default() }), "via-in-pad-fill");
    assert!(e.len() == 1 && e[0].1.contains("fills and caps holes up to 0.55mm"), "{e:?}");
}

const DRILLS: &str = r#"
[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [0, 0]
size = [0.6, 0.6]
drill = 0.1

[[pads]]
kind = "npth"
shape = "circle"
at = [2, 0]
size = [0.4, 0.4]
drill = 0.4

[[pads]]
number = "2"
kind = "tht"
shape = "oval"
at = [4, 0]
size = [0.8, 1.6]
drill = [0.3, 1.0]

[[pads]]
number = "3"
kind = "tht"
shape = "oval"
at = [6, 0]
size = [1.2, 1.6]
drill = [0.6, 1.0]
"#;

#[test]
fn drill_and_slot_sizes_follow_the_fab() {
    let p = load(&Fixture {
        preset: "jlcpcb-4l-1.6mm-7628",
        footprints: &[("DRILLS", DRILLS)],
        parts: &[("R1", "DRILLS", [5.0, 5.0])],
        ..Default::default()
    });
    let e = hits(&p, "drill-size");
    assert!(
        e.iter().any(|x| x.1.contains("plated hole 0.1mm is under the fab minimum 0.15mm")),
        "{e:?}"
    );
    assert!(
        e.iter().any(|x| x.1.contains("non-plated hole 0.4mm is under the fab minimum 0.5mm")),
        "{e:?}"
    );
    let e = hits(&p, "slot-size");
    assert!(
        e.iter().any(|x| x.1.contains("plated slot 0.3mm wide is under the fab minimum 0.35mm")),
        "{e:?}"
    );
    assert!(e.iter().any(|x| x.1.contains("0.6mm x 1mm is shorter than twice its width")), "{e:?}");
}

#[test]
fn a_thin_board_drill_trips_the_aspect_ratio() {
    let p = load(&Fixture { pcb: &via("A", [15.0, 10.0]), ..Default::default() });
    assert!(hits(&p, "aspect-ratio").is_empty());
    let p = load(&Fixture {
        pcb: &via("A", [15.0, 10.0]),
        board: "[rules]\nmax_aspect_ratio = 5\n",
        ..Default::default()
    });
    let e = hits(&p, "aspect-ratio");
    assert!(e.len() == 1 && e[0].1.contains(":1, over the fab's 5:1"), "{e:?}");
}

fn track(net: &str, layer: &str, pts: &str) -> String {
    format!("\n[[tracks]]\nnet = \"{net}\"\nlayer = \"{layer}\"\npoints = {pts}\n")
}

#[test]
fn a_via_hole_near_another_net_is_flagged() {
    let board = "[[vias]]\nname = \"tiny\"\ndrill = \"0.3mm\"\ndiameter = \"0.35mm\"\n";
    let pcb = format!(
        "\n[[vias]]\nnet = \"A\"\nat = [15.0, 10.0]\nvia = \"tiny\"\n{}",
        track("B", "B.Cu", "[[13.0, 10.42], [17.0, 10.42]]")
    );
    let e = hits(&load(&Fixture { pcb: &pcb, board, ..Default::default() }), "hole-to-copper");
    assert!(
        e.len() == 1
            && e[0].1.starts_with("1 holes closer than 0.2mm")
            && e[0].1.contains("0.17mm from track 0 (B)"),
        "{e:?}"
    );
    let pcb = pcb.replace("10.42", "10.46");
    assert!(
        hits(&load(&Fixture { pcb: &pcb, board, ..Default::default() }), "hole-to-copper")
            .is_empty()
    );
}

const THT: &str = r#"
[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [0, 0]
size = [1.1, 1.1]
drill = 1.0

[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [3, 0]
size = [1, 1]
"#;

#[test]
fn inner_layer_hole_clearance_runs_only_with_inner_layers() {
    let pcb = track("B", "In1.Cu", "[[3.0, 5.89], [7.0, 5.89]]");
    let four = load(&Fixture {
        preset: "jlcpcb-4l-1.6mm-7628",
        footprints: &[("THT", THT)],
        parts: &[("R1", "THT", [5.0, 5.0])],
        pcb: &pcb,
        ..Default::default()
    });
    let e = hits(&four, "inner-hole-to-copper");
    assert!(e.len() == 1 && e[0].1.contains("R1.1 is 0.29mm from track 0 (B)"), "{e:?}");
    assert!(hits(&four, "hole-to-copper").is_empty());
    let applies = |p: &Project| {
        let l = &p.layouts[0].item;
        let b = &p.boards[0].item;
        let s = agentee_core::drc::Setup::of(&agentee_core::drc::Ctx::of_layout(b, l));
        agentee_core::drc::status(b, &s)
            .into_iter()
            .find(|r| r.id == "inner-hole-to-copper")
            .unwrap()
            .applies
    };
    assert!(applies(&four));
    let pcb = track("B", "B.Cu", "[[3.0, 5.89], [7.0, 5.89]]");
    let two = load(&Fixture {
        footprints: &[("THT", THT)],
        parts: &[("R1", "THT", [5.0, 5.0])],
        pcb: &pcb,
        ..Default::default()
    });
    assert!(!applies(&two));
    assert!(hits(&two, "inner-hole-to-copper").is_empty());
}

const NPTH: &str = r#"
[[pads]]
kind = "npth"
shape = "circle"
at = [0, 0]
size = [1, 1]
drill = 1.0

[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [3, 0]
size = [1, 1]
count = 2
pitch = [2, 0]
"#;

#[test]
fn npth_keeps_off_copper_and_the_edge() {
    let pcb = track("A", "F.Cu", "[[8.0, 5.75], [12.0, 5.75]]");
    let p = load(&Fixture {
        footprints: &[("NPTH", NPTH)],
        parts: &[("R1", "NPTH", [10.0, 5.0])],
        pcb: &pcb,
        ..Default::default()
    });
    let e = hits(&p, "npth-to-copper");
    assert!(
        e.len() == 1 && e[0].1.contains("R1 hole at [10.000, 5.000] is 0.15mm from track 0 (A)"),
        "{e:?}"
    );
    assert!(hits(&p, "hole-to-edge").is_empty());
    let p = load(&Fixture {
        footprints: &[("NPTH", NPTH)],
        parts: &[("R1", "NPTH", [0.7, 5.0])],
        ..Default::default()
    });
    let e = hits(&p, "hole-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("is 0.2mm from the board edge, needs 0.3mm"), "{e:?}");
}

#[test]
fn smd_pads_of_other_nets_keep_the_fab_gap() {
    let p = load(&Fixture {
        parts: &[("R1", "TWO", [5.0, 5.0]), ("R2", "TWO", [8.12, 5.0])],
        nets: &[("A", &["R1.1"]), ("B", &["R1.2"]), ("C", &["R2.1"]), ("D", &["R2.2"])],
        ..Default::default()
    });
    let e = hits(&p, "smd-pad-gap");
    assert!(e.len() == 1 && e[0].1.contains("closest 0.12mm (R1.2 and R2.1)"), "{e:?}");
}

const TWO_EDGE: &str = r#"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [-1.0, 0]
size = [1.0, 1.0]
edge = true

[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [1.0, 0]
size = [1.0, 1.0]
"#;

#[test]
fn pads_keep_off_the_edge_unless_marked() {
    let p = load(&Fixture { parts: &[("R1", "TWO", [1.6, 5.0])], ..Default::default() });
    let e = hits(&p, "pad-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("pad R1.1 is 0.1mm from the board edge"), "{e:?}");
    let edge = |x: f64| {
        load(&Fixture {
            footprints: &[("TWO", TWO_EDGE)],
            parts: &[("R1", "TWO", [x, 5.0])],
            ..Default::default()
        })
    };
    let p = edge(1.6);
    assert!(hits(&p, "pad-to-edge").is_empty());
    let w = hits(&p, "edge-pad-reach");
    assert!(w.len() == 1 && w[0].1.contains("stops 0.1mm short"), "{w:?}");
    let p = edge(1.5);
    assert!(hits(&p, "pad-to-edge").is_empty() && hits(&p, "edge-pad-reach").is_empty());
}

#[test]
fn a_pad_on_a_thin_neck_of_its_pour_is_starved() {
    let zone = r#"
[[zones]]
net = "A"
layers = ["F.Cu"]
outline = [[4.4, 4.9], [5.2, 4.9], [5.2, 5.1], [4.4, 5.1]]
min_width = 0.1
min_island_area = 0.0
"#;
    let p = load(&Fixture { pcb: zone, ..Default::default() });
    let w = hits(&p, "starved-thermal");
    assert!(w.len() == 1 && w[0].1.contains("joined to the A pour on F.Cu by 1 spoke"), "{w:?}");
    let full = zone.replace(
        "[4.4, 4.9], [5.2, 4.9], [5.2, 5.1], [4.4, 5.1]",
        "[3.0, 4.0], [5.1, 4.0], [5.1, 6.0], [3.0, 6.0]",
    );
    assert!(
        hits(&load(&Fixture { pcb: &full, ..Default::default() }), "starved-thermal").is_empty()
    );
}

fn bga(pitch: f64, pad: f64) -> String {
    let mut s = String::new();
    for (row, name) in ["A", "B", "C", "D"].iter().enumerate() {
        s += &format!(
            "\n[[pads]]\nnumber = \"{name}1\"\nkind = \"smd\"\nshape = \"circle\"\nat = [0, {}]\nsize = [{pad}, {pad}]\ncount = 4\npitch = [{pitch}, 0]\n",
            row as f64 * pitch
        );
    }
    s + "\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"rect\"\nat = [-3, 0]\nsize = [0.5, 0.5]\ncount = 2\npitch = [0, 1]\n"
}

#[test]
fn bga_pads_and_pitch_follow_the_fab() {
    let fp = bga(0.5, 0.15);
    let p = load(&Fixture {
        footprints: &[("BGA", &fp)],
        parts: &[("R1", "BGA", [10.0, 8.0])],
        ..Default::default()
    });
    let e = hits(&p, "bga-pad");
    assert!(e.len() == 1 && e[0].1.contains("are 0.15mm, under the fab minimum 0.2mm"), "{e:?}");
    let w = hits(&p, "bga-pad-ratio");
    assert!(w.len() == 1 && w[0].1.contains("(30%)"), "{w:?}");
    assert!(hits(&p, "bga-pitch").is_empty());
    let fp = bga(0.25, 0.125);
    let p = load(&Fixture {
        footprints: &[("BGA", &fp)],
        parts: &[("R1", "BGA", [10.0, 8.0])],
        ..Default::default()
    });
    let e = hits(&p, "bga-pitch");
    assert!(
        e.len() == 1 && e[0].1.contains("0.25mm ball pitch, finer than the assembler's 0.3mm"),
        "{e:?}"
    );
    let p = load(&Fixture::default());
    let s = agentee_core::drc::Setup::of(&agentee_core::drc::Ctx::of_layout(
        &p.boards[0].item,
        &p.layouts[0].item,
    ));
    assert!(!s.bga);
}

#[test]
fn assembly_advisories() {
    let fp = TWO_PADS
        .replace("size = [1.0, 1.0]", "size = [1.0, 1.0]\nlayers = [\"F.Cu\", \"F.Paste\"]");
    let p = load(&Fixture {
        footprints: &[("TWO", &fp)],
        parts: &[("R1", "TWO", [1.9, 5.0])],
        ..Default::default()
    });
    let w = hits(&p, "paste-without-mask");
    assert!(
        w.len() == 1 && w[0].1.contains("pads 1, 2 of TWO have paste but no mask opening"),
        "{w:?}"
    );
    let w = hits(&p, "part-to-edge");
    assert!(
        w.len() == 1
            && w[0].0 == Severity::Warning
            && w[0].1.contains("pad 1 is 0.4mm from the board edge"),
        "{w:?}"
    );
    assert!(hits(&p, "pad-to-edge").is_empty());
    assert_eq!(hits(&p, "fiducials")[0].0, Severity::Info);
    assert_eq!(hits(&p, "tooling-holes")[0].0, Severity::Info);
}

#[test]
fn layout_checks_carry_rule_ids_and_follow_the_drc_table() {
    let pcb = "\n[[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\npoints = [[4, 5], [6, 5]]\n\n\
               [[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\npoints = [[4, 5], [4, 8]]\n";
    let p = load(&Fixture { pcb, ..Default::default() });
    let s = hits(&p, "short");
    assert!(
        s.len() == 1 && s[0].0 == Severity::Error && s[0].1.contains("R1.2 touches track 0 (A)"),
        "{s:?}"
    );
    let w = hits(&p, "dangling-track");
    assert!(w.len() == 2 && w[1].1.contains("end at [4.000, 8.000] connects to nothing"), "{w:?}");
    let p = load(&Fixture {
        pcb,
        board: "[drc]\ndisable = [\"dangling-track\"]\nseverity = { \"short\" = \"warning\" }\n",
        ..Default::default()
    });
    assert!(hits(&p, "dangling-track").is_empty());
    let s = hits(&p, "short");
    assert!(s.len() == 1 && s[0].0 == Severity::Warning, "{s:?}");
    assert!(!p.layouts[0].diags.iter().any(|d| d.message.contains("connects to nothing")));
}

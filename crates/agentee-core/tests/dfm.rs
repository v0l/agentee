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

#[test]
fn a_via_of_another_net_on_a_pad_is_a_short_not_a_cut() {
    for at in [[4.45, 5.0], [4.0, 5.0]] {
        let pcb = via("B", at);
        let p = load(&Fixture { pcb: &pcb, ..Default::default() });
        assert!(hits(&p, "via-cuts-pad").is_empty(), "{at:?}");
        assert!(hits(&p, "via-in-pad").is_empty(), "{at:?}");
        let s = hits(&p, "short");
        assert!(s.len() == 1 && s[0].1.contains("R1.1 touches via at"), "{s:?}");
    }
}

#[test]
fn pads_that_share_a_number_are_starved_once() {
    let dup = format!(
        "{TWO_PADS}\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"rect\"\nat = [-1.0, 0]\nsize = [1.0, 1.0]\n"
    );
    let zone = "\n[[zones]]\nnet = \"A\"\nlayers = [\"F.Cu\"]\n\
                outline = [[4.4, 4.9], [5.2, 4.9], [5.2, 5.1], [4.4, 5.1]]\n\
                min_width = 0.1\nmin_island_area = 0.0\n";
    let p = load(&Fixture { footprints: &[("TWO", &dup)], pcb: zone, ..Default::default() });
    let w = hits(&p, "starved-thermal");
    assert!(w.len() == 1 && w[0].1.contains("by 1 spoke"), "{w:?}");
}

#[test]
fn thin_board_silk_lines_are_flagged() {
    let pcb = "\n[[graphics]]\nkind = \"line\"\nlayer = \"F.SilkS\"\nstart = [10, 12]\nend = [20, 12]\n\
               width = \"0.1mm\"\n\n[[graphics]]\nkind = \"line\"\nlayer = \"F.SilkS\"\n\
               start = [10, 14]\nend = [20, 14]\nwidth = \"0.2mm\"\n";
    let w = hits(&load(&Fixture { pcb, ..Default::default() }), "silk-width");
    assert!(w.len() == 1 && w[0].1.contains("1 board silk lines under"), "{w:?}");
    assert!(w[0].1.contains("thinnest 0.1mm, at [10.000, 12.000] on F.SilkS"), "{w:?}");
    let wide = pcb.replace("0.1mm", "0.15mm");
    assert!(hits(&load(&Fixture { pcb: &wide, ..Default::default() }), "silk-width").is_empty());
}

const FAB_BODY: &str = r#"
[[graphics]]
kind = "rect"
layer = "F.Fab"
start = [-2.0, -1.0]
end = [2.0, 1.0]
"#;

#[test]
fn part_bodies_keep_off_the_edge_unless_they_overhang() {
    let fab = format!("{TWO_PADS}{FAB_BODY}");
    let at = |fp: &str, x: f64| {
        let fp = fp.to_string();
        let p = load(&Fixture {
            footprints: &[("TWO", fp.as_str())],
            parts: &[("R1", "TWO", [x, 5.0])],
            ..Default::default()
        });
        (hits(&p, "part-body-to-edge"), hits(&p, "part-to-edge"))
    };
    let (b, e) = at(&fab, 2.6);
    assert!(
        b.len() == 1 && b[0].0 == Severity::Info && b[0].1.contains("its fab outline is 0.6mm"),
        "{b:?}"
    );
    assert!(e.is_empty(), "{e:?}");
    let court = fab.replace("F.Fab", "F.CrtYd");
    let (b, _) = at(&court, 2.6);
    assert!(b.len() == 1 && b[0].1.contains("its courtyard is 0.6mm"), "{b:?}");
    let (b, _) = at(TWO_PADS, 2.3);
    assert!(b.len() == 1 && b[0].1.contains("its pad copper is 0.8mm"), "{b:?}");
    assert!(at(&fab, 3.1).0.is_empty());
    let (b, e) = at(&fab, 1.8);
    assert!(
        b.len() == 1 && b[0].1.contains("its fab outline reaches past the board edge"),
        "{b:?}"
    );
    assert!(e.len() == 1 && e[0].0 == Severity::Warning, "{e:?}");
    let (b, e) = at(&format!("overhang = true\n{fab}"), 1.8);
    assert!(b.is_empty(), "{b:?}");
    assert!(e.len() == 1 && e[0].1.contains("pad 1 is 0.3mm from the board edge"), "{e:?}");
    let p = load(&Fixture {
        footprints: &[("TWO", fab.as_str())],
        parts: &[("R1", "TWO", [2.6, 5.0])],
        board: "[drc]\nseverity = { \"part-body-to-edge\" = \"error\" }\n",
        ..Default::default()
    });
    assert_eq!(hits(&p, "part-body-to-edge")[0].0, Severity::Error);
}

fn chip(pitch: [f64; 2], size: [f64; 2]) -> String {
    format!(
        "\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"roundrect\"\nat = [{}, {}]\nsize = [{}, {}]\ncount = 2\npitch = [{}, {}]\n",
        -pitch[0] / 2.0,
        -pitch[1] / 2.0,
        size[0],
        size[1],
        pitch[0],
        pitch[1]
    )
}

const HOLE: &str = r#"
[[pads]]
number = ""
kind = "npth"
shape = "circle"
at = [0, 0]
size = [3.0, 3.0]
drill = 3.0
"#;

#[test]
fn ceramic_caps_in_the_flex_zone_are_noted_by_case_and_orientation() {
    let h = chip([0.96, 0.0], [0.56, 0.62]);
    let v = chip([0.0, 0.96], [0.62, 0.56]);
    let big = chip([1.9, 0.0], [1.0, 1.45]);
    let fps = [
        ("C_0402_1005Metric", h.as_str()),
        ("C_0402_1005Metric_V", v.as_str()),
        ("C_0805_2012Metric", big.as_str()),
        ("MountingHole_3mm", HOLE),
    ];
    let p = load(&Fixture {
        footprints: &fps,
        parts: &[
            ("C1", "C_0402_1005Metric", [2.5, 10.0]),
            ("C2", "C_0402_1005Metric", [15.0, 2.5]),
            ("C3", "C_0805_2012Metric", [27.0, 10.0]),
            ("C4", "C_0805_2012Metric", [6.8, 10.0]),
            ("H1", "MountingHole_3mm", [15.0, 10.0]),
            ("C5", "C_0402_1005Metric_V", [15.0, 13.0]),
        ],
        nets: &[],
        ..Default::default()
    });
    let e = hits(&p, "mlcc-flex-zone-case");
    assert!(
        e.len() == 1
            && e[0].0 == Severity::Info
            && e[0]
                .1
                .contains("0805 ceramic capacitor 1.55mm from the board edge at [30.000, 10.000]"),
        "{e:?}"
    );
    let w = hits(&p, "mlcc-flex-zone");
    assert!(w.len() == 2, "{w:?}");
    assert!(
        w[0].1.contains("0402 ceramic capacitor 1.74mm from the board edge at [0.000, 10.000]"),
        "{w:?}"
    );
    assert!(w[1].1.contains("from mounting hole H1 with its long axis pointing at it"), "{w:?}");
    let i = hits(&p, "mlcc-flex-zone-info");
    assert!(
        i.len() == 1 && i[0].1.contains("1 small ceramic capacitors") && i[0].1.ends_with("C2"),
        "{i:?}"
    );

    let p = load(&Fixture {
        footprints: &fps,
        parts: &[("C1", "C_0402_1005Metric", [2.5, 10.0])],
        nets: &[],
        pcb: "mlcc = false\n",
        ..Default::default()
    });
    assert!(hits(&p, "mlcc-flex-zone").is_empty());
    let s = agentee_core::drc::Setup::of(&agentee_core::drc::Ctx::of_layout(
        &p.boards[0].item,
        &p.layouts[0].item,
    ));
    assert!(!s.mlcc && s.small_chips && !s.tall_parts);
    let opted = format!("mlcc = false\n{h}");
    let p = load(&Fixture {
        footprints: &[("C_0402_1005Metric", opted.as_str())],
        parts: &[("C1", "C_0402_1005Metric", [2.5, 10.0])],
        nets: &[],
        ..Default::default()
    });
    assert!(hits(&p, "mlcc-flex-zone").is_empty());
}

#[test]
fn small_chips_with_unbalanced_pads_risk_tombstoning() {
    let even = chip([0.96, 0.0], [0.56, 0.62]);
    let fixture = |fp: &str, pcb: &str| {
        let fp = fp.to_string();
        let pcb = pcb.to_string();
        let p = load(&Fixture {
            footprints: &[("R_0402_1005Metric", fp.as_str())],
            parts: &[("R1", "R_0402_1005Metric", [10.0, 10.0])],
            pcb: &pcb,
            ..Default::default()
        });
        hits(&p, "tombstone-risk")
    };
    let tracks = format!(
        "{}{}",
        track("A", "F.Cu", "[[9.52, 10.0], [7.0, 10.0]]"),
        track("B", "F.Cu", "[[10.48, 10.0], [13.0, 10.0]]")
    );
    assert!(fixture(&even, &tracks).is_empty());
    let odd = even.replace("count = 2\npitch = [0.96, 0]\n", "")
        + "\n[[pads]]\nnumber = \"2\"\nkind = \"smd\"\nshape = \"rect\"\nat = [0.48, 0]\nsize = [0.7, 0.62]\n";
    let t = fixture(&odd, &tracks);
    assert!(
        t.len() == 1
            && t[0].0 == Severity::Info
            && t[0].1.contains("pads 1 and 2 differ in size or shape"),
        "{t:?}"
    );
    let t = fixture(&even, &format!("{tracks}{}", via("A", [9.52, 10.0])));
    assert!(t.len() == 1 && t[0].1.contains("pad 1 has a via in it and pad 2 not"), "{t:?}");
    let pour = format!(
        "{}\n[[zones]]\nnet = \"B\"\nlayers = [\"F.Cu\"]\noutline = [[10.3, 8.0], [14.0, 8.0], [14.0, 12.0], [10.3, 12.0]]\nmin_island_area = 0.0\n",
        track("A", "F.Cu", "[[9.52, 10.0], [7.0, 10.0]]")
    );
    let t = fixture(&even, &pour);
    assert!(
        t.len() == 1 && t[0].1.contains("pad 2 has") && t[0].1.contains("and pad 1 0.06 mm2"),
        "{t:?}"
    );
}

#[test]
fn tombstone_weighs_a_thermal_relief_by_its_spokes_and_skips_unrouted_pads() {
    let even = chip([0.96, 0.0], [0.56, 0.62]);
    let fixture = |board: &str, pcb: &str| {
        let p = load(&Fixture {
            board,
            footprints: &[("R_0402_1005Metric", even.as_str())],
            parts: &[("R1", "R_0402_1005Metric", [10.0, 10.0])],
            pcb,
            ..Default::default()
        });
        hits(&p, "tombstone-risk")
    };
    let relief = format!(
        "{}\n[[zones]]\nnet = \"B\"\nlayers = [\"F.Cu\"]\n\
         outline = [[10.6, 9.85], [10.91, 9.85], [10.91, 8.0], [14.0, 8.0], [14.0, 12.0], [10.91, 12.0], [10.91, 10.15], [10.6, 10.15]]\n\
         min_island_area = 0.0\n",
        track("A", "F.Cu", "[[9.52, 10.0], [7.0, 10.0]]")
    );
    let t = fixture("", &relief);
    assert!(t.is_empty(), "{t:?}");
    let t = fixture("[drc]\ntombstone_ratio = 1.2\n", &relief);
    assert!(
        t.len() == 1
            && t[0].1.contains("pad 2 is fed by 0.3mm of spokes and tracks and pad 1 by 0.2mm"),
        "{t:?}"
    );
    let unrouted = "\n[[zones]]\nnet = \"B\"\nlayers = [\"F.Cu\"]\n\
                    outline = [[10.3, 8.0], [14.0, 8.0], [14.0, 12.0], [10.3, 12.0]]\nmin_island_area = 0.0\n";
    let t = fixture("", unrouted);
    assert!(t.is_empty(), "{t:?}");
}

#[test]
fn small_chips_keep_a_tall_part_height_away() {
    let tall = format!("height = \"4mm\"\n{TWO_PADS}{FAB_BODY}");
    let small = chip([0.96, 0.0], [0.56, 0.62]);
    let near = |x: f64, fp: &str| {
        let fp = fp.to_string();
        let p = load(&Fixture {
            footprints: &[("TALL", fp.as_str()), ("R_0402_1005Metric", small.as_str())],
            parts: &[("U1", "TALL", [10.0, 10.0]), ("R1", "R_0402_1005Metric", [x, 10.0])],
            nets: &[],
            ..Default::default()
        });
        hits(&p, "tall-part-shadow")
    };
    let w = near(13.3, &tall);
    assert!(
        w.len() == 1
            && w[0].0 == Severity::Info
            && w[0].1.contains("0402 chip 0.54mm from U1, which is 4mm tall"),
        "{w:?}"
    );
    assert!(near(17.0, &tall).is_empty());
    assert!(near(13.3, &format!("{TWO_PADS}{FAB_BODY}")).is_empty());

    let named = format!("{TWO_PADS}{FAB_BODY}");
    let p = load(&Fixture {
        footprints: &[("L_Big_h5.0mm", named.as_str()), ("R_0402_1005Metric", small.as_str())],
        parts: &[("L1", "L_Big_h5.0mm", [10.0, 10.0]), ("R1", "R_0402_1005Metric", [13.3, 10.0])],
        nets: &[],
        ..Default::default()
    });
    let w = hits(&p, "tall-part-shadow");
    assert!(w.len() == 1 && w[0].1.contains("from L1, which is 5mm tall"), "{w:?}");
}

#[test]
fn disabled_silk_rules_skip_the_silk_text_search() {
    let fp = format!(
        "{}\n[[graphics]]\nkind = \"text\"\nlayer = \"F.SilkS\"\nat = [0, -1.5]\ntext = \"${{REFERENCE}}\"\nsize = 1.0\n",
        chip([0.96, 0.0], [0.56, 0.62])
    );
    let load_with = |board: &str| {
        load(&Fixture {
            board,
            footprints: &[("R_0402_1005Metric", fp.as_str())],
            parts: &[
                ("R1", "R_0402_1005Metric", [10.0, 10.0]),
                ("R2", "R_0402_1005Metric", [10.0, 10.3]),
            ],
            nets: &[],
            ..Default::default()
        })
    };
    let p = load_with("");
    assert!(!hits(&p, "silk-text").is_empty());
    assert!(!p.layouts[0].item.label_fixes.is_empty());
    let p = load_with("[drc]\ndisable = [\"silk-text\", \"silk-hidden\"]\n");
    assert!(hits(&p, "silk-text").is_empty());
    assert!(p.layouts[0].item.label_fixes.is_empty());
}

#[test]
fn a_relief_zone_joins_smd_pads_by_four_spokes() {
    let fp = chip([0.96, 0.0], [0.56, 0.62]);
    let zone = |extra: &str| {
        format!(
            "\n[[zones]]\nnet = \"B\"\nlayers = [\"F.Cu\"]\noutline = [[10.3, 8.0], [14.0, 8.0], [14.0, 12.0], [10.3, 12.0]]\nmin_island_area = 0.0\n{extra}"
        )
    };
    let load_with = |pcb: &str| {
        load(&Fixture {
            footprints: &[("R_0402_1005Metric", fp.as_str())],
            parts: &[("R1", "R_0402_1005Metric", [10.0, 10.0])],
            pcb,
            ..Default::default()
        })
    };
    let corner = [10.91, 10.46];
    let p = load_with(&zone(""));
    assert!(p.layouts[0].item.zones[0].filled(corner));
    let p = load_with(&zone(
        "pad_connection = \"relief\"\nrelief_gap = \"0.3mm\"\nspoke_width = \"0.3mm\"\n",
    ));
    let fill = &p.layouts[0].item.zones[0];
    assert!(!fill.filled(corner));
    assert!(fill.filled([10.91, 10.0]) && fill.filled([10.48, 10.46]));
    assert!(fill.filled([11.4, 10.46]));
    assert!(p.layouts[0].item.nets.iter().all(|n| n.unrouted == 0));
    assert!(hits(&p, "starved-thermal").is_empty());
}

const SLOT: &str = "[[outline.cutouts]]\norigin = [14, 8]\nsize = [2, 4]\n";

#[test]
fn a_board_cutout_is_board_edge_for_copper_pads_and_holes() {
    let pcb = format!(
        "{}{}",
        track("A", "F.Cu", "[[10.0, 10.0], [13.8, 10.0]]"),
        track("B", "B.Cu", "[[12.0, 13.0], [12.0, 7.0], [18.0, 7.0], [18.0, 11.0], [13.0, 11.0]]")
    );
    let p = load(&Fixture {
        board: SLOT,
        parts: &[("R1", "TWO", [17.6, 13.0]), ("R2", "TWO", [15.0, 9.0])],
        nets: &[("A", &["R1.1"]), ("B", &["R1.2"])],
        pcb: &pcb,
        ..Default::default()
    });
    let e = hits(&p, "copper-to-edge");
    assert!(
        e.len() == 2
            && e[0].1.contains("track 0 (A) is 0.1mm from the board edge, needs 0.3mm")
            && e[1].1.contains("track 1 (B) leaves the board"),
        "{e:?}"
    );
    let e = hits(&p, "pad-off-board");
    assert!(e.len() == 1 && e[0].1.contains("pads 1, 2 hang off the board"), "{e:?}");
    let p =
        load(&Fixture { board: SLOT, parts: &[("R1", "TWO", [17.6, 10.0])], ..Default::default() });
    let e = hits(&p, "pad-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("pad R1.1 is 0.1mm from the board edge"), "{e:?}");
    let e = hits(&p, "part-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("pad 1 is 0.1mm from the board edge"), "{e:?}");
    let e = hits(&p, "part-body-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("its pad copper is 0.1mm"), "{e:?}");
    let p = load(&Fixture {
        board: SLOT,
        footprints: &[("NPTH", NPTH)],
        parts: &[("R1", "NPTH", [16.7, 10.0])],
        ..Default::default()
    });
    let e = hits(&p, "hole-to-edge");
    assert!(e.len() == 1 && e[0].1.contains("is 0.2mm from the board edge, needs 0.3mm"), "{e:?}");
    let p = load(&Fixture {
        board: "[[outline.cutouts]]\norigin = [25, 8]\nsize = [8, 4]\n",
        ..Default::default()
    });
    let e: Vec<_> =
        p.layouts[0].diags.iter().filter(|d| d.at.contains("outline.cutouts[0]")).collect();
    assert!(e.len() == 1 && e[0].message.contains("runs past the board outline"), "{e:?}");
}

#[test]
fn part_bodies_keep_off_a_board_cutout() {
    let fab = format!("{TWO_PADS}{FAB_BODY}");
    let p = load(&Fixture {
        board: SLOT,
        footprints: &[("TWO", fab.as_str())],
        parts: &[("R1", "TWO", [18.6, 10.0])],
        ..Default::default()
    });
    let b = hits(&p, "part-body-to-edge");
    assert!(b.len() == 1 && b[0].1.contains("its fab outline is 0.6mm"), "{b:?}");
    let p = load(&Fixture {
        board: SLOT,
        footprints: &[("TWO", fab.as_str())],
        parts: &[("R1", "TWO", [15.0, 10.0])],
        ..Default::default()
    });
    let b = hits(&p, "part-body-to-edge");
    assert!(b.len() == 1 && b[0].1.contains("reaches past the board edge"), "{b:?}");
}

#[test]
fn ceramic_caps_near_a_board_cutout_are_in_the_flex_zone() {
    let big = chip([1.9, 0.0], [1.0, 1.45]);
    let p = load(&Fixture {
        board: SLOT,
        footprints: &[("C_0805_2012Metric", big.as_str())],
        parts: &[("C1", "C_0805_2012Metric", [18.0, 10.0])],
        nets: &[],
        ..Default::default()
    });
    let e = hits(&p, "mlcc-flex-zone-case");
    assert!(
        e.len() == 1 && e[0].1.contains("from the edge of board cutout 0 at [16.000, 10.000]"),
        "{e:?}"
    );
}

#[test]
fn a_pour_clears_a_board_cutout_and_its_stored_fill_goes_stale() {
    let zone = format!(
        "{}\n[[zones]]\nnet = \"A\"\nlayers = [\"B.Cu\"]\nmin_island_area = 0.0\n",
        via("A", [5.0, 15.0])
    );
    let pour = |board: &str| {
        let p = load(&Fixture { board, pcb: &zone, ..Default::default() });
        let l = &p.layouts[0].item;
        (l.fill_keys[0].hash, l.zones[0].clone(), l.board_cutouts.clone(), l.outline.clone())
    };
    let (plain, fill, ..) = pour("");
    assert!(fill.filled([15.0, 10.0]));
    let (slotted, fill, cutouts, outline) = pour(SLOT);
    assert_ne!(plain, slotted);
    assert!(!fill.filled([15.0, 10.0]) && !fill.filled([16.2, 10.0]) && fill.filled([16.5, 10.0]));
    let edge = agentee_core::geom::BoardEdge::new(&outline, &cutouts);
    let closest = fill.rings.iter().flatten().map(|p| edge.distance(*p)).fold(f64::MAX, f64::min);
    assert!(closest > 0.3 - 1e-6, "fill {closest} from the edge");
    let (moved, ..) = pour(&SLOT.replace("[14, 8]", "[14, 9]"));
    assert_ne!(moved, slotted);
    assert_eq!(pour("").0, plain);
}

#[test]
fn the_router_goes_around_a_board_cutout() {
    let p = load(&Fixture {
        board: "[[outline.cutouts]]\norigin = [14, 5]\nsize = [2, 10]\n",
        parts: &[("R1", "TWO", [10.0, 10.0]), ("R2", "TWO", [20.0, 10.0])],
        nets: &[("A", &["R1.2", "R2.1"])],
        ..Default::default()
    });
    let l = &p.layouts[0].item;
    let opts = agentee_core::route::RouteOptions {
        nets: vec!["A".into()],
        layers: vec!["F.Cu".into()],
        grid: 0.1,
        ..Default::default()
    };
    let r = agentee_core::route::route(l, &p.boards[0].item, &opts).unwrap();
    assert!(r.connections == 1 && r.routed == 1, "{:?}", r.failed);
    let edge = l.edge();
    let closest = r
        .tracks
        .iter()
        .flat_map(|t| t.points.windows(2).map(|w| edge.segment_distance(w[0], w[1])))
        .fold(f64::MAX, f64::min);
    assert!(closest >= 0.3 + 0.1 - 1e-6, "track centre {closest} from the edge");
}

#[test]
fn silk_over_a_board_cutout_runs_off_the_board_and_the_watermark_avoids_it() {
    let text = "\n[[graphics]]\nkind = \"text\"\nlayer = \"F.SilkS\"\nat = [15.0, 10.0]\ntext = \"SLOT EDGE\"\nsize = 1.0\n";
    let p = load(&Fixture { board: SLOT, pcb: text, ..Default::default() });
    let e = hits(&p, "silk-text");
    assert!(e.iter().any(|(_, m)| m.contains("runs off the board")), "{e:?}");
    let slot = [[1.0, 1.0], [26.0, 1.0], [26.0, 19.0], [1.0, 19.0]]
        .iter()
        .map(|p| format!("[{}, {}]", p[0], p[1]))
        .collect::<Vec<_>>()
        .join(", ");
    let wide = format!("[[outline.cutouts]]\npoints = [{slot}]\n");
    let p = load(&Fixture { board: &wide, parts: &[], nets: &[], ..Default::default() });
    let l = &p.layouts[0].item;
    let w = l.watermark.as_ref().expect("a spot beside the cutout");
    let edge = l.edge();
    assert!(edge.holds(&w.outline()) && w.at[0] > 26.0, "watermark at {:?}", w.at);
}

const HDI_VIAS: &str = r#"
[rules]
hdi = true
min_microvia_diameter = "0.25mm"
"#;

const HDI_VIA_TYPES: &str = r#"
[[vias]]
name = "uv"
drill = "0.1mm"
diameter = "0.25mm"
type = "microvia"
from = "F.Cu"
to = "In1.Cu"
[[vias]]
name = "bu"
drill = "0.2mm"
diameter = "0.45mm"
from = "In1.Cu"
to = "In4.Cu"
[[vias]]
name = "vip"
drill = "0.1mm"
diameter = "0.25mm"
type = "microvia"
from = "F.Cu"
to = "In1.Cu"
fill = "plugged"
[[vias]]
name = "bd"
drill = "0.2mm"
diameter = "0.45mm"
backdrill = { from = "B.Cu", to = "In1.Cu", max_stub = "0.2mm" }
"#;

fn typed_via(net: &str, at: [f64; 2], kind: &str) -> String {
    format!("\n[[vias]]\nnet = \"{net}\"\nat = [{}, {}]\nvia = \"{kind}\"\n", at[0], at[1])
}

fn hdi(pcb: &str, board: &str) -> Project {
    let board = format!("{HDI_VIAS}{board}{HDI_VIA_TYPES}");
    load(&Fixture { preset: "hdi-6l-1n1", board: &board, pcb, ..Default::default() })
}

#[test]
fn a_microvia_occupies_only_its_span() {
    let crossing = track("B", "In2.Cu", "[[8.0, 12.0], [12.0, 12.0]]");
    let p = hdi(&format!("{}{crossing}", typed_via("A", [10.0, 12.0], "uv")), "");
    let v = &p.layouts[0].item.vias[0];
    assert_eq!(v.layers, ["F.Cu", "In1.Cu"]);
    assert!(hits(&p, "short").is_empty(), "{:?}", hits(&p, "short"));
    assert!(hits(&p, "hole-to-copper").is_empty(), "{:?}", hits(&p, "hole-to-copper"));
    let p = hdi(&format!("{}{crossing}", typed_via("A", [10.0, 12.0], "std")), "");
    assert_eq!(hits(&p, "short").len(), 1);
}

#[test]
fn hole_to_hole_counts_only_holes_through_a_shared_dielectric() {
    let p = hdi(
        &format!("{}{}", typed_via("A", [10.0, 12.0], "uv"), typed_via("A", [10.3, 12.0], "bu")),
        "",
    );
    assert!(hits(&p, "hole-to-hole").is_empty(), "{:?}", hits(&p, "hole-to-hole"));
    let p = hdi(
        &format!("{}{}", typed_via("A", [10.0, 12.0], "uv"), typed_via("B", [10.3, 12.0], "uv")),
        "",
    );
    assert_eq!(hits(&p, "hole-to-hole").len(), 1);
}

#[test]
fn stacked_vias_follow_the_fab_rule() {
    let stack =
        format!("{}{}", typed_via("A", [10.0, 12.0], "uv"), typed_via("A", [10.0, 12.0], "bu"));
    let e = hits(&hdi(&stack, ""), "stacked-via");
    assert!(e.len() == 1 && e[0].1.contains("stacked on another via"), "{e:?}");
    assert!(hits(&hdi(&stack, "stacked_microvias = true\n"), "stacked-via").is_empty());
    let twice =
        format!("{}{}", typed_via("A", [10.0, 12.0], "uv"), typed_via("A", [10.0, 12.0], "uv"));
    let e = hits(&hdi(&twice, "stacked_microvias = true\n"), "stacked-via");
    assert!(e.len() == 1 && e[0].1.contains("through the same layers"), "{e:?}");
}

#[test]
fn a_via_in_pad_needs_a_filled_and_capped_type() {
    let p = hdi(&typed_via("A", [4.0, 5.0], "vip"), "");
    let e = hits(&p, "via-in-pad-fill");
    assert!(e.len() == 1 && e[0].1.contains("plugged (IPC-4761 type III)"), "{e:?}");
    let info = hits(&p, "via-in-pad");
    assert!(info[0].1.contains("microvia plugged"), "{info:?}");
    assert!(hits(&hdi(&typed_via("A", [4.0, 5.0], "uv"), ""), "via-in-pad-fill").is_empty());
}

#[test]
fn a_pour_on_a_layer_the_via_misses_keeps_no_antipad() {
    let zone = "\n[[zones]]\nnet = \"B\"\nlayers = [\"In2.Cu\", \"In1.Cu\"]\n\
                outline = [[8.0, 10.0], [12.0, 10.0], [12.0, 14.0], [8.0, 14.0]]\n\
                min_island_area = 0.0\n";
    let anchor = typed_via("B", [9.0, 11.0], "std");
    let p = hdi(&format!("{}{anchor}{zone}", typed_via("A", [10.0, 12.0], "uv")), "");
    let l = &p.layouts[0].item;
    let on = |layer: &str| l.zones.iter().find(|z| z.layer == layer).unwrap().filled([10.0, 12.0]);
    assert!(on("In2.Cu"));
    assert!(!on("In1.Cu"));
    assert!(hits(&p, "short").is_empty());
}

#[test]
fn a_via_stub_ends_at_the_via_span_or_its_backdrill() {
    let lane = format!(
        "{}{}\n[[interfaces]]\nname = \"x\"\nnets = [\"A\"]\nmax_stub = \"0.1mm\"\n",
        track("A", "F.Cu", "[[4.0, 5.0], [10.0, 12.0]]"),
        track("A", "In1.Cu", "[[10.0, 12.0], [14.0, 12.0]]")
    );
    let stub = |kind: &str| {
        let p = hdi(&format!("{lane}{}", typed_via("A", [10.0, 12.0], kind)), "");
        let l = &p.layouts[0].item;
        (l.interfaces[0].lanes[0].stub_mm, hits(&p, "interface-stub"))
    };
    let (mm, e) = stub("uv");
    assert!(mm.abs() < 1e-9 && e.is_empty(), "{mm} {e:?}");
    let (mm, e) = stub("bd");
    assert!((mm - 0.2).abs() < 1e-9 && e.len() == 1, "{mm} {e:?}");
    let (mm, e) = stub("std");
    assert!(mm > 0.6 && e.len() == 1, "{mm} {e:?}");
}

#[test]
fn the_router_picks_the_cheapest_class_via_that_spans_the_layer_change() {
    let wall = track("B", "F.Cu", "[[12.0, 0.1], [12.0, 19.9]]");
    let board = format!("{HDI_VIAS}{HDI_VIA_TYPES}");
    let p = load(&Fixture {
        preset: "hdi-6l-1n1",
        board: &board,
        parts: &[("R1", "TWO", [5.0, 5.0]), ("R2", "TWO", [20.0, 5.0])],
        nets: &[("A", &["R1.2", "R2.1"]), ("B", &["R1.1"])],
        pcb: &wall,
        ..Default::default()
    });
    let l = &p.layouts[0].item;
    let routed = |uv_cost: f64, layers: &[&str]| {
        let mut b = p.boards[0].item.clone();
        b.netclasses[0].via = vec!["uv".into(), "std".into()];
        b.vias.iter_mut().find(|v| v.name == "uv").unwrap().cost = uv_cost;
        let opts = agentee_core::route::RouteOptions {
            nets: vec!["A".into()],
            layers: layers.iter().map(|s| s.to_string()).collect(),
            grid: 0.1,
            ..Default::default()
        };
        let r = agentee_core::route::route(l, &b, &opts).unwrap();
        assert!(r.routed == 1, "{:?}", r.failed);
        let mut names: Vec<String> = r.vias.iter().map(|v| v.via.clone()).collect();
        names.dedup();
        names
    };
    assert_eq!(routed(1.0, &["F.Cu", "In1.Cu"]), ["uv"]);
    assert_eq!(routed(5.0, &["F.Cu", "In1.Cu"]), ["std"]);
    assert_eq!(routed(1.0, &["F.Cu", "In2.Cu"]), ["std"]);
}

#[test]
fn the_router_staggers_vias_unless_the_fab_stacks_them() {
    let walls = format!(
        "{}{}",
        track("B", "F.Cu", "[[12.0, 0.1], [12.0, 19.9]]"),
        track("B", "In1.Cu", "[[12.0, 0.1], [12.0, 19.9]]")
    );
    let routed = |stack: bool| {
        let board = format!("{HDI_VIAS}stacked_microvias = {stack}\n{HDI_VIA_TYPES}");
        let p = load(&Fixture {
            preset: "hdi-6l-1n1",
            board: &board,
            parts: &[("R1", "TWO", [5.0, 5.0]), ("R2", "TWO", [20.0, 5.0])],
            nets: &[("A", &["R1.2", "R2.1"]), ("B", &["R1.1"])],
            pcb: &walls,
            ..Default::default()
        });
        let mut b = p.boards[0].item.clone();
        b.netclasses[0].via = vec!["uv".into(), "bu".into()];
        let opts = agentee_core::route::RouteOptions {
            nets: vec!["A".into()],
            layers: vec!["F.Cu".into(), "In1.Cu".into(), "In4.Cu".into()],
            grid: 0.1,
            ..Default::default()
        };
        let r = agentee_core::route::route(&p.layouts[0].item, &b, &opts).unwrap();
        assert!(r.routed == 1, "{:?}", r.failed);
        let stacked =
            r.vias.iter().enumerate().any(|(i, a)| {
                r.vias[..i].iter().any(|b| agentee_core::geom::dist(a.at, b.at) < 1e-6)
            });
        let kinds: std::collections::BTreeSet<String> =
            r.vias.iter().map(|v| v.via.clone()).collect();
        (stacked, kinds)
    };
    let (stacked, kinds) = routed(false);
    assert!(!stacked);
    assert_eq!(kinds.into_iter().collect::<Vec<_>>(), ["bu", "uv"]);
    let (stacked, _) = routed(true);
    assert!(stacked);
}

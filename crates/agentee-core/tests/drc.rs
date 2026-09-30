use agentee_core::Project;
use agentee_core::diag::Severity;
use std::path::Path;

fn project(parts: &[(&str, &str)], nets: &str, pcb: &str) -> Project {
    project_with(&[], parts, nets, pcb)
}

fn project_with(files: &[(&str, &str)], parts: &[(&str, &str)], nets: &str, pcb: &str) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-drc-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for s in ["R", "MountingHole_Pad"] {
        let f = format!("symbols/{s}.sym.toml");
        std::fs::copy(lna.join(&f), dir.join(&f)).unwrap();
    }
    for s in ["R_0402_1005Metric", "MountingHole_2.2mm_M2_Pad_Via"] {
        let f = format!("footprints/{s}.fp.toml");
        std::fs::copy(lna.join(&f), dir.join(&f)).unwrap();
    }
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        r#"name = "t"
fab = "jlcpcb"
[outline]
size = [30, 20]
[stackup]
preset = "jlcpcb-4l-1.6mm-7628"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.15mm"
via = "std"
"#,
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    for (i, (r, sym)) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"{sym}\"\nvalue = \"0\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
    }
    sch += nets;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    let pcb = format!("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n{pcb}");
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    Project::load(&dir).unwrap()
}

fn errors(p: &Project) -> Vec<String> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}: {}", d.at, d.message))
        .collect()
}

const RESISTORS: &[(&str, &str)] = &[("R1", "R"), ("R2", "R")];

fn placed(r1: [f64; 2], r2: [f64; 2], extra: &str) -> String {
    format!(
        "[[footprints]]\nref = \"R1\"\nat = [{}, {}]\n\n[[footprints]]\nref = \"R2\"\nat = [{}, {}]\n{extra}\n",
        r1[0], r1[1], r2[0], r2[1]
    )
}

#[test]
fn silk_text_over_pads_or_other_text_is_an_error_with_a_spot() {
    let p = project(RESISTORS, "", &placed([10.0, 10.0], [10.0, 11.2], ""));
    let e = errors(&p);
    assert!(e.iter().any(|t| t.contains("silk R2") && t.contains("sits on pads R1.")), "{e:?}");
    assert!(e.iter().any(|t| t.contains("silk R2") && t.contains("label = { at")), "{e:?}");
    assert!(p.layouts[0].item.label_fixes.iter().any(|f| f.reference == "R2" && f.at.is_some()));

    let p = project(RESISTORS, "", &placed([10.0, 10.0], [10.3, 10.6], ""));
    let e = errors(&p);
    assert!(e.iter().any(|t| t.contains("crowds `R")), "{e:?}");

    let clear = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\nlabel = { at = [10, 8.5] }\n\n\
                 [[footprints]]\nref = \"R2\"\nat = [16, 10]\nlabel = { at = [16, 8.5] }\n";
    let p = project(RESISTORS, "", clear);
    let e = errors(&p);
    assert!(!e.iter().any(|t| t.contains("silk")), "{e:?}");
    assert!(p.layouts[0].item.label_fixes.is_empty());
}

#[test]
fn silk_text_off_the_board_or_over_a_via_is_an_error() {
    let p = project(RESISTORS, "", &placed([10.0, 0.9], [16.0, 10.0], ""));
    let e = errors(&p);
    assert!(e.iter().any(|t| t.contains("silk R1") && t.contains("runs off the board")), "{e:?}");

    let nets = "\n[[nets]]\nname = \"A\"\npins = [\"R2.1\"]\n";
    let via = "\n[[vias]]\nnet = \"A\"\nat = [16.0, 8.83]\n";
    let p = project(RESISTORS, nets, &placed([10.0, 10.0], [16.0, 10.0], via));
    let e = errors(&p);
    assert!(e.iter().any(|t| t.contains("silk R2") && t.contains("prints over 1 via")), "{e:?}");
}

#[test]
fn courtyards_overlap_by_their_outline_on_the_same_side() {
    let e = errors(&project(RESISTORS, "", &placed([10.0, 10.0], [11.5, 10.0], "")));
    assert!(e.iter().any(|t| t.contains("part R1: courtyard overlaps R2 on F.CrtYd")), "{e:?}");

    let e = errors(&project(RESISTORS, "", &placed([10.0, 10.0], [11.86, 10.0], "")));
    assert!(!e.iter().any(|t| t.contains("courtyard")), "{e:?}");

    let bottom = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\n\n\
                  [[footprints]]\nref = \"R2\"\nat = [11.5, 10]\nside = \"bottom\"\n";
    let e = errors(&project(RESISTORS, "", bottom));
    assert!(!e.iter().any(|t| t.contains("courtyard")), "{e:?}");

    let turned = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\nrotation = 45\n\n\
                  [[footprints]]\nref = \"R2\"\nat = [11.9, 11.2]\n";
    let e = errors(&project(RESISTORS, "", turned));
    assert!(!e.iter().any(|t| t.contains("courtyard")), "{e:?}");
}

#[test]
fn a_courtyard_over_a_mounting_hole_is_an_error_on_either_side() {
    let parts = &[("R1", "R"), ("R2", "R"), ("H1", "MountingHole_Pad")];
    let hole = "\n[[footprints]]\nref = \"H1\"\nat = [20, 10]\n";
    let under = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\n\n\
                 [[footprints]]\nref = \"R2\"\nat = [21, 10]\nside = \"bottom\"\n";
    let e = errors(&project(parts, "", &format!("{under}{hole}")));
    assert!(
        e.iter().any(|t| t.contains("part R2: courtyard on B.CrtYd covers the mounting hole H1")),
        "{e:?}"
    );

    let beside = under.replace("side = \"bottom\"\n", "");
    let e = errors(&project(parts, "", &format!("{beside}{hole}")));
    assert!(e.iter().any(|t| t.contains("courtyard overlaps") && t.contains("H1")), "{e:?}");
    assert!(!e.iter().any(|t| t.contains("covers the mounting hole")), "{e:?}");

    let clear = under.replace("[21, 10]", "[24, 10]");
    let e = errors(&project(parts, "", &format!("{clear}{hole}")));
    assert!(!e.iter().any(|t| t.contains("courtyard")), "{e:?}");
}

fn frame_footprint() -> String {
    let mut f = String::from("name = \"Frame\"\n");
    for (n, x) in [("1", -5.5), ("2", 5.5)] {
        f += &format!(
            "[[pads]]\nnumber = \"{n}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [{x}, 0]\nsize = [0.5, 0.5]\n"
        );
    }
    for h in [6.0, 5.0] {
        let c = [[-h, -h], [h, -h], [h, h], [-h, h]];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            f += &format!(
                "[[graphics]]\nkind = \"line\"\nlayer = \"F.CrtYd\"\nstart = [{}, {}]\nend = [{}, {}]\n",
                a[0], a[1], b[0], b[1]
            );
        }
    }
    f
}

#[test]
fn a_ring_courtyard_leaves_its_inside_free_for_other_parts() {
    let frame = frame_footprint();
    let files = [("footprints/Frame.fp.toml", frame.as_str())];
    let sch = "\n[[parts]]\nref = \"J1\"\nsymbol = \"R\"\nvalue = \"0\"\nat = [10.16, 20.32]\n\
               footprint = \"Frame\"\n\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"0\"\n\
               at = [20.32, 20.32]\n";
    let pcb = |x: f64| {
        format!(
            "[[footprints]]\nref = \"J1\"\nat = [15, 10]\n\n[[footprints]]\nref = \"R1\"\nat = [{x}, 10]\n"
        )
    };
    let e = errors(&project_with(&files, &[], sch, &pcb(15.0)));
    assert!(!e.iter().any(|t| t.contains("courtyard overlaps")), "{e:?}");
    let e = errors(&project_with(&files, &[], sch, &pcb(19.5)));
    assert!(e.iter().any(|t| t.contains("courtyard overlaps")), "{e:?}");
}

const BRIDGE: &str = r#"name = "Bridge"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [-0.65, 0]
size = [1.0, 1.5]
layers = ["F.Cu", "F.Mask"]
[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [0.65, 0]
size = [1.0, 1.5]
layers = ["F.Cu", "F.Mask"]
[[graphics]]
kind = "polygon"
layer = "F.Cu"
points = [[-0.25, -0.3], [0.25, -0.3], [0.25, 0.3], [-0.25, 0.3]]
fill = "solid"
[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-1.4, -1]
end = [1.4, 1]
"#;

#[test]
fn footprint_copper_joins_the_pads_it_bridges() {
    let files = [("footprints/Bridge.fp.toml", BRIDGE)];
    let sch = "\n[[parts]]\nref = \"JP1\"\nsymbol = \"R\"\nvalue = \"0\"\nat = [10.16, 20.32]\n\
               footprint = \"Bridge\"\n\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"0\"\n\
               at = [20.32, 20.32]\n\n[[parts]]\nref = \"R2\"\nsymbol = \"R\"\nvalue = \"0\"\n\
               at = [30.48, 20.32]\n\n[[nets]]\nname = \"A\"\npins = [\"R1.1\", \"JP1.1\", \"R2.1\"]\n\n\
               [[nets]]\nname = \"B\"\npins = [\"JP1.2\"]\n\n[[nets]]\nname = \"C\"\npins = [\"R1.2\"]\n";
    let parts = "[[footprints]]\nref = \"JP1\"\nat = [15, 10]\n\n[[footprints]]\nref = \"R1\"\n\
                 at = [10, 10]\n\n[[footprints]]\nref = \"R2\"\nat = [20, 10]\n\n\
                 [[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\n\
                 points = [[9.49, 10], [9.49, 12], [14.35, 12], [14.35, 10]]\n\n\
                 [[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\n\
                 points = [[15, 10], [15, 14], [19.49, 14], [19.49, 10]]\n";
    let e = errors(&project_with(&files, &[], sch, parts));
    assert!(!e.iter().any(|t| t.contains("unrouted") || t.contains("short")), "{e:?}");

    let across = format!(
        "{parts}\n[[tracks]]\nnet = \"C\"\nlayer = \"F.Cu\"\npoints = [[15, 7], [15, 8.5]]\n\
         \n[[tracks]]\nnet = \"C\"\nlayer = \"F.Cu\"\npoints = [[15, 8.5], [15, 9.8]]\n"
    );
    let e = errors(&project_with(&files, &[], sch, &across));
    assert!(e.iter().any(|t| t.contains("JP1 copper on F.Cu touches track")), "{e:?}");

    let cut = parts.replace("[15, 10], [15, 14]", "[15, 10.6], [15, 14]");
    let e = errors(&project_with(&files, &[], sch, &cut));
    assert!(e.iter().any(|t| t.contains("unrouted")), "{e:?}");
}

#[test]
fn a_footprint_clearance_replaces_the_class_clearance_for_its_pads() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let r = std::fs::read_to_string(lna.join("footprints/R_0402_1005Metric.fp.toml")).unwrap();
    let loose = r.replacen("\n[[pads]]", "\nclearance = \"0.1mm\"\n\n[[pads]]", 1);
    let track =
        "\n[[tracks]]\nnet = \"B\"\nlayer = \"F.Cu\"\npoints = [[8, 10.54], [9.5, 10.54]]\n";
    let pcb = placed([10.0, 10.0], [20.0, 10.0], track);
    let e = errors(&project(RESISTORS, TWO_NETS, &pcb));
    assert!(e.iter().any(|t| t.contains("R1.1 is 0.12mm from track 0 (B), needs 0.15mm")), "{e:?}");
    let files = [("footprints/R_0402_1005Metric.fp.toml", loose.as_str())];
    let e = errors(&project_with(&files, RESISTORS, TWO_NETS, &pcb));
    assert!(!e.iter().any(|t| t.starts_with("clearance")), "{e:?}");
}

const TIGHT: &str = r#"name = "Tight2"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [-0.19, 0]
size = [0.3, 0.3]
[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [0.19, 0]
size = [0.3, 0.3]
[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-0.5, -0.3]
end = [0.5, 0.3]
"#;

#[test]
fn mask_openings_of_different_nets_need_a_web() {
    let files = [("footprints/Tight2.fp.toml", TIGHT)];
    let sch = |b: &str| {
        format!(
            "\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"0\"\nat = [10.16, 20.32]\n\
             footprint = \"Tight2\"\n\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"{b}]\n"
        )
    };
    let pcb = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\nlabel = { hide = true }\n";
    let split = format!("{}\n[[nets]]\nname = \"B\"\npins = [\"R1.2\"]\n", sch(""));
    let e = errors(&project_with(&files, &[], &split, pcb));
    assert!(
        e.iter().any(|t| t.contains("part R1 F.Mask: 1 pad pairs")
            && t.contains("0.1mm mask web")
            && t.contains("R1.1 leaves 0.08mm to R1.2 at [10.000, 10.000]")),
        "{e:?}"
    );
    let e = errors(&project_with(&files, &[], &sch(", \"R1.2\""), pcb));
    assert!(!e.iter().any(|t| t.contains("mask")), "{e:?}");
}

#[test]
fn a_footprint_can_opt_out_of_the_mask_web_within_itself() {
    let merged = format!("mask_web = false\n{TIGHT}");
    let files = [("footprints/Tight2.fp.toml", merged.as_str())];
    let sch = "\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"0\"\nat = [10.16, 20.32]\n\
               footprint = \"Tight2\"\n\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n\n\
               [[nets]]\nname = \"B\"\npins = [\"R1.2\"]\n";
    let pcb = "[[footprints]]\nref = \"R1\"\nat = [10, 10]\nlabel = { hide = true }\n";
    let p = project_with(&files, &[], sch, pcb);
    assert!(p.layouts[0].item.parts.iter().any(|q| !q.footprint.mask_web));
    let e = errors(&p);
    assert!(!e.iter().any(|t| t.contains("mask web")), "{e:?}");
}

const TWO_NETS: &str =
    "\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n\n[[nets]]\nname = \"B\"\npins = [\"R2.1\"]\n";

fn with_copper(copper: &str) -> Vec<String> {
    errors(&project(RESISTORS, TWO_NETS, &placed([10.0, 10.0], [20.0, 10.0], copper)))
}

#[test]
fn a_track_too_close_to_a_via_of_another_net_is_an_error() {
    let track = "\n[[tracks]]\nnet = \"A\"\nlayer = \"F.Cu\"\npoints = [[5, 15], [10, 15]]\n";
    let near = format!("{track}\n[[vias]]\nnet = \"B\"\nat = [7.5, 15.5]\n");
    let e = with_copper(&near);
    assert!(
        e.iter().any(|t| t.starts_with("clearance")
            && t.contains("track 0 (A) is 0.1mm from via at [7.500, 15.500] (B), needs 0.15mm")),
        "{e:?}"
    );
    let far = format!("{track}\n[[vias]]\nnet = \"B\"\nat = [7.5, 15.6]\n");
    let e = with_copper(&far);
    assert!(!e.iter().any(|t| t.starts_with("clearance")), "{e:?}");
}

#[test]
fn vias_of_different_nets_keep_clearance_and_hole_spacing() {
    let e = with_copper(
        "\n[[vias]]\nnet = \"A\"\nat = [5, 5]\n\n[[vias]]\nnet = \"B\"\nat = [5.7, 5]\n",
    );
    assert!(
        e.iter().any(|t| t.starts_with("clearance")
            && t.contains("via at [5.000, 5.000] (A) is 0.1mm from via at [5.700, 5.000] (B)")),
        "{e:?}"
    );
    assert!(e.iter().any(|t| t.starts_with("drills") && t.contains("1 drill pairs")), "{e:?}");
    let e = with_copper(
        "\n[[vias]]\nnet = \"A\"\nat = [5, 5]\n\n[[vias]]\nnet = \"B\"\nat = [5, 5.3]\n",
    );
    assert!(e.iter().any(|t| t.starts_with("short") && t.contains("touches")), "{e:?}");
}

#[test]
fn a_via_on_a_pad_of_another_net_is_a_short() {
    let e = with_copper("\n[[vias]]\nnet = \"B\"\nat = [9.49, 10]\n");
    assert!(
        e.iter().any(
            |t| t.starts_with("short") && t.contains("R1.1 touches via at [9.490, 10.000] (B)")
        ),
        "{e:?}"
    );
    let e = with_copper("\n[[vias]]\nnet = \"A\"\nat = [9.49, 10]\n");
    assert!(!e.iter().any(|t| t.starts_with("short")), "{e:?}");
}

#[test]
fn same_net_vias_on_the_same_spot_are_an_error() {
    let e =
        with_copper("\n[[vias]]\nnet = \"A\"\nat = [5, 5]\n\n[[vias]]\nnet = \"A\"\nat = [5, 5]\n");
    assert!(e.iter().any(|t| t.starts_with("vias") && t.contains("[5.000, 5.000] (A)")), "{e:?}");
    let e = with_copper("\n[[vias]]\nnet = \"A\"\nat = [5, 5]\ncount = 2\npitch = [0, 0.1]\n");
    assert!(e.iter().any(|t| t.starts_with("drills")), "{e:?}");
    let e =
        with_copper("\n[[vias]]\nnet = \"A\"\nat = [5, 5]\n\n[[vias]]\nnet = \"A\"\nat = [6, 5]\n");
    assert!(!e.iter().any(|t| t.starts_with("vias") || t.starts_with("drills")), "{e:?}");
}

#[test]
#[ignore]
fn sdr_in6_ground_has_no_stubs_between_the_u3_antipads() {
    let sdr = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sdr");
    let p = Project::load(&sdr).unwrap();
    let layout = &p.layouts[0].item;
    let gnd = layout.nets.iter().position(|n| n.name == "GND").unwrap();
    let fill = layout.zones.iter().find(|z| z.layer == "In6.Cu" && z.net == gnd).unwrap();
    let copper = |q: [f64; 2]| {
        fill.rings.iter().filter(|r| agentee_core::geom::point_in_polygon(q, r)).count() % 2 == 1
    };
    let (m, across) = ([64.75, 43.15], [-0.196, 0.981]);
    for k in -20..=20 {
        let s = k as f64 * 0.01;
        let q = [m[0] + across[0] * s, m[1] + across[1] * s];
        assert!(!copper(q), "copper at {q:?}");
    }
    assert!(copper([64.7, 43.7]));
}

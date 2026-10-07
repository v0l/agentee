use agentee_core::Project;
use agentee_core::diag::Severity;
use agentee_core::testpoint;
use std::path::Path;

struct Part<'a> {
    reference: &'a str,
    symbol: &'a str,
    footprint: &'a str,
    at: [f64; 2],
    side: &'a str,
}

fn part<'a>(reference: &'a str, at: [f64; 2]) -> Part<'a> {
    Part { reference, symbol: "R", footprint: "R_0402_1005Metric", at, side: "top" }
}

fn tp(reference: &str, at: [f64; 2]) -> Part<'_> {
    Part {
        reference,
        symbol: testpoint::SYMBOL,
        footprint: testpoint::PAD_FOOTPRINT,
        at,
        side: "bottom",
    }
}

fn load(board: &str, parts: &[Part], nets: &[(&str, &str, &[&str])], pcb: &str) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-tp-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(dir.join("symbols/TestPoint.sym.toml"), testpoint::SYMBOL_TOML).unwrap();
    std::fs::write(
        dir.join("footprints/TestPoint_Pad_D1.0mm.fp.toml"),
        testpoint::PAD_FOOTPRINT_TOML,
    )
    .unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 30]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n{board}"
        ),
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut layout = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, p) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{}\"\nsymbol = \"{}\"\nvalue = \"x\"\nfootprint = \"{}\"\nat = [{}, 20.32]\n",
            p.reference,
            p.symbol,
            p.footprint,
            10.16 * (i + 1) as f64
        );
        layout += &format!(
            "\n[[footprints]]\nref = \"{}\"\nat = [{}, {}]\nside = \"{}\"\n",
            p.reference, p.at[0], p.at[1], p.side
        );
    }
    for (n, class, pins) in nets {
        let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
        sch += &format!(
            "\n[[nets]]\nname = \"{n}\"\nclass = \"{class}\"\npins = [{}]\n",
            pins.join(", ")
        );
    }
    layout += pcb;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), layout).unwrap();
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

const RAIL_AND_SIGNAL: &[(&str, &str, &[&str])] =
    &[("3V3", "Default", &["R1.1"]), ("SIG", "Default", &["R1.2"])];

#[test]
fn test_access_lists_rails_and_control_nets_without_a_probe() {
    let p = load("", &[part("R1", [20.0, 15.0])], RAIL_AND_SIGNAL, "");
    let h = hits(&p, "test-access");
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].0, Severity::Info);
    assert!(h[0].1.contains("1 nets have no probe access from B: 3V3;"), "{}", h[0].1);
}

#[test]
fn switch_nodes_and_regulator_control_nets_get_no_default_probe() {
    let board = "[[netclasses]]\nname = \"Power\"\ntrack_width = \"0.5mm\"\nclearance = \"0.2mm\"\ncurrent = \"2A\"\n";
    let nets: &[(&str, &str, &[&str])] = &[
        ("/buck/SW", "Power", &["R1.1"]),
        ("FB_3V3", "Default", &["R1.2"]),
        ("VCC_EN", "Default", &["R2.1"]),
        ("NR_LDO", "Default", &["R2.2"]),
    ];
    let parts = &[part("R1", [20.0, 15.0]), part("R2", [25.0, 15.0])];
    let h = hits(&load(board, parts, nets, ""), "test-access");
    assert!(
        h.len() == 1 && h[0].1.contains("1 nets have no probe access from B: VCC_EN;"),
        "{h:?}"
    );
    let h = hits(&load(board, parts, nets, "[test]\nnets = [\"FB*\"]\n"), "test-access");
    assert!(h.len() == 1 && h[0].1.contains("from B: FB_3V3;"), "{h:?}");
}

#[test]
fn a_test_point_on_the_net_gives_access() {
    let nets: &[(&str, &str, &[&str])] =
        &[("3V3", "Default", &["R1.1", "TP1.1"]), ("SIG", "Default", &["R1.2"])];
    let p = load("", &[part("R1", [20.0, 15.0]), tp("TP1", [15.0, 15.0])], nets, "");
    assert!(hits(&p, "test-access").is_empty());
    assert!(hits(&p, "test-pad-geometry").is_empty(), "{:?}", hits(&p, "test-pad-geometry"));
}

#[test]
fn the_test_section_picks_nets_and_exempts_impedance_classes() {
    let board = "[[netclasses]]\nname = \"RF\"\ntrack_width = \"0.3mm\"\nclearance = \"0.2mm\"\nimpedance = \"50ohm\"\n";
    let nets: &[(&str, &str, &[&str])] = &[("3V3", "Default", &["R1.1"]), ("SIG", "RF", &["R1.2"])];
    let p = load(
        board,
        &[part("R1", [20.0, 15.0])],
        nets,
        "[test]\nnets = [\"*\"]\nexclude = [\"3V*\"]\n",
    );
    let h = hits(&p, "test-access");
    assert!(h.is_empty(), "{h:?}");
    let p = load(board, &[part("R1", [20.0, 15.0])], nets, "[test]\nnets = [\"s*\", \"3v3\"]\n");
    let h = hits(&p, "test-access");
    assert!(h[0].1.contains("from B: 3V3;") && h[0].1.contains("SIG (impedance class)"), "{h:?}");
}

#[test]
fn test_pad_geometry_flags_size_pitch_edge_body_and_side() {
    let nets: &[(&str, &str, &[&str])] = &[
        ("3V3", "Default", &["R1.1", "TP1.1", "TP2.1"]),
        ("SIG", "Default", &["R1.2", "TP3.1", "TP4.1"]),
    ];
    let mut top = tp("TP4", [30.0, 10.0]);
    top.side = "top";
    let parts = [
        {
            let mut r = part("R1", [20.0, 15.0]);
            r.side = "bottom";
            r
        },
        tp("TP1", [10.0, 15.0]),
        tp("TP2", [11.0, 15.0]),
        tp("TP3", [1.5, 20.0]),
        top,
    ];
    let p = load("", &parts, nets, "[test]\nmin_test_pad = \"1.2mm\"\n");
    let h: Vec<String> = hits(&p, "test-pad-geometry").into_iter().map(|h| h.1).collect();
    let all = h.join("\n");
    assert!(all.contains("TP1.1 pad is 1mm across, under min_test_pad 1.2mm"), "{all}");
    assert!(all.contains("TP1 and TP2 are 1mm apart centre to centre"), "{all}");
    assert!(all.contains("TP3.1") && all.contains("from the board edge"), "{all}");
    assert!(all.contains("TP4.1") && all.contains("is not on the probe side B"), "{all}");
    let p = load("", &[parts[0].clone_at([20.0, 15.0]), tp("TP1", [21.0, 15.0])], nets, "");
    let all: Vec<String> = hits(&p, "test-pad-geometry").into_iter().map(|h| h.1).collect();
    assert!(all.iter().any(|m| m.contains("min_test_pad_to_body 1mm to R1")), "{all:?}");
}

impl Part<'_> {
    fn clone_at(&self, at: [f64; 2]) -> Part<'_> {
        Part { at, ..*self }
    }
}

#[test]
fn test_points_resolve_with_the_default_spec() {
    let p = load("", &[part("R1", [20.0, 15.0])], RAIL_AND_SIGNAL, "");
    let spec = &p.layouts[0].item.test;
    assert_eq!((spec.side.as_str(), spec.min_test_pad, spec.min_test_pad_pitch), ("B", 1.0, 1.27));
    assert_eq!((spec.min_test_pad_to_body, spec.min_test_pad_to_edge), (1.0, 3.0));
}

#[test]
fn a_pair_class_net_with_no_partner_still_needs_a_probe() {
    let board = "[[netclasses]]\nname = \"Pair\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\ndiff_gap = \"0.2mm\"\n";
    let nets: &[(&str, &str, &[&str])] =
        &[("MCU_EN", "Pair", &["R1.1"]), ("SIG", "Default", &["R1.2"])];
    let p = load(board, &[part("R1", [20.0, 15.0])], nets, "[test]\nnets = [\"*EN\"]\n");
    let h = hits(&p, "test-access");
    assert!(h.iter().any(|h| h.1.contains("from B: MCU_EN;")), "{h:?}");
}

#[test]
fn placed_test_pads_keep_clear_of_each_other() {
    let nets: &[(&str, &str, &[&str])] = &[
        ("3V3", "Default", &["R1.1", "R2.1"]),
        ("1V8", "Default", &["R1.2", "R2.2"]),
        ("VBUS", "Default", &["R3.1"]),
        ("VBAT", "Default", &["R3.2"]),
    ];
    let parts = [part("R1", [20.0, 15.0]), part("R2", [21.5, 15.0]), part("R3", [20.5, 16.5])];
    let p = load("", &parts, nets, "");
    let layout = &p.layouts[0].item;
    let board = &p.boards[0].item;
    let targets: Vec<usize> = (0..layout.nets.len()).collect();
    let spots = testpoint::place(layout, board, &layout.test, &targets, 1.27);
    let placed: Vec<([f64; 2], [f64; 2], usize)> =
        spots.iter().filter_map(|s| Some((s.at?, s.via?, s.net))).collect();
    assert!(placed.len() >= 3, "{}", placed.len());
    let (pad_r, via_r, gap) = (0.5, 0.3, 0.15 - 1e-6);
    let d = agentee_core::geom::dist;
    let seg = agentee_core::geom::segment_segment_distance;
    for (i, a) in placed.iter().enumerate() {
        for b in &placed[i + 1..] {
            assert!(d(a.0, b.0) - 2.0 * pad_r >= gap, "{a:?} {b:?}");
            assert!(d(a.0, b.1) - pad_r - via_r >= gap, "{a:?} {b:?}");
            assert!(d(a.1, b.0) - pad_r - via_r >= gap, "{a:?} {b:?}");
            assert!(d(a.1, b.1) - 2.0 * via_r >= gap, "{a:?} {b:?}");
            assert!(seg(a.0, a.1, b.0, b.1) - 0.2 >= gap, "{a:?} {b:?}");
        }
    }
}

#[test]
fn a_glob_fanout_leaves_test_points_alone() {
    let nets: &[(&str, &str, &[&str])] =
        &[("3V3", "Default", &["R1.1", "TP1.1"]), ("SIG", "Default", &["R1.2"])];
    let parts = [part("R1", [20.0, 15.0]), tp("TP1", [12.0, 15.0])];
    let fanout = "\n[[fanouts]]\nref = \"*\"\nnets = [\"3V3\"]\n";
    let p = load("", &parts, nets, fanout);
    let vias = &p.layouts[0].item.vias;
    assert!(!vias.iter().any(|v| agentee_core::geom::dist(v.at, [12.0, 15.0]) < 0.5), "{vias:?}");
    let p = load("", &parts, nets, "\n[[fanouts]]\nref = \"TP1\"\n");
    let vias = &p.layouts[0].item.vias;
    assert!(vias.iter().any(|v| agentee_core::geom::dist(v.at, [12.0, 15.0]) < 0.5), "{vias:?}");
}

#[test]
fn a_net_with_copper_on_the_probe_side_gets_a_stub_and_no_via() {
    let parts = [Part { side: "bottom", ..part("R1", [20.0, 15.0]) }];
    let p = load("", &parts, RAIL_AND_SIGNAL, "");
    let layout = &p.layouts[0].item;
    let r1 = layout.parts.iter().find(|q| q.reference == "R1").unwrap();
    let pad = r1.pads.iter().find(|q| q.net.is_some()).unwrap();
    let c = testpoint::pad_center(pad);
    let net = layout.nets[pad.net.unwrap()].name.clone();
    let track = format!(
        "\n[[tracks]]\nnet = \"{net}\"\nlayer = \"B.Cu\"\npoints = [[{}, {}], [{}, {}]]\n",
        c[0],
        c[1],
        c[0],
        c[1] + 4.0
    );
    let p = load("", &parts, RAIL_AND_SIGNAL, &track);
    let layout = &p.layouts[0].item;
    let ni = layout.nets.iter().position(|n| n.name == net).unwrap();
    let spots = testpoint::place(layout, &p.boards[0].item, &layout.test, &[ni], 1.27);
    let s = &spots[0];
    let (at, stub) = (s.at.unwrap(), s.stub.unwrap());
    assert!(s.via.is_none(), "a via was added at {:?}", s.via);
    let on_track = agentee_core::geom::point_segment_distance(stub, c, [c[0], c[1] + 4.0]);
    let on_pad = agentee_core::geom::dist(stub, c);
    assert!(on_track < 1e-3 || on_pad < 1e-3, "stub from {at:?} ends at {stub:?}");
}

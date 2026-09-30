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

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

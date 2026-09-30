use agentee_core::Project;
use agentee_core::diag::Severity;
use std::path::Path;

fn project(parts: &[(&str, &str)], nets: &str, pcb: &str) -> Project {
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

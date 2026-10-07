use agentee_core::Project;
use agentee_core::geom::{P, dist, point_in_polygon, polyline_polygon_distance};
use agentee_core::route::{RouteOptions, route};
use std::path::{Path, PathBuf};

const BIG: &str = r#"name = "Big"
mount = "smd"

[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [0.0, -3.0]
size = [3.0, 3.0]
layers = ["F.Cu", "F.Paste", "F.Mask"]

[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [0.0, 3.0]
size = [3.0, 3.0]
layers = ["F.Cu", "F.Paste", "F.Mask"]
"#;

const HOLDER: &str = r#"name = "Holder"
mount = "tht"

[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [0.0, -2.5]
size = [2.0, 2.0]
drill = 1.0
layers = ["*.Cu", "*.Mask"]

[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [0.0, 2.5]
size = [2.0, 2.0]
drill = 1.0
layers = ["*.Cu", "*.Mask"]

[[pads]]
number = "2"
kind = "tht"
shape = "circle"
at = [8.0, 0.0]
size = [2.0, 2.0]
drill = 1.0
layers = ["*.Cu", "*.Mask"]
"#;

fn project() -> (PathBuf, Project) {
    let dir = std::env::temp_dir().join(format!("agentee-route-ends-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(dir.join("footprints/Big.fp.toml"), BIG).unwrap();
    std::fs::write(dir.join("footprints/Holder.fp.toml"), HOLDER).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 24]\n[stackup]\n\
         preset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\n\
         diameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.25mm\"\n\
         clearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    let parts =
        [("U1", "Big", [6.0, 12.0]), ("U2", "Big", [20.0, 8.0]), ("F1", "Holder", [26.0, 14.0])];
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, fp, at)) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"x\"\nfootprint = \"{fp}\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!(
            "\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\nlabel = {{ hide = true }}\n",
            at[0], at[1]
        );
    }
    for (n, pins) in [
        ("A", "\"U1.2\", \"U2.1\""),
        ("B", "\"U2.2\", \"F1.1\""),
        ("C", "\"U1.1\""),
        ("D", "\"F1.2\""),
    ] {
        sch += &format!("\n[[nets]]\nname = \"{n}\"\nclass = \"Default\"\npins = [{pins}]\n");
    }
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    (dir, p)
}

#[test]
fn routed_ends_finish_on_pad_centres() {
    let (dir, p) = project();
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let opts = RouteOptions { nets: vec!["A".into(), "B".into()], ..Default::default() };
    let r = route(layout, board, &opts).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    let pads: Vec<(String, P, &Vec<P>, &Vec<String>)> = layout
        .parts
        .iter()
        .flat_map(|q| {
            q.pads.iter().flat_map(move |x| {
                x.outlines.iter().map(move |o| {
                    (
                        format!("{}.{}", q.reference, x.number),
                        agentee_core::testpoint::pad_center(x),
                        o,
                        &x.copper,
                    )
                })
            })
        })
        .collect();
    let mut ended: Vec<String> = Vec::new();
    for t in &r.tracks {
        for end in [t.points[0], *t.points.last().unwrap()] {
            for (name, c, o, layers) in &pads {
                let on =
                    point_in_polygon(end, o) || polyline_polygon_distance(&[end, end], o) < 1e-6;
                if layers.contains(&t.layer) && on {
                    assert!(
                        dist(end, *c) < 1e-6,
                        "{} ends at {end:?} in {name}, centre {c:?}",
                        t.net
                    );
                    ended.push(format!("{name}@{:.2},{:.2}", c[0], c[1]));
                }
            }
        }
    }
    ended.sort();
    ended.dedup();
    assert_eq!(ended.len(), 4, "{ended:?}");
}

#[test]
fn routes_keep_off_an_inner_plane_of_another_net() {
    let dir = std::env::temp_dir().join(format!("agentee-route-plane-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\n\
         preset = \"jlcpcb-4l-1.6mm-7628\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\n\
         diameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\n\
         clearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, at)) in
        [("R1", [5.0, 10.0]), ("R2", [25.0, 10.0]), ("R3", [5.0, 3.0])].iter().enumerate()
    {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"x\"\nfootprint = \"R_0402_1005Metric\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!(
            "\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\nlabel = {{ hide = true }}\n",
            at[0], at[1]
        );
    }
    for (n, pins) in [
        ("A", "\"R1.2\", \"R2.1\""),
        ("B", "\"R1.1\""),
        ("C", "\"R2.2\""),
        ("W", "\"R3.1\""),
        ("GND", "\"R3.2\""),
    ] {
        sch += &format!("\n[[nets]]\nname = \"{n}\"\nclass = \"Default\"\npins = [{pins}]\n");
    }
    for layer in ["F.Cu", "B.Cu"] {
        pcb += &format!(
            "\n[[tracks]]\nnet = \"W\"\nlayer = \"{layer}\"\nwidth = 0.5\npoints = [[15.0, -1.0], [15.0, 21.0]]\n"
        );
    }
    pcb += "\n[[vias]]\nnet = \"GND\"\nat = [8.0, 3.0]\n";
    pcb += "\n[[tracks]]\nnet = \"GND\"\nlayer = \"F.Cu\"\npoints = [[5.51, 3.0], [8.0, 3.0]]\n";
    pcb += "\n[[zones]]\nnet = \"GND\"\nlayers = [\"In1.Cu\"]\n";
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let filled = |z: &agentee_core::layout::ZoneFill| z.mask.iter().filter(|&&m| m != 0).count();
    assert!(
        layout.zones.iter().any(|z| z.layer == "In1.Cu" && filled(z) > 1000),
        "the plane did not fill"
    );
    let opts = RouteOptions { nets: vec!["A".into()], ..Default::default() };
    let r = route(layout, board, &opts).unwrap();
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    let on = |l: &str| r.tracks.iter().filter(|t| t.layer == l).count();
    assert_eq!(on("In1.Cu"), 0, "{:?}", r.tracks);
    assert!(on("In2.Cu") > 0, "{:?}", r.tracks);
}

use agentee_core::place::{self, PlaceInput, PlaceOptions, Sides};
use agentee_core::project::Project;
use std::collections::HashMap;

const THT: &str = r#"name = "BIG"
mount = "tht"
[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [-4.0, 0.0]
size = [2.0, 2.0]
drill = 1.0
[[pads]]
number = "2"
kind = "tht"
shape = "circle"
at = [4.0, 0.0]
size = [2.0, 2.0]
drill = 1.0
[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-5.5, -3.0]
end = [5.5, 3.0]
width = 0.05
"#;

const SMD: &str = r#"name = "SMALL"
mount = "smd"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [-0.5, 0.0]
size = [0.6, 0.6]
[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [0.5, 0.0]
size = [0.6, 0.6]
[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-1.0, -0.6]
end = [1.0, 0.6]
width = 0.05
"#;

fn symbol(name: &str) -> String {
    let mut s = format!("name = \"{name}\"\nreference = \"U\"\n");
    for i in 1..=2 {
        s += &format!(
            "[[pins]]\nnumber = \"{i}\"\ntype = \"passive\"\nat = [0.0, {}]\nside = \"left\"\nlength = 1.27\nunit = 1\n",
            i as f64 * 2.54
        );
    }
    s
}

#[test]
fn a_small_part_fits_on_the_back_under_a_through_hole_body() {
    let dir = std::env::temp_dir().join(format!("agentee-under-tht-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    std::fs::write(dir.join("symbols/S2.sym.toml"), symbol("S2")).unwrap();
    std::fs::write(dir.join("footprints/BIG.fp.toml"), THT).unwrap();
    std::fs::write(dir.join("footprints/SMALL.fp.toml"), SMD).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [15, 9]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"U1\"\nsymbol = \"S2\"\nvalue = \"x\"\nfootprint = \"BIG\"\nat = [10.16, 20.32]\n[[parts]]\nref = \"C1\"\nsymbol = \"S2\"\nvalue = \"x\"\nfootprint = \"SMALL\"\nat = [20.32, 20.32]\n[[nets]]\nname = \"A\"\npins = [\"U1.1\", \"C1.1\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"U1\"\nat = [7.5, 4.5]\nlocked = true\n[[footprints]]\nref = \"C1\"\nat = [7.5, 4.5]\n",
    )
    .unwrap();
    let p = Project::load(&dir).unwrap();
    let text = std::fs::read_to_string(&p.layouts[0].path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let layout = &p.layouts[0].item;
    let file: agentee_core::layout::LayoutFile = agentee_core::project::parse(&text).unwrap();
    let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
        p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect();
    let spec = place::PlaceFile::default();
    let input = PlaceInput {
        board: &p.boards[0].item,
        outline: &layout.outline,
        cutouts: &layout.board_cutouts,
        schematic: &p.schematics[0].item,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: Vec::new(),
        heat: Vec::new(),
        silk: Vec::new(),
        texts: Vec::new(),
        tracks: &[],
        vias: &[],
        budgets: Vec::new(),
        regions: Vec::new(),
    };
    let opts = PlaceOptions { sides: Sides::Both, ..Default::default() };
    let r = place::place(&input, &opts).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let c1 = r.placements.iter().find(|q| q.reference == "C1").expect("C1 placed");
    assert!(c1.bottom, "C1 at {:?} on the top", c1.at);
}

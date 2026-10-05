use agentee_core::Project;
use std::path::Path;

fn project(units: &str) -> (Project, std::path::PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fab-bom-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo");
    for f in ["symbols/LM358.sym.toml", "footprints/SOIC-8_3.9x4.9mm_P1.27mm.fp.toml"] {
        std::fs::copy(demo.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 30]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("t.sch.toml"), format!("name = \"t\"\nboard = \"t\"\n{units}"))
        .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"U1\"\nat = [20, 15]\n",
    )
    .unwrap();
    (Project::load(&dir).unwrap(), dir)
}

#[test]
fn a_part_drawn_as_several_units_is_one_bom_line() {
    let (p, dir) = project(
        "[[parts]]\nref = \"U1\"\nsymbol = \"LM358\"\nvalue = \"LM358\"\nat = [25.4, 25.4]\nfields = { mpn = \"TI LM358DR\", lcsc = \"C7950\" }\n[[parts]]\nref = \"U1\"\nsymbol = \"LM358\"\nunit = 2\nvalue = \"LM358\"\nat = [50.8, 25.4]\n",
    );
    let out = dir.join("fab");
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, &out)
        .unwrap();
    let bom = std::fs::read_to_string(out.join("bom.csv")).unwrap();
    assert_eq!(
        bom,
        "Qty,Designators,Value,Footprint,MPN\n1,U1,LM358,SOIC-8_3.9x4.9mm_P1.27mm,TI LM358DR\n"
    );
    let jlc = std::fs::read_to_string(out.join("bom-jlcpcb.csv")).unwrap();
    assert_eq!(
        jlc,
        "Comment,Designator,Footprint,LCSC Part #\nLM358,U1,SOIC-8_3.9x4.9mm_P1.27mm,C7950\n"
    );
}

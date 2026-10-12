use agentee_core::project::Project;

#[test]
fn two_pins_closer_than_their_class_route_out_on_the_footprint_clearance() {
    let dir = std::env::temp_dir().join(format!("agentee-fanprobe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let mut sym = String::from("name = \"S2\"\nreference = \"U\"\n");
    for i in 1..=2 {
        sym += &format!(
            "[[pins]]\nnumber = \"{i}\"\ntype = \"passive\"\nat = [0.0, {}]\nside = \"left\"\nlength = 1.27\nunit = 1\n",
            i as f64 * 2.54
        );
    }
    std::fs::write(dir.join("symbols/S2.sym.toml"), &sym).unwrap();
    let s1 = String::from(
        "name = \"S1\"\nreference = \"J\"\n[[pins]]\nnumber = \"1\"\ntype = \"passive\"\nat = [0.0, 0.0]\nside = \"left\"\nlength = 1.27\nunit = 1\n",
    );
    std::fs::write(dir.join("symbols/S1.sym.toml"), &s1).unwrap();
    std::fs::write(dir.join("footprints/PAIR.fp.toml"), "name = \"PAIR\"\nmount = \"smd\"\nclearance = \"0.9mm\"\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"rect\"\nat = [0.0, 0.0]\nsize = [0.6, 1.5]\n[[pads]]\nnumber = \"2\"\nkind = \"smd\"\nshape = \"rect\"\nat = [1.94, 0.0]\nsize = [0.6, 1.5]\n[[graphics]]\nkind = \"rect\"\nlayer = \"F.CrtYd\"\nstart = [-1.0, -1.5]\nend = [3.0, 1.5]\nwidth = 0.05\n").unwrap();
    std::fs::write(dir.join("footprints/ONE.fp.toml"), "name = \"ONE\"\nmount = \"smd\"\n[[pads]]\nnumber = \"1\"\nkind = \"smd\"\nshape = \"rect\"\nat = [0.0, 0.0]\nsize = [1.0, 1.0]\n").unwrap();
    std::fs::write(dir.join("t.board.toml"), "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 30]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.25mm\"\nclearance = \"0.2mm\"\nvia = \"std\"\nlayers = [\"F.Cu\"]\n[[netclasses]]\nname = \"HV\"\ntrack_width = \"0.4mm\"\nclearance = \"2.5mm\"\nvia = \"std\"\nlayers = [\"F.Cu\"]\n").unwrap();
    let sch = "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"U1\"\nsymbol = \"S2\"\nvalue = \"x\"\nfootprint = \"PAIR\"\nat = [10.16, 20.32]\n[[parts]]\nref = \"J1\"\nsymbol = \"S1\"\nvalue = \"x\"\nfootprint = \"ONE\"\nat = [20.32, 20.32]\n[[parts]]\nref = \"J2\"\nsymbol = \"S1\"\nvalue = \"x\"\nfootprint = \"ONE\"\nat = [30.48, 20.32]\n[[nets]]\nname = \"HB\"\nclass = \"HV\"\npins = [\"U1.1\", \"J1.1\"]\n[[nets]]\nname = \"RVCC\"\nclass = \"HV\"\npins = [\"U1.2\", \"J2.1\"]\n";
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    let pcb = "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"U1\"\nat = [19.0, 15.0]\n[[footprints]]\nref = \"J1\"\nat = [8.0, 25.0]\n[[footprints]]\nref = \"J2\"\nat = [32.0, 25.0]\n";
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    let opts = agentee_core::route::RouteOptions {
        nets: vec!["HB".into(), "RVCC".into()],
        grid: 0.05,
        ..Default::default()
    };
    let r = agentee_layout::autoroute::route(&p.layouts[0].item, &p.boards[0].item, &opts).unwrap();
    assert_eq!((r.routed, r.connections), (2, 2), "{:?}", r.failed);
    let mut routed = pcb.to_string();
    for t in &r.tracks {
        let pts: Vec<String> = t.points.iter().map(|q| format!("[{}, {}]", q[0], q[1])).collect();
        routed += &format!(
            "\n[[tracks]]\nnet = \"{}\"\nlayer = \"{}\"\nwidth = {}\npoints = [{}]\n",
            t.net,
            t.layer,
            t.width.unwrap_or(0.4),
            pts.join(", ")
        );
    }
    std::fs::write(dir.join("t.pcb.toml"), routed).unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let bad: Vec<String> = p.layouts[0]
        .diags
        .iter()
        .filter(|d| matches!(d.rule.as_deref(), Some("clearance" | "short")))
        .map(|d| d.message.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

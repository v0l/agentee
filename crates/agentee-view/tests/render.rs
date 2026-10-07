use agentee_core::project::Project;
use agentee_view::{RenderOptions, render_png};
use std::path::Path;

fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    (info.width, info.height, buf)
}

#[test]
fn every_example_item_renders_something() {
    for example in ["demo", "lna"] {
        renders_something(example);
    }
}

fn renders_something(example: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(example);
    let project = Project::load(&root).unwrap();
    assert!(project.failures.is_empty(), "{:?}", project.failures);
    let opts = RenderOptions { width: 480, height: 320, ..Default::default() };
    for item in project.all_refs() {
        let (w, h, px) = decode(&render_png(&project, item, &opts).unwrap());
        assert_eq!((w, h), (480, 320));
        let distinct: std::collections::HashSet<&[u8]> = px.chunks(4).collect();
        assert!(distinct.len() > 20, "{} rendered almost blank", project.name_of(item));
    }
}

fn substrate_share(cutouts: &str) -> f64 {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-view-cut-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            "name = \"t\"\n[outline]\nsize = [20, 10]\n{cutouts}[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("t.sch.toml"), "name = \"t\"\nboard = \"t\"\n").unwrap();
    std::fs::write(dir.join("t.pcb.toml"), "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n")
        .unwrap();
    let project = Project::load(&dir).unwrap();
    let opts = RenderOptions {
        width: 480,
        height: 320,
        panels: false,
        region: Some([9.5, 4.5, 10.5, 5.5]),
        ..Default::default()
    };
    let item = agentee_core::project::ItemRef::Layout(0);
    let (_, _, px) = decode(&render_png(&project, item, &opts).unwrap());
    let substrate = agentee_view::pcb::SUBSTRATE;
    let hits =
        px.chunks(4).filter(|c| c[..3] == [substrate.r(), substrate.g(), substrate.b()]).count();
    hits as f64 / (px.len() / 4) as f64
}

#[test]
fn a_board_cutout_renders_as_a_hole_in_the_substrate() {
    let plain = substrate_share("");
    assert!(plain > 0.5, "{plain}");
    let cut = substrate_share("[[outline.cutouts]]\norigin = [8, 3]\nsize = [4, 4]\n");
    assert!(cut < 0.01, "{cut}");
}

fn lna() -> Project {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    Project::load(&root).unwrap()
}

fn find(p: &Project, name: &str) -> agentee_core::project::ItemRef {
    p.find(name).unwrap()
}

fn lit(px: &[u8]) -> usize {
    px.chunks(4).filter(|c| c[..3].iter().map(|v| *v as u32).sum::<u32>() > 300).count()
}

#[test]
fn a_canvas_render_crops_to_the_drawing() {
    let p = lna();
    let opts = RenderOptions { width: 1400, height: 1400, panels: false, ..Default::default() };
    let (w, h, _) = decode(&render_png(&p, find(&p, "pcb:lna"), &opts).unwrap());
    assert!(w <= 1400 && h <= 1400 && (w < 1400 || h < 1400), "{w}x{h}");
    let opts = RenderOptions { width: 1400, height: 1400, ..Default::default() };
    let (w, h, _) = decode(&render_png(&p, find(&p, "pcb:lna"), &opts).unwrap());
    assert_eq!((w, h), (1400, 1400));
}

#[test]
fn focus_hides_or_dims_what_it_does_not_name() {
    let p = lna();
    for item in ["sch:lna", "pcb:lna"] {
        let r = find(&p, item);
        let render = |context| {
            let opts = RenderOptions {
                width: 900,
                height: 900,
                panels: false,
                focus: vec!["U1".into()],
                context,
                ..Default::default()
            };
            let (w, h, px) = decode(&render_png(&p, r, &opts).unwrap());
            (w, h, lit(&px))
        };
        let (w, h, shown) = render(agentee_view::Context::Show);
        let (_, _, dimmed) = render(agentee_view::Context::Dim);
        let (_, _, hidden) = render(agentee_view::Context::Hide);
        assert!(w <= 900 && h <= 900);
        assert!(hidden > 0, "{item}");
        assert!(dimmed < shown && hidden <= dimmed, "{item} {shown} {dimmed} {hidden}");
    }
}

#[test]
fn focus_takes_nets_pins_and_globs_and_rejects_unknown_names() {
    let p = lna();
    for item in ["sch:lna", "pcb:lna"] {
        let r = find(&p, item);
        for names in [&["RF_*"][..], &["U1.1"], &["VCC", "C1"]] {
            let opts = RenderOptions {
                panels: false,
                focus: names.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            assert!(render_png(&p, r, &opts).is_ok(), "{item} {names:?}");
        }
        let opts =
            RenderOptions { panels: false, focus: vec!["NOPE".into()], ..Default::default() };
        let err = render_png(&p, r, &opts).unwrap_err();
        assert!(err.contains("NOPE"), "{err}");
    }
    let opts = RenderOptions { focus: vec!["U1".into()], ..Default::default() };
    assert!(render_png(&p, find(&p, "board:lna"), &opts).is_err());
}

#[test]
fn rulers_label_the_edges() {
    let p = lna();
    let r = find(&p, "pcb:lna");
    let render = |rulers| {
        let opts = RenderOptions { panels: false, rulers, ..Default::default() };
        decode(&render_png(&p, r, &opts).unwrap()).2
    };
    let (plain, ruled) = (render(false), render(true));
    let top = |px: &[u8]| px[..px.len() / 30].to_vec();
    assert_ne!(top(&plain), top(&ruled));
}

#[test]
fn a_canvas_sim_render_stops_at_its_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
    let p = Project::load(&root).unwrap();
    let opts = RenderOptions { width: 1400, height: 1400, panels: false, ..Default::default() };
    let (w, h, _) = decode(&render_png(&p, p.find("sim:counter").unwrap(), &opts).unwrap());
    assert_eq!(w, 1400);
    assert!(h < 700, "{h}");
}

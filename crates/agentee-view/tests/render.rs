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
        let (w, h, px) = decode(&render_png(&project, item, &opts));
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
    let (_, _, px) = decode(&render_png(&project, item, &opts));
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

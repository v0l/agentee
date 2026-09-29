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

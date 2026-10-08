use agentee_core::project::Project;
use std::path::Path;

#[test]
fn the_engine_places_bottom_parts_on_the_bottom() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = Project::load(&lna).unwrap();
    let i = p.layouts.iter().position(|l| l.name == "lna").unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let text = std::fs::read_to_string(&p.layouts[i].path).unwrap();
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
    let parked = [("C3", [4.0, 4.0]), ("C5", [6.0, 4.0])];
    for t in doc["footprints"].as_array_of_tables_mut().unwrap().iter_mut() {
        let r = t["ref"].as_str().unwrap().to_string();
        if let Some((_, at)) = parked.iter().find(|(p, _)| *p == r) {
            let mut a = toml_edit::Array::new();
            a.push(at[0]);
            a.push(at[1]);
            t["at"] = toml_edit::value(a);
            t["side"] = toml_edit::value("bottom");
        }
    }
    let run = agentee_layout::Run {
        from: Some("place".into()),
        to: Some("place".into()),
        only: None,
        watch: None,
        stop: None,
    };
    let out = agentee_layout::run_text(&inputs, &doc.to_string(), &run).unwrap();
    let layout = inputs.resolve(&out.text).unwrap().item;
    let edge = layout.edge();
    for (r, at) in parked {
        let q = layout.parts.iter().find(|q| q.reference == r).unwrap();
        assert!(q.bottom, "{r} left the bottom");
        assert!(agentee_core::geom::dist(q.at.to_mm(), at) > 0.1, "{r} stayed at {at:?}");
        assert!(edge.contains(q.at.to_mm()), "{r} at {:?} is off the board", q.at.to_mm());
    }
}

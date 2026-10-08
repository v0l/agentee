use agentee_core::project::Project;
use std::path::Path;

#[test]
fn a_chain_part_on_a_locked_part_moves_off_it() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == "lna").unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let text = std::fs::read_to_string(&p.layouts[i].path).unwrap().replacen(
        "ref = \"L1\"\nat = [14.775, 16.415]\nrotation = 90",
        "ref = \"L1\"\nat = [13.0, 12.5]\nrotation = 90\nlocked = true",
        1,
    );
    assert!(text.contains("locked = true"));
    let run = agentee_layout::Run {
        from: None,
        to: Some("place".into()),
        only: None,
        watch: None,
        stop: None,
    };
    let r = agentee_layout::run_text(&inputs, &text, &run).unwrap();
    let overlap = r.score.terms["overlap"].raw;
    assert_eq!(overlap, 0.0, "{:?}", r.score.terms["overlap"].worst);
    let l = inputs.resolve(&r.text).unwrap().item;
    let at = |r: &str| l.parts.iter().find(|p| p.reference == r).unwrap().at;
    assert_eq!(at("L1").to_mm(), [13.0, 12.5], "the locked part stays");
}

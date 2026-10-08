use agentee_core::project::Project;
use agentee_layout::Run;
use agentee_layout::search::{Ask, search};
use std::path::Path;

#[test]
fn the_beam_branches_route_knobs_from_each_kept_placement() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == "lna").unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let text = std::fs::read_to_string(&p.layouts[i].path).unwrap()
        + "\n[engine.search]\nknobs = { \"place.seed\" = [1, 2, 3, 4], \"detail.via_cost\" = [\"1mm\", \"3mm\"] }\n";
    let run = Run { from: None, to: Some("detail".into()), only: None, watch: None, stop: None };
    let r = search(&inputs, &text, &run, &Ask { tries: Some(4), keep: Some(2) }).unwrap();
    let placed = r.tried.iter().filter(|t| t.stage == "place").count();
    assert_eq!(placed, 4, "{}", r.table());
    let routed: Vec<_> = r.tried.iter().filter(|t| t.stage == "detail").collect();
    let mut parents: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
    for t in &routed {
        let p = t.parent.unwrap();
        let parent = &r.tried[p];
        assert_eq!(parent.stage, "global");
        assert!(parent.kept);
        assert_eq!(parent.knobs["place.seed"], t.knobs["place.seed"]);
        parents.entry(p).or_default().push(t.knobs["detail.via_cost"].clone());
    }
    assert!((2..=3).contains(&parents.len()), "{}", r.table());
    for costs in parents.values() {
        let mut c = costs.clone();
        c.sort();
        assert_eq!(c, ["1mm", "3mm"], "{}", r.table());
    }
    assert!(r.run.text.contains("via_cost"), "the winner's via cost is pinned");
}

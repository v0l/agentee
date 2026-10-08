use agentee_core::project::Project;
use agentee_layout::fit::{Ask, Strategy, fit};
use std::path::Path;

fn lna() -> (agentee_core::project::LayoutInputs, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == "lna").unwrap();
    (p.layout_inputs(i).unwrap(), std::fs::read_to_string(&p.layouts[i].path).unwrap())
}

#[test]
fn the_lna_fits_a_smaller_board_and_the_router_agrees() {
    let (inputs, text) = lna();
    let r = fit(&inputs, &text, &Ask { verify: 2, ..Default::default() }).unwrap();
    let best = r.best_trial().expect("a size that fits");
    assert!(best.width * best.height < 36.0 * 24.0, "{}", r.table());
    assert!(best.routed.as_ref().is_some_and(|x| x.ok), "{}", r.table());
    assert!(!best.moves.is_empty());
}

#[test]
fn spread_needs_more_board_than_tight() {
    let (inputs, text) = lna();
    let area = |strategy| {
        let ask = Ask { strategy, aspects: vec![1.0], ..Default::default() };
        let r = fit(&inputs, &text, &ask).unwrap();
        r.best.map(|[w, h]| w * h).unwrap()
    };
    assert!(area(Strategy::Tight) < area(Strategy::Spread));
}

#[test]
fn a_held_width_stays() {
    let (inputs, text) = lna();
    let r = fit(&inputs, &text, &Ask { width: Some(30.0), ..Default::default() }).unwrap();
    let [w, h] = r.best.unwrap();
    assert_eq!(w, 30.0);
    assert!(h < 24.0, "{}", r.table());
}

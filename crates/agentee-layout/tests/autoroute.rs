use agentee_core::layout::Layout;
use agentee_core::project::{LayoutInputs, Project};
use agentee_core::route::RouteOptions;
use agentee_layout::start::{Reset, reset};
use std::path::Path;

fn stripped(example: &str, extra: &str) -> (LayoutInputs, Layout) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(example);
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == example).unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let text = std::fs::read_to_string(&p.layouts[i].path).unwrap() + extra;
    let text = reset(&text, &Reset { routing: true, ..Default::default() }).unwrap();
    let layout = inputs.resolve(&text).unwrap().item;
    (inputs, layout)
}

fn all() -> RouteOptions {
    RouteOptions { nets: vec!["*".into()], ..Default::default() }
}

#[test]
fn routes_every_lna_connection_within_the_rules() {
    let (inputs, layout) = stripped("lna", "");
    let r = agentee_layout::autoroute::route(&layout, &inputs.board, &all()).unwrap();
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    assert!(r.failed.is_empty());
    assert_eq!(r.per_net.values().sum::<usize>(), r.routed);
}

#[test]
fn a_glob_leaves_plane_nets_to_their_pour() {
    let (inputs, layout) = stripped("lna", "");
    let r = agentee_layout::autoroute::route(&layout, &inputs.board, &all()).unwrap();
    let planes = agentee_core::tie::plane_nets(&layout);
    assert!(!planes.is_empty());
    for n in planes {
        let name = &layout.nets[n].name;
        assert!(r.tracks.iter().all(|t| &t.net != name), "{name} was routed");
    }
}

#[test]
fn layers_hold_tracks_to_the_named_copper() {
    let (inputs, layout) = stripped("lna", "");
    let opts = RouteOptions { layers: vec!["F.Cu".into()], ..all() };
    let r = agentee_layout::autoroute::route(&layout, &inputs.board, &opts).unwrap();
    assert!(r.tracks.iter().any(|t| t.layer == "F.Cu"));
    for t in r.tracks.iter().filter(|t| t.layer != "F.Cu") {
        let len: f64 = t.points.windows(2).map(|w| agentee_core::geom::dist(w[0], w[1])).sum();
        let ends = [t.points[0], *t.points.last().unwrap()];
        let at_via = r.vias.iter().any(|v| {
            v.net == t.net && ends.iter().any(|e| agentee_core::geom::dist(*e, v.at) < 1e-6)
        });
        assert!(len < 2.0 && at_via, "{} runs {len:.2} mm on {}", t.net, t.layer);
    }
}

#[test]
fn an_unknown_layer_or_via_is_an_error() {
    let (inputs, layout) = stripped("lna", "");
    let bad = RouteOptions { layers: vec!["X.Cu".into()], ..all() };
    assert!(agentee_layout::autoroute::route(&layout, &inputs.board, &bad).is_err());
    let bad = RouteOptions { via: vec!["nope".into()], ..all() };
    assert!(agentee_layout::autoroute::route(&layout, &inputs.board, &bad).is_err());
}

#[test]
fn via_budgeted_lanes_keep_their_budget() {
    let extra = "\n[[interfaces]]\nname = \"rf\"\nnets = [\"RF_*\"]\nmax_vias = 0\n";
    let (inputs, layout) = stripped("lna", extra);
    let r = agentee_layout::autoroute::route(&layout, &inputs.board, &all()).unwrap();
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    assert!(r.vias.iter().all(|v| !v.net.starts_with("RF_")));
}

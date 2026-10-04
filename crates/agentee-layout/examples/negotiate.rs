use agentee_core::project::Project;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(&args[0]);
    let name = &args[1];
    let strip_tracks = args.iter().any(|a| a == "--strip-tracks");
    let via_in_pad = args.iter().any(|a| a == "--via-in-pad");
    let rounds = args
        .iter()
        .position(|a| a == "--rounds")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| &l.item.name == name).expect("layout");
    let inputs = p.layout_inputs(i).unwrap();
    let mut text = std::fs::read_to_string(&p.layouts[i].path).unwrap();
    for phase in
        std::iter::once(agentee_layout::ROUTE).chain(agentee_layout::RETIRED_PLANS.iter().copied())
    {
        text = agentee_layout::strip_plan(&text, phase);
    }
    if strip_tracks {
        let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
        doc.remove("tracks");
        text = doc.to_string();
    }
    let layout = inputs.resolve(&text).unwrap().item;
    for part in layout.parts.iter().filter(|p| agentee_layout::escape::is_bga(p)) {
        eprintln!("bga {} pitch {}", part.reference, agentee_layout::escape::pitch_of(part));
    }
    if let Ok(n) = std::env::var("AGENTEE_STUB_NET") {
        eprintln!("net {} = {:?}", n, layout.nets.iter().position(|x| x.name == n));
    }
    let opts = agentee_layout::negotiate::Options {
        via_in_pad,
        rounds,
        verbose: true,
        ..Default::default()
    };
    let t = std::time::Instant::now();
    let r = agentee_layout::negotiate::route(&layout, &inputs.board, &opts).unwrap();
    eprintln!(
        "{} of {} connections, {} tracks, {} vias, {:.1}s",
        r.routed,
        r.connections,
        r.tracks.len(),
        r.vias.len(),
        t.elapsed().as_secs_f64()
    );
    for f in &r.failed {
        eprintln!("  {}: {}", f.net, f.reason);
    }
    let body = agentee_layout::routed_toml(&r);
    let text = agentee_layout::write_plan(&text, agentee_layout::ROUTE, &body);
    std::fs::write(&p.layouts[i].path, text).unwrap();
}

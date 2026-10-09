use crate::negotiate;
use agentee_core::board::Board;
use agentee_core::layout::{Layout, Track, Via, glob};
use agentee_core::route::{self, RouteOptions, RouteResult};

pub fn needs_grid_router(opts: &RouteOptions) -> bool {
    opts.pairs
        || opts.connection.is_some()
        || !opts.corridors.is_empty()
        || !opts.fences.is_empty()
        || !opts.tiers.is_empty()
        || !opts.class_order.is_empty()
}

fn targets(layout: &Layout, nets: &[String]) -> Vec<String> {
    let planes = agentee_core::tie::plane_nets(layout);
    (0..layout.nets.len())
        .filter(|&n| {
            let name = &layout.nets[n].name;
            nets.iter().any(|g| g == name)
                || (!planes.contains(&n) && nets.iter().any(|g| glob(g, name)))
        })
        .map(|n| layout.nets[n].name.clone())
        .collect()
}

fn via_budgeted(layout: &Layout) -> Vec<&str> {
    layout
        .interfaces
        .iter()
        .filter(|i| i.spec.max_vias.is_some())
        .flat_map(|i| i.lanes.iter().flat_map(|l| l.nets.iter().map(String::as_str)))
        .collect()
}

fn with_copper(layout: &Layout, board: &Board, r: &RouteResult) -> Layout {
    let mut out = layout.clone();
    let net_of = |name: &str| layout.nets.iter().position(|n| n.name == name);
    for t in &r.tracks {
        let Some(net) = net_of(&t.net) else { continue };
        out.tracks.push(Track {
            source: usize::MAX,
            net,
            layer: t.layer.clone(),
            width: t.width.unwrap_or(layout.nets[net].width),
            points: t.points.clone(),
        });
    }
    for v in &r.vias {
        let (Some(net), Some(spec)) = (net_of(&v.net), board.vias.iter().find(|s| s.name == v.via))
        else {
            continue;
        };
        out.vias.push(Via::of(spec, net, v.at, &layout.copper));
    }
    out
}

fn negotiated(
    layout: &Layout,
    board: &Board,
    opts: &RouteOptions,
    nets: Vec<String>,
) -> Result<RouteResult, String> {
    let n = negotiate::Options {
        nets,
        layers: opts.layers.clone(),
        via: opts.via.clone(),
        grid: opts.grid,
        via_cost: opts.via_cost,
        bend_cost: opts.bend_cost,
        via_in_pad: opts.via_in_pad,
        ..Default::default()
    };
    let r = negotiate::route(layout, board, &n)?;
    Ok(route::drop_spare_vias(layout, board, route::hold_to_rules(layout, board, r)))
}

pub fn route(layout: &Layout, board: &Board, opts: &RouteOptions) -> Result<RouteResult, String> {
    if needs_grid_router(opts) {
        return route::route(layout, board, opts);
    }
    let wanted = targets(layout, &opts.nets);
    if wanted.is_empty() {
        return Err("no net matches".into());
    }
    let budgeted = via_budgeted(layout);
    let (held, free): (Vec<String>, Vec<String>) =
        wanted.into_iter().partition(|n| budgeted.contains(&n.as_str()));
    if held.is_empty() {
        return negotiated(layout, board, opts, free);
    }
    let first = if free.is_empty() {
        RouteResult::default()
    } else {
        negotiated(layout, board, opts, free)?
    };
    let after = with_copper(layout, board, &first);
    let budget = RouteOptions {
        nets: held,
        layers: opts.layers.clone(),
        grid: opts.grid,
        via: opts.via.clone(),
        via_cost: opts.via_cost,
        bend_cost: opts.bend_cost,
        via_in_pad: opts.via_in_pad,
        ..Default::default()
    };
    let rest = route::route(&after, board, &budget)?;
    let mut out = first;
    out.connections += rest.connections;
    out.routed += rest.routed;
    out.tracks.extend(rest.tracks);
    out.vias.extend(rest.vias);
    out.failed.extend(rest.failed);
    for (k, v) in rest.per_net {
        *out.per_net.entry(k).or_default() += v;
    }
    Ok(out)
}

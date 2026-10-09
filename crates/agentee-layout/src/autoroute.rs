use crate::negotiate;
use agentee_core::board::Board;
use agentee_core::layout::{Layout, glob};
use agentee_core::route::{self, RouteOptions, RouteResult};

pub fn needs_grid_router(opts: &RouteOptions) -> bool {
    opts.connection.is_some()
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
    negotiated(layout, board, opts, wanted)
}

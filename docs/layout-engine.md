# Layout engine

The placer and router agentee ships today are one-shot tools: the placer is a force-directed
pass with annealing, the router a grid A* per connection with local rip-up. Neither has a view
of the whole board, so they pack parts into corridors they then cannot route, escape BGAs by
luck, and wander around congestion they created themselves. This is the design for their
replacement.

The engine is a pipeline of phases. Every phase has one job, reads the same board model, and
writes its result into the model for the phases after it. Each phase first solves its own
problem as if nothing else were on the board, then repeats with a growing penalty for whatever
it collides with, until nothing overflows. That is negotiated congestion, the method that
FPGA routers (PathFinder), chip global routers (FastRoute, NCTU-GR) and analytical placers
(ePlace, RePlAce) all converge on, and it is what "everything picks its own optimum, then we
nudge each phase with obstacles" comes down to when written out.

The phases run in a fixed order because each needs what the one before produced. A
configuration can turn phases off, change their weights, or stop after any of them, and a
phase can be rerun on its own with the rest of the model held fixed.

## Board model

One in-memory model, `Layout` plus a `Plan`, carries every phase's result:

| item | written by | read by |
|---|---|---|
| regions: a polygon per subsystem with its parts and role | floorplan | placement, planes, routing |
| placements: position, rotation, side per part | placement, legalise | everything after |
| via sites: the vias each BGA ball and each SMD pad will use, with layer span | escape | placement (decaps), routing |
| planes: polygon per rail per layer, with the stubs that reach them | planes | routing |
| corridors: the tile path per net per layer from global routing | global route | track assign, detailed route |
| tracks and vias | detailed route | DRC, tune, neck |

Everything is checkpointed to the layout file after each phase, as the ordinary `[[footprints]]`,
`[[vias]]`, `[[zones]]`, `[[tracks]]` tables plus one `[[plan.*]]` table per phase for the
intermediate results (regions, via sites, corridors). A rerun from phase N loads the file and
throws away the plan of N and after.

## The shared cost field

Every phase that moves things uses the same board-wide cost field: a tile grid (default 0.5 mm)
per layer, where a tile carries

- **capacity**: how many tracks of the class width can cross it, from the free width after
  pads, vias, keepouts and plane cuts
- **demand**: what the phase has put through it so far
- **history**: how often it has overflowed in earlier rounds, never decreasing

Cost of using a tile is `base * (1 + demand/capacity) + history`. Placement reads the field to
know where routing would be dense (RUDY estimate from the ratsnest), escape and global routing
write demand into it, and detailed routing follows the corridors it produced. The history term
is what makes the negotiation converge: a tile that keeps overflowing becomes expensive enough
that something moves elsewhere.

## Phases

### 1. Floorplan

Assigns each part to a subsystem region and pins what has to be pinned. Input is the schematic
and the board outline; output is the regions and a first placement of anchors.

- **Subsystems** come from the schematic sheets by default (`power`, `rf`, `usb`...), or from
  `[place] regions` in the layout file. A part belongs to the sheet it is drawn on.
- **Connectors** go to edges. `[place] edges` pins them; otherwise each connector's edge is
  chosen so RF and USB connectors do not share an edge, then spread along it.
- **Chains** are ordered runs of two-pin nets from a connector to an IC pin: an RF path
  (SMA, switch, DC block, LNA, balun, transceiver), a power entry (USB-C, TVS, eFuse, bucks), a
  reference input. They are found by walking two-pin nets from each connector. A chain is
  laid out as a line from its connector, in net order, and its parts are locked relative to
  each other for the rest of the flow.
- **Large chips** (BGAs, anything over 25 mm2 with 16+ pins) get a region each, sized by their
  courtyard plus the room their escape needs: once the escape phase has run, that is the actual
  escape tracks and via sites, which later phases treat as copper; before it has, a ring of two
  ball pitches stands in. Their rotation is chosen here by pin access: for each of the four
  rotations, the sum over pin groups (a bank, an interface, a chain end) of the distance to
  where that group's other end sits.
- **Zone rules**: switchers keep a distance from RF and clock regions, hot parts from each
  other, MLCCs out of the flex zone. These are constraints the later phases keep.

Regions are placed by a small force-directed pass over region centroids, weighted by the net
count between regions, then packed into the outline with the connectors on their edges.

### 2. Global placement

Places every part that floorplan did not pin, inside its region, by analytical placement:
minimise wirelength plus a density penalty, solved as ePlace does (electrostatic analogy,
Nesterov descent), with the routing congestion estimate folded into the density so parts
spread where the ratsnest is dense. Wirelength is weighted HPWL: pairs, impedance and
interface nets count three times, plane nets not at all. Decoupling capacitors are held near
the supply pin they serve (a spring, not a fixed spot), crystals near their pins.

Runs until the density overflow is under a threshold, then hands over.

### 3. Legalise and refine

Snaps the placement to legal spots: courtyards clear (Tetris/Abacus row packing for passives,
ring search for the rest), edge and keepout rules kept, sides assigned (`--side both` lets
decaps under a BGA on the back, but only at spots clear of the escape via sites, which phase 4
has already computed for BGAs with a fixed rotation). Then a short detailed placement pass:
swaps of like parts and single moves that reduce HPWL and crossings, scored with the zone rules
as penalties. This is the current annealing refinement, kept.

### 4. Escape

For every BGA, decides how each ball leaves the package. Plane balls get a via straight to
their plane. Signal balls are escaped a layer at a time, outside in: the top layer first, then
each signal layer in the board order. On each layer the ball grid becomes a flow network on a
half-pitch grid (ball cells, the gaps between balls, the channel crossings), with the pads (top
layer) or the vias already placed (inner layers) as blocked cells, one track per gap cell, and
the package edge as the sink. Each ball wants the side its net's other pins are on (the
centroid of them, relative to the package), so the layer is run one side at a time: the balls
that want that side, a min-cost flow in which a step costs 1, that side's edge costs 0, the
neighbouring sides cost a package width and the far side four, and the escapes it finds are
fixed before the next side runs; a last pass takes whatever is left to any edge. Each path is
its track, and no two cross. A ball that escapes on an inner
layer gets a via in its pad. Inside the package tracks run at the fab minimum width and
clearance (the class width would not fit between 0.75 mm balls), and the detailed router widens
them past the edge. Balls that reach no edge on any layer are reported.

Escape also decides the via site for every fine-pitch SMD pad that needs one (QFN, DFN, 0201),
so decaps and routing know where the holes are before either commits.

Output: via sites and escape tracks in the plan, written into the layout file between
`# plan escape` and `# end plan escape` as ordinary vias and tracks; a rerun replaces them.

### 5. Planes

Cuts the power layers into rail regions. Each plane layer starts as its base rail (the one
with the most sinks). Every other rail gets the Voronoi region of its sink vias on that layer,
grown to reach its source, as long as the region stays connected and keeps the base rail
connected; where it cannot, the rail is left to routing as wide tracks. Ground planes are never
cut for signals. Output: the zone outlines and, for each plane pad, the stub or via it uses.

### 6. Global route

Routes every net at once on the tile grid, all layers, with negotiated congestion
(PathFinder): each net takes its cheapest tree (a rectilinear Steiner tree for multi-pin nets,
pattern routes for two-pin nets, then maze routing on the tile graph), the cost field is
updated, and the rounds repeat with rising history cost until no tile is over capacity. Inner
signal layers have a preferred direction (alternating H and V) with a cost for going against
it; vias between tiles cost a fixed track length. Pairs are routed as one net of double width;
nets in an interface with a via budget carry it as a constraint. The RF class is confined to
the layers its board class allows.

Output: a corridor per net, the ordered list of tiles per layer.

### 7. Track assign

Inside each corridor, assigns each net a track position across the corridor width, so
parallel nets in a channel do not cross, and fixes the layer of each segment. Pairs get two
adjacent positions at the pair gap. This is the step that turns "which tiles" into
"which lane in the tile" and makes detailed routing a local problem.

### 8. Detailed route

The current grid A* with rip-up, kept, but confined to each net's corridor grown by one tile,
with the escape tracks and via sites fixed. Necks into small pads, doglegs, chamfers and the
via row alignment stay as they are. A connection that fails inside its corridor is returned
to global routing with its tiles' history raised, and the two phases iterate (bounded, default
three rounds).

### 9. Finish

The existing `tune` (pair skew, match groups), `neck`, `silk` and `fill`, run in that order.

## The score

One score judges every phase and drives the search in each. It is a weighted sum of terms;
every term is a rule an engineer would apply by hand, and the weights are in the layout file
so they can be read and changed. Lower is better. A term with weight 0 is off.

| term | measures | default weight | the rule |
|---|---|---|---|
| `wirelength` | HPWL per net, mm, weighted by class (`pair` 3, `interface` 3, `plane` 0) | 1 | short connections route and behave |
| `overflow` | tiles over capacity, summed excess in track widths | 50 | it has to route |
| `unrouted` | connections with no corridor or no track | 200 | it has to route |
| `vias` | vias per net beyond the class allowance | 2 | each via is a discontinuity and a drill |
| `crossings` | pairs of two-pin ratsnest lines that cross | 1 | crossings become vias or detours |
| `chain_order` | mm a chain part sits off the line from its connector, in net order | 20 | RF and power entry go in a line from the connector |
| `chain_layer` | mm of an RF net not on its class layers | 100 | RF stays on one layer over ground |
| `return_path` | mm of an impedance net over a plane cut or edge | 20 | every fast signal needs its return under it |
| `pair_coupling` | mm of a pair run off its gap, and skew over `max_skew` in ps | 10 | pairs stay coupled and matched |
| `pin_access` | per BGA, distance from each pin group to its partner region, for the chosen rotation | 5 | the BGA faces what it talks to |
| `decap` | distance of each decoupling capacitor from its pin beyond `decoupling_distance` | 5 | decaps at the pin, MT-101 |
| `via_site` | a bottom-side part over an escape via site | 100 | nothing sits on a fanout |
| `switcher` | mm a switcher sits inside `hot_distance` of an RF or clock region | 20 | switching noise away from RF and clocks |
| `hot` | mm two hot parts sit inside `hot_distance` | 5 | heat does not stack |
| `flex` | an MLCC of 0805 or larger in the flex zone | 10 | boards bend at edges and holes |
| `edge` | a body inside `min_body_to_edge`, a connector off its edge | 50 | handling and depaneling |
| `keepout` | courtyard area inside a keepout or an escape ring | 100 | escape channels stay open |
| `overlap` | courtyard overlap area, mm2 | 100 | parts do not stack |
| `plane_reach` | a plane pad not reached by its rail's plane | 20 | planes reach their sinks |
| `area` | board area, mm2, and copper layers, per the outer loop | 0.01 and 100 | smaller and thinner is cheaper |

Placement phases evaluate the terms they can (everything except the routing ones, with
`overflow` from the RUDY estimate). Routing phases evaluate all of them. `agentee layout` prints
the score per term after each phase, so a bad result says which rule it broke.

## Outline and stackup search

Layer count and board size are not inputs; they fall out of an outer loop:

1. Start from the smallest outline that packs the courtyards at the target density, and the
   fewest copper layers the classes allow (a board with an RF or impedance class needs a ground
   under the signal layer, so four).
2. Run the pipeline. If `overflow` or `unrouted` is not zero, grow the outline by 10% on its
   longer side, or add a layer pair (signal plus plane) when the outline has grown twice without
   routing, and run again.
3. Stop at the first run with zero overflow and unrouted, then try one step smaller on each
   axis and keep whichever scores lower with `area` included.

`[engine] outline = "search"` and `layers = "search"` turn this on; a fixed outline or stackup
in the board file pins either one. The loop reuses the previous placement as its start, so a
step costs one routing pass, not a fresh flow.

## Configuration

```toml
[engine]                       # in the layout file, all optional
phases = ["floorplan", "place", "legalise", "escape", "planes", "global", "assign", "detail", "finish"]
tile = "0.5mm"
rounds = 3                     # global/detail iterations
outline = "search"             # or fixed by the board file
layers = "search"

[engine.score]                 # weights, the table above; any subset
overflow = 50
chain_order = 20

[engine.floorplan]
regions = { rf = ["U2", "T1", "T2", "U14", "U15", "U16"], usb = ["U3", "J1", "U5"] }
edges = { J1 = "bottom", J2 = "left" }
escape_ring = 2                # ball pitches kept clear around a BGA

[engine.place]
weights = { pair = 3.0, interface = 3.0, plane = 0.0 }
density = 0.7                  # target tile utilisation

[engine.escape]
layers = ["F.Cu", "In2.Cu", "B.Cu"]   # ring order outside in

[engine.global]
direction = { "In2.Cu" = "H", "In3.Cu" = "V" }
via_cost = "1mm"
```

`agentee layout NAME` runs the pipeline; `--from escape` reruns from a phase with the earlier
results held; `--to global` stops after a phase; `--only planes` reruns one. Each phase reports
what it did and what it could not (parts that would not fit, tiles still over capacity, nets
with no corridor) in the same JSON shape the current commands use, and `agentee view` draws the
plan: regions, corridors as translucent bands, tiles over capacity in red.

## What is kept from today

The DRC, the fill, `tune`, `neck`, `silk`, the interfaces and pair rules, the net classes, the
grid A* (as the detailed router), the annealing refinement (as detailed placement), and the
KiCad-style fanout as a fallback when a BGA has no escape plan. The current `place` and `route`
commands stay until the pipeline matches them on the examples, then become aliases for
`layout --only`.

## Crate

`agentee-layout`, depending on `agentee-core` for the model and DRC. One module per phase with
one entry point each:

```rust
pub trait Phase {
    fn name(&self) -> &'static str;
    fn run(&self, model: &mut Model, cfg: &Config, field: &mut CostField) -> PhaseReport;
}
```

The driver runs the configured list, checkpoints after each, and handles the global/detail
iteration. Phases do not call each other.

## State

`agentee layout NAME` runs the score and the phases that exist: floorplan, place, legalise,
escape, planes, global and detail. Assign and finish are listed as skipped. After each phase the driver writes its plan into the
layout file between `# plan <phase>` markers and resolves the layout again, so the next phase
sees the zones filled around the new copper. `--only X` and `--from X` drop the plans of X and
every later phase first.

Floorplan finds the chains from each connector and lays them in a line inward from it (on
`examples/sdr` the RX and TX RF paths, the reference input and the PPS input). Place runs the
existing placer (its global placement, legalisation and annealing) on every part that is not
locked or chained, so the chains hold. Regions and the analytical placer described above are not
built: an ePlace style pass gave 10 to 11 m of wirelength against the existing placer's 6.7 m.
Legalise moves only parts that still overlap.

Escape uses dog-bone vias unless `[engine.escape] via_in_pad = true`, and falls back to via in
pad where the pitch leaves no room for one (U2 at 0.8 mm, U3 at 0.75 mm). Plane balls take the
diagonal crossing least crowded by signal balls and share a via with a neighbour on the same net
where they can. On `examples/sdr` 22 inner balls of U1 find no escape.

Planes grows each rail from its vias on the plane layer (1.5 mm reach), joins pieces with 0.8 mm
necks up to 6 mm long, and reopens the base rail where a rail cut its vias off. Pieces it cannot
join are left to routing. On `examples/sdr` In3 gets 39 rail regions and the 3V3 base stays whole.

Global routes every net except ground, power nets only between the islands the planes leave.
Detail routes with the core grid router; global's corridors are a cost (off-corridor cells cost
4x) and bound each search's box. `[engine.detail] corridors = false` turns them off. On
`examples/sdr` floorplan to detail takes about 60 s and routes 272 of 361 connections.

## Order of work

1. Cost field, tile grid, the plan tables and the score; `layout` command that runs the
   configured phases and prints the score per term; viewer drawing of the plan.
2. Escape (phase 4). Measured on `examples/sdr`: every ball of U1, U2, U3 gets a legal escape.
3. Global route and track assign (6, 7), detailed route confined to corridors (8). Measured by
   the SDR's failed connections (86 today with the current placement) and total via count.
4. Planes (5).
5. Floorplan (1), then global placement (2) and legalise (3). Measured by the SDR routing again,
   and by the RF chain landing in order from its SMA.
6. The outline and stackup loop.
7. Retire the old `place` and `route`.

## Sources

- PathFinder: McMurchie, Ebeling, "PathFinder: a negotiation-based performance-driven router
  for FPGAs", FPGA 1995.
- Global routing: Pan, Chu, "FastRoute 2.0", ASP-DAC 2007; Liu et al, "NCTU-GR 2.0", TCAD 2013.
- Detailed routing: Kahng et al, "TritonRoute", TCAD 2020.
- Analytical placement: Lu et al, "ePlace", TODAES 2015; Cheng et al, "RePlAce", TCAD 2018.
- Legalisation: Spindler, Schlichtmann, Johannes, "Abacus", ISPD 2008.
- Escape routing: Yan, Wong, "BGA escape routing", ICCAD 2008; Xilinx UG1099.
- Congestion estimate: Spindler, Johannes, "RUDY", DATE 2007.

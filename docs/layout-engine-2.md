# Layout engine 2

This replaces the pipeline in [layout-engine.md](layout-engine.md). The phases there were built
by wrapping the existing placer and router and committing each phase's copper to the file before
the next phase ran. The idea of negotiated congestion was right, but the router never gets to
negotiate: by the time it runs, most of what it collides with is already fixed. This design keeps
the score, the DRC, the A* search and the finishing tools, and changes who owns copper, how
capacity is measured and how the stages talk to each other.

## Why the current engine fails

Measured on copies of the examples with the release build at `8917371`.

| board | run | result |
|---|---|---|
| `sdr` | floorplan to finish | 136 s, 283 of 311 detail connections, 98 unrouted in the score, global overflow 270 after 14 rounds, 289 DRC errors |
| `sdr` | escape to finish, committed placement | 101 s, 280 of 311, 97 unrouted, overflow 268 |
| `lna` | floorplan to finish | 1 s, RF_IN unrouted, global overflow 55, RF_AMP_OUT 11.7 mm (hand layout 4.2 mm), VCC 70 mm (hand 24 mm) |
| `lna` | `agentee route` on the hand placement, tracks stripped | 0.3 s, 17 of 17 |

The router is fine on a good placement of a small board. Everything else is where it breaks:

**Phases commit copper the router cannot move.** `tie` puts a via beside every plane pad and
`escape` fans out every plane ball before any signal is routed. Those vias are fixed obstacles
to detail, so RX_ANT, RX_RF, VCCADC, VDD_GPO, PLL_LE and PLL_DATA find "no path within the
rules" through a field of ground vias that could have gone anywhere. Signal escape exists but is
off by default, for the same reason: committed escape tracks hurt more than they help.

**Power is planned and then dropped.** `planes` splits In3 into 33 to 35 rail regions and leaves
13 pieces "to routing". Detail leaves plane nets out of `*`, so those pieces are never attempted.
Most of the 98 unrouted connections are 1V8 (11), 3V3 (11), 1V0 (9), VSYS (6) and 1V3A (6).

**Global and detail disagree.** A global tile is 0.25 mm with a fixed capacity of `tile / 0.2`
tracks, pads subtract their area from it, and demand is counted per tile rather than per tile
boundary. Terminals therefore sit in tiles with zero capacity and every net overflows where it
starts: 55 on a 21 part board. `sdr` runs with `corridors = false` because the corridors made
detail worse, so global's 13 to 21 s produces nothing that is used.

**Detail is sequential with walls between classes.** Classes route in a fixed order, each on a
grid rebuilt for its width, and every earlier class is a hard obstacle to every later one. Rip-up
only happens inside a class. `sdr` sets `class_order` by hand to get its result. The rebuild per
class (about 12 classes, 10 M cells each at 0.05 mm on 6 layers) is most of detail's 69 to 87 s.

**A class's layer list means two things.** LVDS allows only `In2.Cu`. Its pads are on F.Cu and
escape leaves signal balls to routing, so detail finds no copper of the net on a layer it may use:
all 12 LVDS nets fail with "an end has no copper on the routing layers". A layer list should
limit where the tracks run, never whether the via out of the pad may exist.

**Two placers, neither sees routing.** Floorplan finds chains from connectors named `J` and skips
RF; the core placer has its own RF chain code. On `lna` floorplan finds no chain and the result
loses the hand layout. On `sdr` 45 decaps sit 167 mm in total past their 3 mm budget, U1 and U3
end 0.75 mm apart, 29 mm2 of courtyard lands in the BGA escape rings and the ratsnest has 937
weighted crossings. Placement is scored on HPWL and crossings and never hears that a region did
not route.

**The model is text.** Each phase writes TOML and resolves it again, about 2 s per phase on
`sdr`, 20 s of the 136.

## Principles

- One board model in memory for the whole run. The file is written once at the end.
- Only the router makes copper. Escape, plane ties and fanout become targets and preferences
  that the router is free to ignore when something else needs the room.
- Every net the engine routes negotiates with every other. The hard obstacles are pads, holes,
  keepouts, the board edge and copper the user drew or locked.
- Global and detail measure capacity from the same geometry, so a corridor global calls free
  has room in detail.
- Routing reports back to placement, and placement moves.

## Board model

`Db` is built once from the resolved `Layout` and holds:

| item | contents |
|---|---|
| parts | position, rotation, side, courtyard, locked flag |
| pads | polygons per layer, net, drill, SMD or through |
| fixed copper | tracks, vias and zones from the file that are locked or outside an engine plan section |
| engine copper | tracks and vias the engine owns, with the net and the round that made them |
| regions | plane regions per rail per layer, keepouts, BGA fences |
| plans | constraint groups, pin access, escape preferences, corridors, overlap per round |

Shapes go into a per layer bin index (1 mm bins) with net, clearance and owner. Rules are resolved
once per net: track width per layer, **track layers** (the class list), **entry layers** (every
layer the net's pads and their vias reach), via options, clearance, pair gap, interface via
budget. The clearance between two shapes is the larger of their two clearances, as the DRC checks
it.

The viewer reads `Arc` snapshots of the `Db` after each round, so it can draw progress without a
file round trip. At the end the engine copper is written into `# plan route` and the placement
into `[[footprints]]`, through `toml_edit` as today. `--checkpoint` writes after every stage.

## Stages

```
constraints -> place -> access -> global <-> detail -> finish
                 ^                  |
                 +---- hot tiles ---+
```

### 1. Constraints

Reads the schematic and the board rules and produces groups that placement keeps:

- **Chains**: from every connector pin, any class, walk two-pin series parts until a part with
  more than two signal pins. RF chains keep their class layer and get a straight line template.
  One finder replaces `placement::chains` and the chain code in `place/rf.rs`.
- **Decap groups**: each decoupling capacitor bound to the supply pin it serves, smallest value
  closest, the pin chosen by net and by the IC's pin count on that rail. A spring to within
  `decoupling_distance`, front side or under the pin on the back.
- **Clock groups**: crystals and oscillators bound to their clock pins.
- **Separation**: switchers from RF and clock groups, hot parts from each other, MLCCs out of the
  flex zone. These stay as score terms.

### 2. Place

The core placer stays as the global placer, with groups as rigid macros: a chain moves as one
line, a decap group as its IC plus springs. Legalise stays. What changes is the input on later
passes: when global or detail report hot tiles, every part whose courtyard touches one is
inflated by the overflow there (cell inflation, as RePlAce does it) and the placement is
legalised again from where it was, not started over. At most `place_rounds` passes, default 3.

### 3. Access

Decides how each pad can be left, without committing anything.

- **Pin access**: for each pad, candidate exits on its own layer (a stub in each of 8
  directions, necked where the class width does not fit) and candidate via sites (in the pad
  where `via_in_pad` and the drill allow it, dog-bone on each diagonal for BGA balls, beside the
  pad for SMD). Each candidate is checked against fixed geometry with the same rules the DRC
  uses. A pad with no candidate is reported here, before any routing.
- **Escape**: the min-cost flow per BGA layer that exists in `escape.rs`, run for signal balls
  as well as plane balls. Its output is a preferred layer, direction and via site per ball, which
  detail sees as a cost discount, not as copper.
- **Planes**: the rail regions from `planes.rs`. A plane pad's target becomes "a via site inside
  its rail's region on that layer", and ground pads likewise target any via site on a ground
  plane. Two neighbouring pads may share one via. Pieces the planes could not join are left as
  connections between pins, routed like any other net with the class width.

### 4. Global route

PathFinder on a tile graph, default 1 mm tiles, with capacity measured at tile boundaries:

- **Edge capacity** is the free length along the boundary between two tiles on a layer, after
  cutting out fixed obstacles grown by clearance, in mm. A net crossing it uses its width plus
  clearance; a pair uses both widths, the gap and one clearance. Mixed widths add up exactly.
- **Via capacity** per tile is the number of legal via sites in it, counted as access counts them.
- **Terminals** are the pin access candidates, so a pad's tile has the capacity of its exits and
  nothing more.
- **Nets** are Steiner trees (MST with 1-Steiner improvement), routed as L and Z patterns first
  and by maze search where those overflow. Plane and ground nets are in, ending at their regions.
- **Cost** is `length * (1 + present * overuse) + history`, with the present factor growing 1.5x
  per round and history rising on every tile edge that overflowed. Inner layers may have a
  preferred direction with a cost against it.

It stops at zero overflow, or after `global_rounds` without improvement. Edges still over then go
back to placement as hot tiles.

### 5. Detail route

Negotiated routing on a fine grid (0.05 mm default), all nets at once, each confined to its
corridor grown by one tile.

- **Fixed geometry** is two distance fields per layer, computed once by a distance transform:
  `d0`, the distance to the nearest fixed copper, and `d1`, the smallest of distance minus that
  shape's clearance. A centre cell is legal for a track of half width `h` and clearance `c` if
  `d0 >= h + c` and `d1 >= h`, which is the larger-of-two-clearances rule for every width and
  class at once, so no grid is rebuilt per class. Each cell also keeps the net of its nearest
  shape; when that is the routing net itself the cell is checked exactly against the bin index.
  Vias are checked the same way with the via radius, plus a drill field for hole to hole and
  hole to pad. On `sdr` the fields are about 60 MB.
- **Engine copper** is soft. Each routed net stamps its copper into an overlap count per width
  bucket (the distinct half width and clearance pairs on the board, usually 3 to 5), with the
  exact reach for that bucket. A step costs `length + present * overlap + history`.
- **Rounds**: every connection is routed in round one. Each later round reroutes only the nets
  that overlap something, the present factor grows and history rises where overlap stayed. It
  stops at zero overlap. A connection that still overlaps after `detail_rounds` goes back to
  global with its tile edges' history raised, and global and detail repeat, bounded by `rounds`.
- **Criticality** replaces class order. RF, impedance and pair nets carry a criticality near 1
  and price length over congestion, so other nets give way to them; ordinary signals carry 0.
  Order no longer decides the result.
- **Trees**: a connection's source is the whole copper the net already has, not the pad the MST
  picked, so a multi-pin net grows as a tree.
- **Pairs** route as one unit with the existing coupled search. **Interface via budgets** become
  a cost: a lane over budget pays more per via each round.
- **Entry**: the via out of a pad is always allowed on the pad's layers; the class's track layers
  apply from the far side of it. This is what lets LVDS leave F.Cu for In2.
- **Search** is today's A*: backward Dijkstra for the heuristic, forward search with bend costs
  and via hops. It moves out of `agentee-core/src/route.rs` into a module both routers share.

### 6. Finish

Via removal and via snapping, octilinear cleanup and chamfers, then a spread pass that moves
tracks to the middle of the free space they run through, then `neck`, `tune`, `fill` and `silk`.
All of these exist; spread is new.

## Score

The terms and weights in [layout-engine.md](layout-engine.md) stay, and every stage reports the
score per term. `unrouted` counts plane and ground connections too, since they are now routed.
Two terms are added: `copper_overlap`, the nets detail still found overlapping before its last
pass (weight 200), and `access`, pads with no legal exit (weight 200). `overflow` is now the
global route's overflow in mm.

## Configuration

```toml
[engine]
phases = ["constraints", "place", "access", "global", "detail", "finish"]
rounds = 3                     # global and detail repetitions
place_rounds = 3               # placement passes driven by hot tiles

[engine.access]
via_in_pad = true
escape_layers = ["F.Cu", "In2.Cu", "B.Cu"]

[engine.global]
tile = "1mm"
rounds = 30
via_cost = "1mm"
direction = { "In2.Cu" = "H" }

[engine.detail]
grid = "0.05mm"
rounds = 30
via_cost = "1mm"
bend_cost = "0.1mm"
fences = true
criticality = { RF = 1.0, LVDS = 0.8, USB_SS = 0.8, Clock = 0.5 }
```

`stages` is accepted for `phases`. `class_order`, `tiers`, `corridors`, `rip_limit`, `pairs`, `tile`,
`[engine.escape]` and `[engine.floorplan]` are gone. Old phase names in `phases` map to the stage
that now does that work (`floorplan` and `legalise` to `place`, `layers`, `escape`, `tie` and
`planes` to `access`, `assign` to `global`), with a warning.

The engine writes `# plan planes` for the rail regions and `# plan route` for all engine copper.
Running a routing stage strips those and the retired `detail`, `escape`, `tie` and `global`
sections first.

## Benchmarks

`cargo test -p agentee-layout --release --test bench` runs each case on a copy and prints routed
connections, vias, DRC errors, score and time. Only `lna` and `sdr` are in it.

| case | today | pass |
|---|---|---|
| `lna` route only, hand placement | 17 of 17, 0.3 s | 17 of 17, RF nets within 10% of hand length, 0 DRC errors |
| `lna` full flow | RF_IN unrouted, RF_AMP_OUT 11.7 mm | J1 to J2 chain in a line, all routed, 0 DRC errors |
| `sdr` route only, committed placement | 280 of 311, 97 unrouted, 101 s | 0 unrouted, under 60 s |
| `sdr` full flow | 283 of 311, 98 unrouted, 289 DRC errors, 136 s | 0 unrouted, decap term under 20, under 120 s |

## Order of work

1. `Db`, resolved rules, one write at the end. `sdr` loses its 20 s of reloads.
2. Detail: distance fields, soft engine copper, negotiated rounds, trees, plane and ground
   targets, entry layers. Run without global, bounded by each connection's box. Gate on the two
   route-only benchmarks.
3. Access: pin access candidates and escape as preferences; delete the `tie` and `escape`
   commits.
4. Global with boundary capacities, corridors on, the global and detail loop.
5. Constraints and one placer, hot tiles back into placement. Gate on the full flow benchmarks.
6. Retire `floorplan`, `layers`, `tie`, the old global and the class order in `route.rs`;
   `agentee route` becomes `layout --only detail` on the named nets.

## Sources

- McMurchie, Ebeling, "PathFinder: a negotiation-based performance-driven router for FPGAs",
  FPGA 1995. Criticality weighting is from the same paper.
- Pan, Chu, "FastRoute 2.0", ASP-DAC 2007, for edge capacity and pattern routes.
- Kahng et al, "TritonRoute", TCAD 2020, for pin access analysis.
- Cheng et al, "RePlAce", TCAD 2018, for routability-driven cell inflation.
- Felzenszwalb, Huttenlocher, "Distance transforms of sampled functions", 2012, for the fields.
- Yan, Wong, "BGA escape routing", ICCAD 2008.

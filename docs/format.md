# agentee file format

A project is a directory. Every file below it is loaded by its suffix:

| suffix | holds |
|---|---|
| `*.board.toml` | one board spec: fab rules, stackup, outline, vias, net classes |
| `*.sym.toml` | one schematic symbol |
| `*.fp.toml` | one footprint |
| `*.sch.toml` | a schematic: placed parts, nets, wires |
| `*.pcb.toml` | a layout: footprint placement, tracks, vias, zones |
| `*.sim.toml` | a simulation of a layout: FDTD S-parameters, cascade, channel, PDN, DC drop or thermal (`kind`) |

Names are unique per kind. A symbol links a footprint by its `name`, and a `Library:Name` reference
matches on the part after the colon.

Run `agentee check` after every edit. Unknown keys are errors, so a typo is reported, not ignored.

## Units and coordinates

- Lengths are numbers in millimetres, or strings with a unit: `"0.2mm"`, `"8mil"`, `"35um"`,
  `"0.1in"`, `"1oz"` (copper, 35 um).
- A point is `[x, y]`. X grows to the right, **Y grows down**, for symbols and footprints alike.
- Rotation is in degrees, counter-clockwise as seen on screen.
- Current `"2A"` / `"500mA"`, impedance `"50ohm"`, temperature rise `"10C"`, tolerance `"10%"`.

## Board spec (`*.board.toml`)

```toml
name = "sensor-node"
description = "2 layer sensor board"
fab = "jlcpcb"                 # rule preset: generic | jlcpcb | hdi

[outline]                      # a rectangle...
size = [50, 30]
origin = [0, 0]                # top-left corner, default [0, 0]
corner_radius = 1
# points = [[0,0], [50,0], [50,30], [0,30]]   # ...or a polygon

[[outline.cutouts]]            # a window, slot or large hole routed through the whole board
origin = [20, 12]              # a rectangle, same keys as the outline
size = [6, 2]
corner_radius = 1              # half the width makes a slot
# points = [[x, y], ...]       # ...or a polygon

[stackup]
preset = "JLC04161H-7628"         # a fab build, see Stackup presets
finish = "ENIG"
mask_color = "green"
silk_color = "white"

[rules]                        # overrides the fab preset, any subset
min_track_width = "0.15mm"

[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
# type = "through"             # through | blind | buried | microvia, default from the span
# from = "F.Cu"                # default: outermost copper on each side
# to = "B.Cu"
# fill = "filled_capped"       # IPC-4761 type, see Via types
# drill_kind = "mechanical"    # mechanical | laser | controlled_depth, default from the type
# cost = 1.0                   # router cost of this via, times --via-cost

[[netclasses]]
name = "Default"               # always define Default
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"                    # or a list the router picks from: ["std", "uvia-top"]

[[netclasses]]
name = "Power"
track_width = "1mm"
current = "3A"                 # checked against IPC-2221 on every layer
max_temp_rise = "10C"          # default 10C

[[netclasses]]
name = "USB"
impedance = "90ohm"            # target, checked on every layer in `layers`
impedance_tolerance = "10%"    # default 10%
diff_gap = "0.15mm"            # makes it a differential pair
layers = ["F.Cu"]              # default: every copper layer
# track_width omitted: solved for the target on the first layer
# widths = { "B.Cu" = "0.16mm" } # per layer width where one width does not fit every layer
# max_uncoupled = "1.5mm"      # total run allowed off the pair gap, default 20% of the length
# neckdown = "1.5mm"           # how far a track may run below class width into a pad, default 0.5mm

[[netclasses]]
name = "RF"
impedance = "50ohm"
coplanar_gap = "0.2mm"         # grounded coplanar: pour this far either side, plane below
layers = ["F.Cu"]              # outer layers only
solver = "field"               # check with the GPU field solver (mask, thickness) instead of formulas
```

Board cutouts sit with the outline in the board file because they are part of the board's shape:
the outline is the outer edge and each cutout is an inner edge, both milled from the same
Edge.Cuts profile. The layout's `[[cutouts]]` only keep zone copper off an area on some layers and
leave the board whole. A cutout must lie inside the outline. Everything that uses the outline treats
a cutout edge as board edge: the edge rules (`copper-to-edge`, `pad-to-edge`, `pad-off-board`,
`part-to-edge`, `part-body-to-edge`, `hole-to-edge`, `edge-pad-reach`, the `mlcc-flex-zone` rules,
which name the cutout), silk and the watermark, pours (kept `min_copper_to_edge` off), stitching
vias, test points, the router and tuner, the fab Edge_Cuts profile, the viewers and the FDTD,
thermal and DC models, where a cutout is air. A stored fill goes stale when a cutout changes.

### Stackup presets

`preset` names a fab's standard build. The presets ship with agentee, in
`crates/agentee-core/stackups/*.toml`:

| fab | names | builds |
|---|---|---|
| JLCPCB | the fab's code, `JLC06161H-1080` (layers, thickness, outer and inner copper, prepreg) | every impedance controlled build, 4 to 20 layers, 0.4 to 3.0 mm, plus `jlcpcb-2l-1.6mm` |
| PCBWay | `pcbway-6l-1.6mm-1oz-1oz-70-2116-7628` (layers, thickness, outer and inner oz, inner copper %, prepregs) | the standard through-hole builds, 4 to 18 layers |
| generic HDI | `hdi-6l-1n1`: 6 layer 1+4+1, 0.8 mm, 1080 build-up prepreg (0.07 mm) over a core, 2116, core sub-stack; `hdi-8l-2n2`: 8 layer 2+4+2, 1.0 mm, two 1080 build-up layers each side over the same sub-stack | not a fab's published build |

`agentee stackups` lists them (`--fab`, `--layers`, `--thickness`, `--search`) and
`agentee stackups NAME` prints one preset's layers; over MCP it is the `stackups` tool. Names
match without regard to case. `jlcpcb-4l-1.6mm-7628` and `jlcpcb-4l-1.6mm-3313` are the old names
of `JLC04161H-7628` and `JLC04161H-3313`.

JLCPCB layer thicknesses come from the data behind jlcpcb.com/impedance. Their er comes from the
calculator guide: Nan Ya NP-155F by core thickness and prepreg for 4 to 8 layers, Shengyi
S1000-2M for 10 and more. Where a table lacks an entry the value is the other system's for the
same prepreg, or the nearest core thickness in the same system. PCBWay thicknesses are after
lamination, split between stacked prepreg plies by their raw thickness, with the DK printed on
each build. Loss tangent is 0.02 throughout; neither fab publishes it per build.
`crates/agentee-core/stackups/fetch.py` refetches both.

Or list the layers yourself, top to bottom. Use either `preset` or `layers`, not both.

```toml
[stackup]
[[stackup.layers]]
kind = "silk"
[[stackup.layers]]
kind = "mask"
thickness = "15um"
er = 3.8
[[stackup.layers]]
kind = "copper"                # named F.Cu, In1.Cu ... B.Cu in order, or set `name`
thickness = "1oz"
[[stackup.layers]]
kind = "prepreg"               # prepreg | core
material = "7628"
thickness = "0.2104mm"
er = 4.4
loss_tangent = 0.02
# ... more copper and dielectric ...
```

Layer kinds: `silk`, `paste`, `mask`, `copper`, `core`, `prepreg`. Copper must alternate with
dielectric; mask, paste and silk sit outside the outer copper.

### Lamination

The lamination is the build-up sequence: the drill steps the fab runs between presses, in build
order, each a copper span and a drill kind (`mechanical`, `laser` or `controlled_depth`). A via is
drillable only when its span and drill kind match a step (a `stacked` microvia: when every hop of
the stack is a laser step). A via type a layout places is reported once, by the layout's
`via-lamination` rule, which counts its vias and names the board entry (`vias[i]`) to fix; Check
reports a via type no layout places as a board error. The router leaves such a class via out.
Each lists the spans the lamination drills.

Without `lamination` the sequence comes from the stackup. The build-up layers on each side are the
prepreg-only dielectrics on the outside before the first core, counted per side, so a build can be
asymmetric (`1+4+2`); the `N` layers between are the core sub-stack, pressed once from its cores
and prepregs. The steps are:

1. each core of the sub-stack, drilled mechanically before its press (a buried via in one core);
2. the sub-stack, mechanically, after its press;
3. for each press of build-up layers, inside out: a laser step from each new outer layer to the
   layer under it (and over two dielectrics, a skip via, from that side's second build-up layer
   on), then a mechanical step through everything pressed so far; the last is the through drill.
   The side with fewer build-up layers joins the later presses, so both outer foils go on in the
   last press: on `1+4+2` the first press adds In5.Cu under B.Cu alone, the second F.Cu and B.Cu;
4. controlled depth from F.Cu and from B.Cu to every inner layer, after the last press.

On `hdi-6l-1n1` (1+4+1) that is In1.Cu-In2.Cu and In3.Cu-In4.Cu (cores), In1.Cu-In4.Cu (buried,
the sub-stack), laser F.Cu-In1.Cu and In4.Cu-B.Cu, the through drill F.Cu-B.Cu, and the controlled
depth spans. On `hdi-8l-2n2` (2+4+2) the first build-up adds laser In1.Cu-In2.Cu and
In5.Cu-In6.Cu and a buried In1.Cu-In6.Cu, the second laser F.Cu-In1.Cu, In6.Cu-B.Cu and the skip
vias F.Cu-In2.Cu and In5.Cu-B.Cu, so a stacked microvia F.Cu-In2.Cu is two laser steps, while
F.Cu-In2.Cu on `hdi-6l-1n1` would cross its first core. A stackup with no core, or a core
outermost, is one press: its sub-stack is the whole board. Whether the fab stacks microvias or
wants them staggered stays `stacked_microvias`.

`lamination` lists the steps instead, for a build the stackup does not show (a sub-stack pressed
on one side first, a laser step into a core):

```toml
[stackup]
preset = "hdi-6l-1n1"
lamination = [
  { from = "In1.Cu", to = "In4.Cu", drill_kind = "mechanical" },
  { from = "F.Cu", to = "In4.Cu", drill_kind = "mechanical" },   # F.Cu to In4.Cu pressed first
  { from = "F.Cu", to = "B.Cu", drill_kind = "mechanical" },
]
```

`from` is above `to`, a controlled depth step runs from one outer layer to an inner layer. The
steps are in build order, and a stack is only built from the inside out: a `stacked` microvia type
whose outer hop is drilled at an earlier step than the hop under it is an error, and `stacked-via`
reports two vias at one spot where the via under the stack is drilled after the one on top (see
Via types). The derived sequence is always in build order.

### Via types

A board `[[vias]]` entry is a via type: its hole, pad, the copper layers it spans and how the fab
builds it, as KiCad, Altium and HDI fabs name them.

```toml
[[vias]]
name = "uvia-top"
type = "microvia"              # laser drilled, one dielectric: F.Cu to In1.Cu
from = "F.Cu"
to = "In1.Cu"
drill = "0.1mm"
diameter = "0.25mm"
fill = "filled_capped"         # copper filled and capped, for via-in-pad
# stacked = true               # a stack of microvias, one per dielectric from..to
# skip = true                  # one laser hole over two dielectrics

[[vias]]
name = "core"
type = "buried"                # inner to inner, drilled in a laminated sub-stack
from = "In1.Cu"
to = "In4.Cu"
drill = "0.2mm"
diameter = "0.45mm"

[[vias]]
name = "bd"                    # a through via backdrilled from the bottom
drill = "0.3mm"
diameter = "0.6mm"
backdrill = { from = "B.Cu", to = "In2.Cu", max_stub = "0.2mm" }
# backdrill.diameter defaults to the drill + 0.2 mm
```

| type | span | drilled |
|---|---|---|
| `through` | first to last copper layer | mechanically, after the last lamination |
| `blind` | one outer layer to an inner layer | mechanically, in a sub-stack, or to a controlled depth (`drill_kind = "controlled_depth"`) |
| `buried` | inner layer to inner layer | mechanically, in a laminated sub-stack |
| `microvia` | one dielectric (`stacked`: several, `skip`: two) | by laser, in a build-up layer |

Without `type` the span decides: first to last layer is `through`, one outer end is `blind`,
two inner ends `buried`. Check holds the type to its span (a blind via must touch exactly one
outer layer, a buried via none) and to the fab:

- Any type but `through`, and any backdrill, needs sequential lamination: the `generic` and
  `jlcpcb` presets build through vias only and report such a via as an error, the `hdi` preset or
  `[rules] hdi = true` allows them.
- The span and drill kind must match a drill step of the lamination (see Lamination); the error
  lists the spans it drills. `drill_kind` defaults to `laser` for a microvia and `mechanical`
  otherwise; a microvia is always laser drilled and a laser drilled via is a microvia.
- A controlled depth via (`drill_kind = "controlled_depth"`) is a blind via drilled mechanically
  from its outer layer after the last press, stopping on its inner layer by depth. Its drill is at
  least `min_controlled_depth_drill` (and `min_via_drill`), its depth (outer copper through the
  target copper) over its drill at most `max_controlled_depth_aspect_ratio`, and nothing stacks on
  or under it: the drill stops by depth on a plain pad, so `stacked-via` reports any via at its
  spot. The fab gets it in its own drill file, `drill-FROM-TO-controlled-depth.drl`, apart from
  the sequentially drilled vias of the same span in `drill-FROM-TO.drl`, and the fab notes name
  each via type with its drill file, the controlled depth ones as drilled to a controlled depth
  from their side.
- A microvia spans one dielectric unless `stacked` (a stack of single microvias, which needs
  `[rules] stacked_microvias = true` and copper filled microvias below) or `skip` (one laser hole
  over two dielectrics). Its drill is between `min_microvia_drill` and `max_microvia_drill`, its
  pad at least `min_microvia_diameter`, and its depth over the drill at most
  `max_microvia_aspect_ratio`, for each dielectric of a stack. IPC-T-50 defines a microvia as a
  blind structure of at most 1:1 aspect ratio and 0.25 mm depth; a deeper one is a warning.
- Blind and buried vias drill at least `min_blind_via_drill` (and `min_via_drill`).
- A backdrill runs `from` an outer layer the via reaches and stops before `to`, a layer strictly
  inside the span, which stays connected. The via then has copper only from its far end to `to`,
  and the stub left is at most `max_stub` (default 0.25 mm). Microvias and buried vias cannot be
  backdrilled.

`fill` is the IPC-4761 via protection type: `tented` (I), `tented_covered` (II), `plugged` (III),
`plugged_covered` (IV), `filled` (V), `filled_covered` (VI), `filled_capped` (VII). Via-in-pad
wants type VII; a via in a pad whose type sets any other fill is a `via-in-pad-fill` error, and a
via in a pad with no `fill` is listed in the fab notes to fill and cap as before.

A via occupies only the copper layers it spans (after a backdrill, the layers left). Everything
that places or reads vias follows that: connectivity, shorts and clearance, zone fill (a pour on a
layer the via does not reach keeps no antipad for it), stitching (the via's layers), fanout (the
first class via that reaches the pad's layer), the unrouted and dangling-end checks, the
interface via stubs (a blind via has no stub past its end layer, a backdrilled via keeps its
`max_stub`), `hole-to-copper` and `aspect-ratio` over the hole's span, `hole-to-hole` only for
holes whose spans share a dielectric, and `stacked-via`: two vias at one spot through the same
dielectric are drilled twice, while vias meeting at one layer (a microvia on a buried via, or a
stack of microvias) are stacked vias, allowed only with `stacked_microvias` and in build order:
of two vias meeting at one layer the one nearer the board surface is on top, and the one under it
must be drilled at the same or an earlier lamination step (a microvia on a buried via needs the
buried via's step first). Two vias as near the surface as each other stack in either order. Silk text keeps off only the vias whose
hole opens on its side, so a buried via under a label is fine. The DC, thermal
and FDTD models run a barrel only between the via's first and last layer; a backdrilled via also
keeps its stub, `max_stub` of barrel past the stop layer toward the drilled side (short of the
next layer): FDTD meshes it as metal to the nearest mesh plane, thermal conducts along it as
copper and through the rest of the dielectric as FR-4, and DC leaves it out since a dead end
carries no current. The 2D viewer
draws a via only when one of its layers is shown, rings it in its type's colour (through gold,
blind teal, buried lavender, microvia cyan) with its two end layers' colours on the rim, and marks
a controlled depth via with a dot of its ring colour in the hole, its plated bottom. The 3D
viewer drills the board face only on the sides a via reaches and ends its barrel at its span; a
controlled depth via ends in a plated drill point, a cone 0.6 of its drill radius deep (a 118
degree drill) past its stop layer, where a sequentially drilled via ends flat; a
backdrill shows as a wider unplated hole, at the backdrill diameter, from the drilled face to the
stub, where the plated barrel ends.

A net class may list several vias, `via = ["std", "uvia-top", "uvia-bot"]`. A layout
`[[vias]]` entry and `[[stitching]]` use the first unless they name one, `[[fanouts]]` the
first that reaches the pad's layer, and the router picks, at each layer change, the cheapest
listed via whose layers hold both ends (`cost` times `--via-cost`, ties go to the via
that spans fewer layers), so with `["std", "uvia-top"]` a change from F.Cu to In1.Cu takes the
microvia and a change to In2.Cu the through via. It changes layer twice at one spot (a microvia
on a buried via) only with `stacked_microvias`, otherwise it staggers them, and never on or under a
controlled depth via. Where it changes layer twice at one spot and a listed via spans both
changes, it places that one via instead, when it keeps clear of the fixed copper and of the
copper and via holes routed in the same pass. A class via the lamination cannot drill is left
out; a class with none left fails with the spans the lamination drills.

### Rules

All lengths: `min_track_width`, `min_clearance`, `min_drill`, `min_via_drill`, `min_via_diameter`,
`min_annular_ring` (vias, microvias use `min_microvia_diameter`), `min_blind_via_drill`,
`min_controlled_depth_drill`, `min_microvia_drill`, `max_microvia_drill`, `min_microvia_diameter`, `min_hole_to_hole`, `min_copper_to_edge`, `min_silk_width`,
`min_silk_text_height`, `min_mask_web` (0.1 mm), `max_drill`, `min_npth_drill`, `min_plated_slot_width`,
`min_npth_slot_width`, `min_pth_annular_ring`, `min_via_hole_to_copper`, `min_pth_hole_to_copper`,
`min_inner_pth_hole_to_copper`, `min_npth_to_copper`, `min_smd_pad_gap`, `min_hole_to_smd_pad`,
`max_filled_via_drill`, `min_bga_pad`, `min_bga_pitch`, `min_part_to_edge`, `min_body_to_edge`,
`flex_zone`; plus
`max_aspect_ratio`, a plain number (hole depth over drill), `max_microvia_aspect_ratio` (dielectric
depth over laser drill), `max_controlled_depth_aspect_ratio` (depth over drill of a controlled depth
via), and the switches `hdi` (the fab builds blind, buried and microvias and
backdrills) and `stacked_microvias` (the fab stacks microvias). Footprints are checked against
the rules of the board when the project has exactly one board, otherwise against `generic`.

The fab preset is a table keyed by the copper layer count, the outer copper weight (from the
first copper layer's thickness) and the finish, so a 4 layer 1 oz board gets tighter track rules
than a 2 layer 2 oz one. Anything in `[rules]` overrides the table. The `jlcpcb` values come from
<https://jlcpcb.com/capabilities/pcb-capabilities>:

| rule | 1 layer | 2 layers | 4+ layers | source line |
|---|---|---|---|---|
| `min_track_width`, `min_clearance`, 1 oz | 0.10 | 0.10 | 0.09 | min. track width and spacing (1 oz) |
| same, 2 oz | 0.16 | 0.16 | 0.15 | min. track width and spacing (2 oz) |
| same, 2.5 / 3.5 / 4.5 oz | | 0.2 / 0.25 / 0.3 | | 2 layer heavy copper |
| `min_drill`, `min_via_drill` | 0.3 | 0.15 | 0.15 | drill diameter, min. via hole size |
| `min_via_diameter` | 0.5 | 0.25 | 0.25 | min. via diameter |
| `min_annular_ring` (via) | 0.05 | 0.05 | 0.05 | via diameter 0.1 mm over the hole |
| `min_pth_annular_ring`, 1 oz | 0.18 | 0.18 | 0.15 | PTH annular ring, absolute minimum |
| same, 2 oz | 0.254 | 0.254 | 0.254 | PTH annular ring, 2 oz |
| `max_drill` | 6.3 | 6.3 | 6.3 | drill diameter, larger holes are routed |
| `min_npth_drill` | 0.5 | 0.5 | 0.5 | min. non-plated holes |
| `min_plated_slot_width` | 0.5 | 0.5 | 0.35 | min. plated slot width, slot at least 2 widths long |
| `min_npth_slot_width` | 1.0 | 1.0 | 1.0 | min. non-plated slots |
| `min_via_hole_to_copper` | 0.2 | 0.2 | 0.2 | via hole to track, inner layer via hole to copper |
| `min_pth_hole_to_copper` | 0.28 | 0.28 | 0.28 | PTH to track |
| `min_inner_pth_hole_to_copper` | | | 0.3 | inner layer PTH pad hole to copper |
| `min_npth_to_copper` | 0.2 | 0.2 | 0.2 | NPTH to track |
| `min_smd_pad_gap` | 0.15 | 0.15 | 0.15 | SMD pad to pad clearance, different nets |
| `max_filled_via_drill` | 0.55 | 0.55 | 0.55 | via-in-pad epoxy or copper fill, 0.15 to 0.55 mm |
| `min_bga_pad` | 0.25 (0.2 ENIG) | | | BGA pad, 0.2 to 0.25 mm needs ENIG |
| `min_bga_pitch` | 0.3 | 0.3 | 0.3 | PCBA capabilities, standard: 0.3 mm BGA centre to centre |
| `min_silk_width`, `min_silk_text_height` | 0.15, 1.0 | | | legend line width, text height |
| `max_aspect_ratio` | 10.7 | 10.7 | 10.7 | the 0.15 mm drill on a 1.6 mm board |

Where agentee is stricter than the page, on purpose: `min_copper_to_edge` stays 0.3 mm (the page
allows 0.2 mm on a routed edge, which is milled to +/-0.2 mm, and 0.4 mm on a V-cut),
`min_hole_to_hole` stays 0.5 mm (the page gives 0.45 mm between pad holes and 0.2 mm between
vias), and `min_hole_to_smd_pad` (0.2 mm, the via hole to track figure) and `min_part_to_edge`
(0.5 mm) are agentee's choices, not on the page. `min_body_to_edge` (1.0 mm) follows assembly DFM
guides, which keep every component 1 mm from the board edge for depaneling and handling.
`flex_zone` (5 mm) follows Knowles on MLCC flex cracking: the stress zone is typically within
5 mm of the PCB edge or fixing points.

The `generic` and `jlcpcb` presets build through vias only (`hdi = false`); their microvia and
blind via limits (0.1 mm laser drill, 0.3 mm pad, 0.8:1, 0.2 mm blind drill) apply once
`[rules] hdi = true` is set. The `hdi` preset is `generic` with PCBWay's published HDI capability
table (https://www.pcbway.com/hdi-pcb.html, "HDI PCB Manufacturing Capabilities": builds 1+N+1 to
6+N+6, 0.065/0.065 mm trace and space, 0.15 mm mechanical drill, 4 mil laser drill standard and
8 mil at most on a dielectric of 0.15 mm or less, 0.15 mm controlled depth PTH drill, 14:1 aspect
ratio, 4 mil green mask bridge), for the IPC-2226 builds: type I (one microvia layer, 1+N+1), type II
(type I with buried vias in the core) and type III (two or more microvia layers, 2+N+2), on the
`hdi-6l-1n1` and `hdi-8l-2n2` stackups. Rules the table does not give keep a named reason:

| rule | `hdi` | source |
|---|---|---|
| `hdi`, `stacked_microvias` | true | PCBWay builds 1+N+1 to 6+N+6 by sequential lamination; stacked microvias are copper filled |
| `min_microvia_drill`, `max_microvia_drill` | 0.1, 0.2 | PCBWay min laser drill 4 mil standard, max laser drill 8 mil |
| `min_via_drill`, `min_drill`, `min_blind_via_drill` | 0.15 | PCBWay min mechanical drill 0.15 mm, min controlled depth PTH drill 0.15 mm |
| `min_controlled_depth_drill` | 0.15 | PCBWay "Min. controlled depth drilling, PTH: 0.15mm" |
| `min_track_width`, `min_clearance` | 0.065 | PCBWay min trace/spacing 0.065 mm |
| `max_aspect_ratio` | 14 | PCBWay max 14:1 |
| `min_mask_web` | 0.1 | PCBWay 4 mil green mask bridge (the `generic` value) |
| `min_microvia_diameter` | 0.25 | not in the table: capture pad, 0.075 mm ring on the 0.1 mm drill |
| `max_microvia_aspect_ratio` | 0.8 | not in the table: under the 1:1 of IPC-T-50, room for plating to fill; PCBWay's 8 mil drill on 0.15 mm is 0.75:1 |
| `min_via_diameter`, `min_annular_ring` | 0.35, 0.1 | not in the table: 0.1 mm ring on the 0.15 mm drill |
| `min_hole_to_hole`, `min_via_hole_to_copper` | 0.25, 0.15 | not in the table |
| `min_bga_pad`, `min_bga_pitch` | 0.2, 0.4 | not in the table |

A 0.07 mm 1080 build-up layer under a 0.1 mm laser drill is 0.7:1, inside the 0.8:1.

`max_controlled_depth_aspect_ratio` is 1 in every preset, from Sanmina's fab note "Backdrilling and
Blind/Buried Via Formation" (https://www.sanmina.com/pdf/solutions/bbf.pdf): blind via formation
by controlled depth drilling "is limited by the throw of copper-plating baths to a maximum aspect
ratio of 1:1". No published source gives a controlled depth drill for `generic` or `jlcpcb`:
JLCPCB's capabilities page lists laser blind vias (0.075 to 0.15 mm), mechanical buried vias and
backdrilling but no controlled depth drilling. So both presets reject a controlled depth via like
any other via but `through` (`hdi = false`), and `min_controlled_depth_drill` there is not a number
of its own but their blind via drill (`min_blind_via_drill`, 0.2 mm), applied once `hdi = true`;
set `min_controlled_depth_drill` in `[rules]` from your fab's figure. On `hdi-6l-1n1` a 0.15 mm controlled depth drill reaches
In1.Cu or In4.Cu (0.123 mm deep), not In2.Cu.

### Design rule checks

Layout checks run from a registry of rules, each with a stable id, a category (`copper`,
`drill`, `mask`, `silk`, `assembly`, `zone`, `signal`, `test`, `placement`), a default severity, and a condition on the
board: a rule for inner layers runs only with 4 or more copper layers, a via fill rule only when
vias sit in pads, a BGA rule only when there is a BGA. Every message of a rule starts with its id
in brackets, e.g. `[via-cuts-pad]`. `agentee drc NAME --list` (MCP `drc` with `list = true`) prints
every rule with its category, severity, and whether it applies to this board and why; `agentee
drc NAME` prints the layout's rule messages alone.

```toml
[drc]                          # in the board file
disable = ["silk-width"]       # rule ids to skip; their checks are not computed, so with silk-text
                               # and silk-hidden off `agentee silk` has nothing to move
severity = { "starved-thermal" = "error", "via-in-pad" = "warning" }   # info | warning | error
tombstone_ratio = 3            # copper or feed width one chip pad may have over the other
```

| id | severity | runs when | checks |
|---|---|---|---|
| `via-cuts-pad` | error | always | a via whose copper overlaps or touches an SMD pad while its drill is not fully inside the pad: solder wicks down the barrel and the pad edge is damaged. A via of another net touching a pad is reported as a `short` instead |
| `via-annulus-past-pad` | warning | vias in pads | the drill sits in the pad but the via's annulus reaches past the pad edge under the mask |
| `via-in-pad` | info | vias in pads | counts the vias in SMD pads of their own net (drill inside the pad); `fab-notes.txt` asks the fab to fill and cap exactly these (IPC-4761 type VII) |
| `via-in-pad-fill` | error | vias in pads | a via in a pad drilled wider than `max_filled_via_drill`, or whose via type sets a `fill` other than `filled_capped` (IPC-4761 type VII) |
| `hole-to-smd-pad` | warning | always | a via hole closer than `min_hole_to_smd_pad` to an SMD pad of its own net (or no net) that it does not touch; paste and solder can flow into it |
| `drill-size` | error | always | pad holes under `min_drill` (plated) or `min_npth_drill` (non-plated), or over `max_drill` (larger holes are routed, draw them as cutouts). Via sizes are checked on the board's `[[vias]]` |
| `slot-size` | error | slotted holes | slots narrower than `min_plated_slot_width` or `min_npth_slot_width`, or shorter than twice their width |
| `aspect-ratio` | error | always | plated holes whose depth (the copper and dielectric they pass) over drill exceeds `max_aspect_ratio` |
| `hole-to-copper` | error | always | a via or plated pad hole wall closer than `min_via_hole_to_copper` or `min_pth_hole_to_copper` to copper of another net on a layer the hole passes: tracks, pads, vias, pours |
| `inner-hole-to-copper` | error | 4+ copper layers | a plated pad hole wall closer than `min_inner_pth_hole_to_copper` to another net's copper on an inner layer |
| `npth-to-copper` | error | non-plated holes | a non-plated hole wall closer than `min_npth_to_copper` to any copper, its own net's pour included |
| `hole-to-edge` | error | non-plated holes | a non-plated hole wall closer than `min_copper_to_edge` to the board outline or a board cutout, or through it |
| `smd-pad-gap` | error | always | SMD pads of different nets closer than `min_smd_pad_gap`, one line per pair of parts with the closest pads |
| `pad-to-edge` | error | always | pad copper closer than `min_copper_to_edge` to the board outline or a board cutout; pads marked `edge = true` are exempt |
| `edge-pad-reach` | warning | always | a pad marked `edge = true` that stops short of the board outline |
| `starved-thermal` | warning | zones | a pad joined to a pour of its net over less than half its outline, by fewer than two spokes at least `min_track_width` wide, and with less copper in all than the pad's own width |
| `part-to-edge` | warning | parts | SMD pads closer than `min_part_to_edge` to the outline or a board cutout, where depaneling stress cracks parts; skips fiducials, mounting holes and parts with `edge` pads |
| `part-body-to-edge` | info | parts | a part body closer than `min_body_to_edge` to the outline or a board cutout, or past it (over a cutout counts as past) (assembly DFM guides: no component within 1 mm of the edge). The body is the fab outline, else the courtyard, else the pad copper, and the message names which; skips fiducials, mounting holes, parts with `edge` pads and footprints with `overhang = true` |
| `fiducials` | info | parts | no footprint named like `Fiducial` on the board |
| `tooling-holes` | info | parts | no non-plated hole of 1.5 mm or more |
| `bga-pad` | error | a BGA | BGA pads (16 or more round SMD pads) smaller than `min_bga_pad` |
| `bga-pitch` | error | a BGA | ball pitch finer than `min_bga_pitch` |
| `bga-pad-ratio` | warning | a BGA | pad diameter outside 40% to 65% of the pitch (IPC-7351 land sizes) |
| `paste-without-mask` | warning | parts | a copper pad with paste but no mask opening on that side, so the stencil prints onto mask |
| `mlcc-flex-zone-case` | info | ceramic capacitors | a ceramic capacitor of case 0805 (2012 metric) or larger within `flex_zone` of the outline, a board corner, a board cutout, a mounting hole (a `MountingHole*` footprint) or a non-plated hole of 2 mm or more (Knowles: the stress zone is typically within 5 mm of the PCB edge or fixing points); the longer the chip, the more strain its ends see |
| `mlcc-flex-zone` | info | ceramic capacitors | a smaller ceramic capacitor within `flex_zone` whose long axis points at the nearest edge, corner or hole (Murata FAQ: orient the chip horizontal to the stress direction, so its long axis runs along the edge) |
| `mlcc-flex-zone-info` | info | ceramic capacitors | counts the smaller ceramic capacitors within `flex_zone` that already lie along the edge |
| `tombstone-risk` | info | chips of 0603 or smaller | a two-pad SMD part of 0603 (1608 metric) or smaller whose pads differ in size or shape, that has a via in one pad and not the other, or whose copper within 0.3 mm of one pad on its layer (tracks, vias, pours of its net) is over `tombstone_ratio` (default 3) times that of the other; a pad joined to its pour through a thermal relief is weighed by its spoke width against the track width on the other pad instead, and a pad with no track, via or pour yet is not compared; the end that heats first wets first and stands the part up (EMS DFM guides: symmetric lands and balanced copper on both ends) |
| `tall-part-shadow` | info | part heights over 3 mm | a two-pad chip of 0603 or smaller closer to a part taller than 3 mm than that part's height, measured from the chip's pads to the tall part's body (EMS rule of thumb 1:1: shadowing in reflow and inspection); heights come from the footprint `height`, else the body the 3D view draws without a model: its family default (SOIC, QFN, headers, shields) or an `_h1.25mm` part of the name |
| `test-access` | info | parts | nets the `[test]` section asks for with no probe access from the probe side: no pad of a test point (reference `TP1`..., or a footprint named `TestPoint*`), no exposed plated through-hole pad (`through_holes`), no untented via (`vias`). Nets in classes with an impedance target, and the nets of pairs, are exempt and named, since a stub hurts them |
| `test-pad-geometry` | info | test points | a test point pad under `min_test_pad`, closer than `min_test_pad_pitch` to another centre to centre, closer than `min_test_pad_to_body` to another part's body on the probe side, closer than `min_test_pad_to_edge` to the board edge or a tooling hole (non-plated holes and mounting holes), or not on the probe side |
| `placement-decoupling-distance` | info | parts | a capacitor between a supply and ground whose supply pad is farther than `decoupling_distance` (3 mm) from the nearest pin of an IC on that supply, 2.5 times that for bulk capacitors over 1 uF |
| `placement-crystal-distance` | info | parts | a crystal or oscillator (reference `Y`, or a footprint named like a crystal or oscillator) whose signal pad is farther than `crystal_distance` (5 mm) from the IC pin on its net |
| `placement-large-part-off-centre` | info | parts | a large chip (a BGA, a package of 16 or more pins over 25 mm2, else the parts with the most pins) whose courtyard centre is more than `off_centre` (0.6) of the way from the board centre to the edge |
| `placement-hot-parts-close` | info | parts | two hot parts whose courtyards are closer than `hot_distance` (5 mm): the `[[sources]]` of this layout's thermal sims at 0.25 W or more, and the large packages (courtyard of 49 mm2 or more) those sims do not list |
| `placement-cluster-spread` | info | parts | a two-pin passive whose signal nets reach one IC and nothing else, farther than `cluster_spread` (10 mm) from that IC's pin |
| `placement-connector-not-at-edge` | info | parts | a connector (edge pads, `overhang = true`, or a `J`/`P` reference that is not a Tag-Connect, U.FL or test pad) whose courtyard is farther than `connector_edge` (3 mm) from the outline |
| `short` | error | always | copper of two different nets touches |
| `clearance` | error | always | copper of two nets closer than the larger of their class clearances (a footprint `clearance` replaces them for its pads), or copper run into a non-plated hole |
| `unrouted` | error | always | a net whose pads are not all joined by tracks, vias and pours, naming the groups that are apart |
| `dangling-track` | warning | always | a track end that touches no copper of its net and no pour |
| `track-grazes-pad` | warning | always | tracks that reach a pad only with their edge; run the centre line into the pad |
| `copper-to-edge` | error | always | a track or via closer than `min_copper_to_edge` to the board outline or a board cutout, or off the board (a track across a cutout leaves the board) |
| `pad-off-board` | error | always | pads outside the outline or in a board cutout; pads marked `edge = true` are exempt |
| `stitching` | info | always | counts the vias each `[[stitching]]` entry placed |
| `stitching-empty` | warning | always | a `[[stitching]]` entry that placed no via |
| `fanout-empty` | warning | always | a `[[fanouts]]` entry that placed no via |
| `hole-to-hole` | error | always | holes of different parts or vias closer than `min_hole_to_hole`, wall to wall, counted with the first pair; only holes whose spans share a dielectric count, so a microvia beside a buried via is not a pair |
| `stacked-via` | error | always | a via on the same spot as another via of its net through the same dielectric (drilled twice), stacked on another via at one layer when `stacked_microvias` is off or out of the lamination's build order (the via under a stack drilled at a later step than the one on top), or stacked with a controlled depth via at all |
| `via-lamination` | error | always | vias whose span and drill kind match no drill step of the lamination (see Lamination), per via type and span; the message names the board `[[vias]]` entry and lists every span the lamination drills, by drill kind. The board check leaves a via type a layout places to this rule |
| `neckdown` | info | always | a track narrower than its class width but not under `min_track_width`, on a run up to the class `neckdown` length (0.5 mm by default) |
| `class-width` | error | always | a track narrower than its class width that is not a neck-down |
| `impedance-width` | warning | impedance classes | a track of an impedance class at another width, its impedance moves |
| `track-overlap` | error | always | tracks of one net running on top of each other, the copper is doubled |
| `acute-turn` | warning | always | a track turning back more than 90 degrees, an acid trap |
| `zone-overlap` | error | zones | fills of two nets on one layer overlap, a short |
| `zone-to-zone` | error | zones | fills of two nets on one layer closer than their clearance |
| `zone-clearance` | error | zones | a fill that covers or comes too close to copper of another net |
| `zone-tips` | warning | zones | fill tips sharper than 30 degrees; raise the zone's `min_width` |
| `copper-neck` | warning | zones | necks in a fill narrower than 90% of the zone's `min_width`, which the fill should have opened; counted by place with the narrowest |
| `zone-islands` | info | always | fill islands that reach nothing of the zone's net and were removed |
| `courtyard-overlap` | error | parts | courtyards of two parts on one side overlap by their outline |
| `courtyard-hole` | error | parts | a courtyard that covers a mounting hole or a non-plated hole of another part |
| `mask-web` | error | always | pads of different nets whose mask openings leave less than `min_mask_web`, one line per pair of parts; pads of one footprint with `mask_web = false` are skipped among themselves |
| `silk-text` | error | always | silk text that crowds other text, sits on pads, prints over vias, crosses a silk outline or runs off the board; a reference gets a clear spot (`agentee silk` moves it there) |
| `silk-hidden` | warning | always | silk text only hidden under another part's body |
| `silk-text-height` | warning | always | silk text under `min_silk_text_height` |
| `silk-artwork` | error | always | silk artwork on pads, over silk text or off the board |
| `watermark` | error | always | the `agentee vX.Y.Z-HASH` watermark has no clear spot on the silk, or the `[watermark]` spot is not clear; fab refuses without a spot, and disabling the rule does not remove the watermark |
| `silk-width` | warning | always | board silk lines (the layout's `[[graphics]]`, not text) thinner than `min_silk_width`, counted with the thinnest; footprint silk is checked with the footprint |
| `pair-skew` | error | pairs | a pair skewed over its `max_skew` or the class `max_skew`, with the net to lengthen |
| `pair-skew-info` | info | pairs | the skew of each pair within its limit |
| `pair-gap` | error | pairs | a pair run side by side at another gap than the class `diff_gap`, beyond `max_uncoupled` |
| `pair-coupling` | warning | pairs | less than 80% of a pair runs side by side at the pair gap |
| `match-length` | error | match groups | a member of a match group off its target by more than the tolerance |
| `interface-pair` | error | interfaces | a net of a differential interface with no pair partner |
| `interface-impedance` | error | interfaces | an interface net whose class has no impedance target, one outside the window, or no pair gap on a differential interface |
| `interface-skew` | error | interfaces | a pair skewed over the interface's `max_skew` |
| `interface-bus-skew` | error | interfaces | the data signals spread more than `max_bus_skew` |
| `interface-clock-window` | error | interfaces | a data signal arriving outside `clock_window` from the clock |
| `interface-vias` | error | interfaces | a lane with more vias than `max_vias` |
| `interface-stub` | error | interfaces | a via stub longer than `max_stub` |
| `interface-return-via` | error | interfaces | a signal via with no reference via within `return_via` |
| `interface-length` | error | interfaces | a lane longer than `max_length` |
| `interface-reference` | error | interfaces | a lane running more than `max_unreferenced` with no reference plane next to it |

The placement rules are guidance for where parts sit, notices by default; raise them with
`[drc] severity`. Their thresholds live in the board:

```toml
[drc.placement]
decoupling_distance = "3mm"    # supply pad of a decoupling cap to the IC pin
crystal_distance = "5mm"
cluster_spread = "10mm"        # a passive to the only IC it serves
hot_distance = "5mm"           # courtyard gap between large packages
connector_edge = "3mm"         # connector courtyard to the outline
off_centre = 0.6               # fraction of the half width or height from the centre
```

The mechanical rules (`part-body-to-edge`, the `mlcc-flex-zone` rules, `tombstone-risk`,
`tall-part-shadow`) are guidance and default to info; raise them with `[drc] severity`. A ceramic
capacitor is a part with exactly two SMD pads whose footprint is named `C_*` or contains
`Capacitor`, or whose reference is `C` and a digit, unless the footprint name says `CP_`,
`Tantal`, `Elec` or `Polymer`; `mlcc` on the footprint or the placement overrides the guess. The
case comes from the footprint name (`1608Metric`, else an imperial code such as `0603`), else
from the distance between the two pad centres, which is close to the body length.

An id that names no rule is a warning. Errors in the files themselves (a net that is not in the
schematic, a layer that is not copper, a bad preset) are not rules and cannot be disabled.

### What check computes

For each net class and routing layer it finds the trace geometry from the stackup (outer layers
are microstrip, inner layers are stripline between the nearest copper above and below), then the
impedance (Hammerstad-Jensen microstrip, Wheeler stripline, conformal-mapping grounded coplanar,
all uncoated and zero-thickness for coplanar), the width that meets the
target, and the IPC-2221 current capacity. `agentee show <board>` prints all of it as JSON.

With `solver = "field"` the class is checked by the 2D field solver instead: a node-based finite
difference Laplace solve on a graded mesh, run on the GPU through wgpu, with copper thickness,
solder mask, coplanar grounds and pairs included. On exact references it lands within 0.5% of
Cohn's zero-thickness stripline and 0.4% of Hammerstad-Jensen microstrip. When the class is off
target, check suggests the track width that meets it. `agentee calc field --netclass RF` prints
the full result (Z0, eeff, C and L per metre, delay, grid).

`--sweep 10MHz,20GHz,21` (MCP `sweep`) adds a loss table per frequency: R, L, G, C, Z0 and
dB per metre and per inch, split into conductor and dielectric loss. Conductor loss comes from
Wheeler's incremental inductance, solved by receding every copper surface in the field solver
(within 1% of the Wheeler stripline formula in Pozar), with skin depth from annealed copper and
the DC resistance blended in as sqrt(Rdc^2 + Rac^2). Dielectrics follow the causal
Djordjevic-Sarkar model fitted to each layer's `er` and `loss_tangent`, taken as 1 GHz values.
Copper roughness is set on the stackup:

```toml
[stackup]
roughness = "0.5um"            # rms, Hammerstad-Jensen
# huray = { radius = "0.5um", ratio = 2.0 }   # or the Huray snowball model
finish = "ENIG"
nickel = "4.5um"               # ENIG nickel thickness, default 4.5 um
gold = "0.075um"               # ENIG immersion gold thickness, default 0.075 um
```

The 2D solver applies the roughness as a loss factor on the resistance at each frequency. The
FDTD uses the roughness and the ENIG finish as described under "Losses in the FDTD" below;
`nickel` and `gold` only matter when `finish = "ENIG"`. The defaults are the middle of the
IPC-4552 windows (3 to 6 um nickel, gold 0.05 um minimum with fabs aiming for 0.05 to 0.1 um).

A class with `diff_gap` is a pair: its `impedance` is the differential impedance, and the solver
runs both the odd and even modes. `calc field` then adds a `pair` block with Zdiff, Zcommon, the
odd and even mode impedances and delays, the coupling coefficient (Ze - Zo)/(Ze + Zo), and the
saturated near-end crosstalk of a long line (half the coupling). The two modes land within 0.5% of
Cohn's exact edge-coupled stripline. A difference between the odd and even delays is what drives
far-end crosstalk on microstrip.

## Symbol (`*.sym.toml`)

```toml
name = "LM358"
reference = "U"                # designator prefix
value = "LM358"                # default: name
description = "Dual op amp"
datasheet = "https://..."
keywords = ["opamp", "dual"]
footprint = "SOIC-8_3.9x4.9mm_P1.27mm"
footprint_filters = ["SOIC*3.9x4.9mm*"]
pin_names = "inside"           # inside | outside | hidden
# pin_name_offset = 0.508
# hide_pin_numbers = true
# power = true                 # a power flag symbol like GND

[[graphics]]
kind = "polygon"
points = [[-5.08, -5.08], [5.08, 0], [-5.08, 5.08]]
fill = "background"            # none | solid | background
unit = 1                       # 0 or absent: drawn in every unit

[[pins]]
number = "3"
name = "+"
type = "input"
at = [-7.62, -2.54]            # the connection point, where wires land
side = "left"                  # left | right | top | bottom of the body
length = 2.54                  # default 2.54
unit = 1
# shape = "inverted"           # line | inverted | clock | inverted_clock | input_low | clock_low | output_low | edge_clock_high | non_logic
# hidden = true
```

`side` says which edge of the body the pin sticks out of. A `left` pin runs from `at` rightwards to
the body. Keep connection points on the 1.27 mm grid (2.54 mm is better).

Pin types: `input`, `output`, `bidirectional`, `tri_state`, `passive`, `free`, `unspecified`,
`power_in`, `power_out`, `open_collector`, `open_emitter`, `no_connect`.

Names take `~{...}` for an overbar: `~{RESET}`.

### Generated bodies

For a box symbol, list pins per side and let agentee place them on the grid and size the body.
Each entry is a pin, or `{ gap = N }` to leave N empty slots.

```toml
name = "STM32F103C8"
reference = "U"

[[bodies]]
unit = 1                       # optional, one [[bodies]] per unit
# width = 20.32                # minimum body width
# pin_length = 2.54
left = [
  { number = "7", name = "NRST", type = "input" },
  { gap = 1 },
  { number = "10", name = "PA0", type = "bidirectional" },
]
right = [ { number = "30", name = "PA9", type = "bidirectional" } ]
top = [ { number = "1", name = "VBAT", type = "power_in" } ]
bottom = [ { number = "8", name = "VSS", type = "power_in" } ]
```

Bodies and hand-placed `[[pins]]` / `[[graphics]]` can be mixed.

Multi-unit symbols: give pins and graphics a `unit`. The same pin number may appear in more than
one unit only when it has the same name (a shared pin).

## Footprint (`*.fp.toml`)

```toml
name = "SOIC-8_3.9x4.9mm_P1.27mm"
description = "SOIC, 8 pin"
tags = ["SOIC", "SO"]
mount = "smd"                  # smd | tht | other, default from the pads
height = "1.5mm"               # body height in the 3D view when there is no model
model = "${KICAD9_3DMODEL_DIR}/Package_SO.3dshapes/SOIC-8_3.9x4.9mm_P1.27mm.step"
model_offset = ["0mm", "0mm", "0mm"]   # optional, as in KiCad: model frame, Y up
model_rotate = [0, 0, 0]               # optional, degrees about X, Y, Z
model_scale = [1, 1, 1]                # optional
# mask_web = false             # the fab opens the mask over all pads of a fine pitch part as one
                               # window, so min_mask_web is not checked between its own pads
# clearance = "0.2mm"          # as KiCad's footprint clearance: its pads keep this from other
                               # copper instead of the net class clearance
# overhang = true              # a connector meant to hang over the board edge: part-body-to-edge
                               # skips it, its SMD pads still keep min_part_to_edge
# mlcc = false                 # not a ceramic capacitor (film, polymer): the mlcc-flex-zone rules
                               # skip it; true marks one that the name does not give away
# net_tie_pad_groups = [["1", "2"]]  # as KiCad's net tie: copper of these pads' nets may touch or
                               # come near any pad of the group (a bridged solder jumper's strip
                               # and the tracks landing on it) without a short or clearance error

[[pads]]
number = "1"
kind = "smd"                   # smd | tht | npth | connect
shape = "roundrect"            # rect | roundrect | circle | oval | custom
at = [-2.475, -1.905]
size = [1.95, 0.6]
roundrect_ratio = 0.25         # default 0.25
count = 4                      # a row of 4 pads...
pitch = [0, 1.27]              # ...each this far from the last, numbered 1, 2, 3, 4

[[pads]]
number = "5"
kind = "smd"
shape = "roundrect"
at = [2.475, 1.905]
size = [1.95, 0.6]
count = 4
pitch = [0, -1.27]             # counts 5..8 back up the right side

[[pads]]
number = "1"
kind = "tht"
shape = "circle"
at = [0, 0]
size = [1.7, 1.7]
drill = 1.0                    # round, or [w, h] for a slot
# rotation = 90
# layers = ["*.Cu", "*.Mask"]  # default by kind: smd F.Cu F.Paste F.Mask, tht *.Cu *.Mask
# edge = true                  # the copper is meant to reach the board edge (edge-launch
                               # connector, castellation, edge finger): exempt from the edge
                               # clearance, listed in the fab notes
# zone_connect = "solid"       # solid | relief | none: this pad's join to a same-net zone,
                               # over the zone's pad_connection (KiCad's pad zone_connect)

[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-3.7, -2.7]
end = [3.7, 2.7]

[[graphics]]
kind = "text"
layer = "F.SilkS"
text = "${REFERENCE}"
at = [0, -3.4]
size = 1.0
```

Numbering in a row counts up the trailing digits (`A1`, `A2`, ...). `number_step = 2` counts by
two. A custom pad is the anchor rectangle from `size` plus `points`, a polygon relative to `at`
(set `size = [0, 0]` for the polygon alone).

Layers: `F.Cu`, `B.Cu`, `F.SilkS`, `B.SilkS`, `F.Mask`, `B.Mask`, `F.Paste`, `B.Paste`, `F.Fab`,
`B.Fab`, `F.CrtYd`, `B.CrtYd`, `Edge.Cuts`, `*.Cu`, `*.Mask`.

Footprint graphics on a copper, mask or paste layer are plotted as copper, mask openings or
paste, and drawn in the viewer: the copper strip of a bridged solder jumper, say. Check does not
see them as copper.

Check looks for overlapping pads, pads closer than the fab clearance, drills and annular rings
under the rules (`min_drill`, `min_pth_annular_ring`), a courtyard that encloses the pads, and
silk that runs over exposed copper.

## Schematic (`*.sch.toml`)

Parts are placed symbols; nets list the pins they join. Wires are drawn for you on a 1.27 mm
grid unless you give them.

```toml
name = "lna"
board = "lna"                  # net classes come from this board
no_connect = ["U2.7"]          # pins left open on purpose

[[parts]]
ref = "U1"
symbol = "SPF5189Z"
value = "SPF5189Z"
at = [66.04, 50.8]             # keep on the 1.27 mm grid
rotation = 90                  # 0, 90, 180, 270, counter-clockwise
mirror = true                  # flip left to right before rotating
# unit = 2                     # one [[parts]] per unit of a multi-unit symbol, same ref
# footprint = "SOT-89-3"       # default: the symbol's footprint
# dnp = true
fields = { mpn = "Qorvo SPF5189Z" }

[[nets]]
name = "RF_OUT"
class = "RF"                   # a netclass of the board; without one the net is in "Default"
pins = ["C2.2", "J2.1", "L3.1"]   # REF.PIN, by number or by a unique pin name
# style = "power"              # wire (default) | label | power (ground and supply symbols)
# wires = [[[x, y], [x, y]], ...] # draw it yourself; check verifies it reaches every pin
```

### Sheets

Split a large design into one schematic per section and join them in a top schematic that lists
them. The layout places the top one.

```toml
name = "sdr"                   # sdr.sch.toml
board = "sdr"
sheets = ["power", "fpga", "rf", "usb"]
```

Each sheet is an ordinary schematic file with its own parts and nets. Nets join across sheets by
name, so `3V3` on every sheet is one net and a signal leaves one sheet and arrives on another
under the same name. A net's pins must be parts on the same sheet; a net that spans sheets needs
the same `class` wherever it names one, and is drawn with labels. References are unique across
the design. A sheet on its own skips the single-pin and Default class warnings (the rest of the net, and its
class, may be on another sheet);
the top schematic runs every check on the joined design and draws the sheets stacked top to
bottom. The top file may hold parts and nets of its own too.

Check reports pins in two nets, pins in no net, single-pin nets, several outputs on one net,
hand wires that miss a pin or touch another net's pin, overlapping parts, and footprints whose
pads do not cover the symbol's pins. Every net in the `Default` netclass is a warning, whether it
names no class or names `Default`: the class sets the net's width, clearance, vias and routing
order, so each net should say what it is. The warning needs a board and a layout of the
schematic; a schematic only simulated (like `examples/logic`) skips it. Give the board a class
per kind of net (`Signal`, `Data`, `Clock`, `Analog`, `Ground`, `Power`, `RF`, pairs) and keep
`Default` as the fallback the layout uses for rules no class covers. `agentee show sch:lna` prints every pin's position.

## Layout (`*.pcb.toml`)

Places the schematic's footprints on the board and routes them.

```toml
name = "lna"
board = "lna"
schematic = "lna"

[[footprints]]
ref = "U1"
at = [13.0, 8.05]
rotation = 90                  # degrees, counter-clockwise
# side = "bottom"              # mirrors the footprint and swaps F./B. layers
label = { at = [9.9, 7.4], rotation = 90 }   # move the silk reference; size = 0.8, hide = true
# mlcc = false                 # this part is not a ceramic capacitor, over the footprint's `mlcc`
# locked = true                # `agentee place` never moves it

[[tracks]]
net = "RF_OUT"
layer = "F.Cu"
points = [[18.68, 10], [25.5, 10]]
# width = 0.36                 # default: the net class width

[[vias]]
net = "GND"
at = [7.2, 8.9]
# via = "std"                  # a [[vias]] name from the board, default the first class via
# count = 6                    # a row, like pads
# pitch = [1.2, 0]

[[fanouts]]                    # a via in every connected pad of a BGA
ref = "U3"                     # * and ? globs: ref = "*" with nets = [...] fans out every plane pad
# via = "bga"                  # or a list, the first reaching the pad layer; default the class vias
# skip_rings = 2               # leave the two outer rings for escape on the outer layer
# always = ["GND", "LVDS_*"]   # nets that get a via even in those rings, globs allowed
# skip = ["A1", "B7"]          # pads to leave alone
# skip_at = [[4.5, 3.0]]       # no via at these spots (the viewer adds one when you edit a via)
# nets = ["GND", "3V3"]        # only pads on these nets, globs allowed
# exclude = ["C2?", "J1"]      # refs to leave out when ref is a glob; a glob never
                               # matches test points, a probe pad keeps no via

[[stitching]]                  # ground vias wherever they clear every other net
net = "GND"
# via = "std"                  # default the net's first class via
# pitch = "2.5mm"              # grid pitch, or the spacing along a fence (default 1mm)
# outline = [[x, y], ...]      # default the board outline
# margin = "0.6mm"             # extra distance from the board edge
# fence = ["RF_*"]             # instead of a grid: a row either side of these nets' tracks
# offset = "0.5mm"             # fence row distance from the track centre, default just past
                               # the class coplanar gap
# skip_at = [[4.5, 3.0]]       # no via at these spots (the viewer adds one when you edit a via)

[[zones]]
net = "GND"
layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
# outline = [[x, y], ...]      # default: the board outline
# clearance = 0.25             # default: the net class clearance
# priority = 1                # higher fills first; other nets' zones on the layer pour around it
# min_island_area = 2.0       # mm2; a piece touching one item of the net is kept only this big
# pad_connection = "relief"    # solid (default) | relief: pads of the net join the pour by four
                               # spokes across a gap, so they heat like a track-fed pad;
                               # none: the pour keeps its clearance from the pads. A pad's
                               # own zone_connect wins, and BGA balls (16 or more round SMD
                               # pads) stay solid, since spokes starve a ball of solder heat
# relief_tht_only = true       # relief on through-hole pads only, SMD pads solid (KiCad's
                               # thru_hole_only)
# relief_gap = "0.3mm"         # default: the zone clearance
# spoke_width = "0.3mm"        # default: the net class track width, at least min_width

[[cutouts]]                    # keep zones off an area, e.g. under an SMA centre pin;
                               # a hole through the board is [[outline.cutouts]] in the board file
layers = ["In1.Cu"]
points = [[0, 9], [5.4, 9], [5.4, 11], [0, 11]]

[[graphics]]                   # board text and lines, same keys as footprint graphics
kind = "text"
layer = "F.SilkS"              # F.SilkS, B.SilkS, F.Fab or B.Fab
at = [7.4, 7.6]
# locked = true                # `agentee place` never moves it
text = "RF IN"
size = 1.0                     # mm, the fab minimum is in the board rules

[[artwork]]                    # a filled logo or icon
layer = "F.SilkS"
icon = "arrow"                 # built in: arrow, warning, ground, antenna, lightning, pin1, ce
# file = "logo.svg"            # or any SVG, relative to this file
at = [7.4, 6.3]                # centre of the artwork
height = 0.8                   # mm, the width follows the aspect ratio
# rotation = 90

[test]                         # in-circuit and flying probe test access, all optional
# nets = ["3V3", "*RST*"]      # nets that need a probe, globs, any case; default below
# exclude = ["LED_*"]          # nets to leave out
# side = "B"                   # probe side, F or B
# through_holes = true         # exposed plated through-hole pads count as access
# vias = false                 # vias count as access; opens the probe side mask over every via
# min_test_pad = "1.0mm"       # smallest test pad
# min_test_pad_pitch = "1.27mm"   # centre to centre between test pads
# min_test_pad_to_body = "1.0mm"  # to another part's body on the probe side
# min_test_pad_to_edge = "3.0mm"  # to the board edge and tooling holes

[place]                        # optional, for `agentee place`
# edges = { J1 = "left", J2 = "right" }   # pin a connector to an edge: left, right, top
                               # (smallest y) or bottom
# keepouts = [[[0, 0], [8, 0], [8, 6], [0, 6]]]   # polygons no courtyard may enter

[watermark]                    # optional: where the agentee version watermark goes
# at = [30, 21]                # centre of the text, default a clear spot found by check
# layer = "B.SilkS"            # default B.SilkS, else F.SilkS
# rotation = 90                # default 0, or 90 where only a tall gap is clear
```

Every layout carries silk text `agentee vX.Y.Z-HASH`: the version of the agentee that built the
package and the short git hash of its source, with `-dirty` when the tree had uncommitted
changes, or `unknown` outside git. It is always plotted and cannot be turned off. It is
`min_silk_text_height` tall, centred, and by default goes on `B.SilkS` (`F.SilkS` when the bottom
has no room or the board has no bottom silk) at the clear spot nearest the board's bottom left
corner, rotated 90 degrees if only a tall gap fits. A clear spot keeps off pads, vias, other silk
text and lines, artwork and part bodies, and stays `min_copper_to_edge` inside the outline. The
viewer, render and assembly drawings show it, and `fab-notes.txt` names it. When no spot is clear,
check reports a `watermark` error with the size to clear and the least crowded spot, and fab
refuses; clear room there or set `[watermark] at` (plus `layer`, `rotation`) yourself. A
`[watermark]` spot that is not clear is a `watermark` error naming what it hits, and a stackup
with no `kind = "silk"` layer is a `watermark` error asking for one.

Test access: by default the nets that need a probe are power nets (a class with `current`, or a
name like `3V3`, `1V8`, `+5V`, `VCC*`, `VDD*`, `VBUS*`, `VBAT*`, `VIN*`, `VSYS*`), ground (`GND`,
`*GND`, `GND*`) and nets named like `*RST*`, `*RESET*`, `*EN*`, `*PG*`, `*CLK*`, `*TX*`, `*RX*`,
`*SCL*`, `*SDA*`, `*SWD*`, `*TCK*`, `*TMS*`, `*TDI*`, `*TDO*`. The defaults leave out switch
nodes and regulator feedback, soft-start and noise-reduction nets (`SW*`, `*_SW`, `LX*`, `FB*`,
`SS*`, `NR*`, matched on the whole name or its last `/` segment), even when their class is power:
a probe's capacitance and the stub to the pad couple switching noise into a feedback divider or
slow a soft-start or noise filter, and a stub on a switch node radiates. `nets` replaces that list
(name such a net there to probe it anyway) and `exclude` takes nets out of it. A test point is a part with a reference `TP` and a number, or a
footprint named `TestPoint*`; like every part it must be in the schematic. The built in
`TestPoint_Pad_D1.0mm` footprint (a 1.0 mm round SMD pad, mask open, no paste) and `TestPoint`
symbol are written into `footprints/` and `symbols/` by `agentee testpoints`.

`agentee testpoints NAME --nets "3V3,*RST*" [--side B] [--pitch 2.54]` (MCP `testpoints`) adds a
test pad to each matching net that has no probe access yet (nets are the `[test]` defaults when
`--nets` is left out, impedance and pair nets are skipped): it looks on the probe side for a free
spot on a `--pitch` grid near the net's copper, keeping `min_test_pad_to_edge` from the edge and
tooling holes, `min_test_pad_to_body` from part bodies, `--pitch` from other test pads, and the
net clearance from other copper and from the pads, stubs and vias it placed for other nets. It
adds a `TP` part joined to the net to the schematic sheet that names the net, a `[[footprints]]`
entry on the probe side to the layout with a short stub track to a via beside the pad, then
routes each new pad to the net's copper with the autorouter and appends those tracks and vias.
A pad the router cannot join is taken out of the schematic and layout again and listed with the
nets that found no spot. Labels of the new pads are moved to a clear spot or hidden, and when the
layout stores zone fills they are refreshed as `agentee fill` would. `--dry-run` reports the
spots without writing.

Silk text must keep 0.4 mm from other silk text and 0.2 mm from silk outlines, stay off pads,
vias and other parts' bodies, and stay on the board. Each of these is an error, except text under
another part's body, which is a warning. When a reference label fails, check names a
spot that passes every rule, as a `label = { at = [...] }` line to paste. `agentee silk NAME`
(MCP `silk`) pastes them all for you and repeats until the labels settle; `--hide` hides the
references that have no clear spot, typically small passives under a BGA.

A fill piece is kept only if it touches two items of its net, or one and is at least
`min_island_area`; the rest reach nothing new and are removed. Check verifies every fill against
other nets: a fill that overlaps another net's zone, track, via or pad, or comes closer than the
clearance, is an error, and copper tips sharper than 30 degrees are flagged.

Zones on the same layer fill in order of `priority`, then smallest first, and each keeps its
clearance from the fills already placed, so a small switch-node or supply pour inside a
board-wide ground pour is poured around rather than shorted to it.

Pours keep `min_via_hole_to_copper` from other nets' via holes and `min_npth_to_copper` from
non-plated holes as well as the clearance, with arcs drawn outside the true circle so the gap is
never short. Stitching vias go only where the via clears every other net's copper on each layer it spans,
keeps its hole `min_via_hole_to_copper` from that copper and `min_hole_to_smd_pad` from SMD pads of any net,
keeps `min_hole_to_hole` from every drill and the edge rule from the outline, and lands inside a
zone of its net; check reports how many it placed. They are drilled and plotted like any via.

Zone fills are exact polygons: the zone outline less every other net's copper grown by its
clearance, with round corners, so pours render and plot without stair steps. Necks and slivers
narrower than the zone's `min_width` (default 0.25 mm) are removed, the way a fab would etch them.
Where two clearance areas (antipads, track and pad clearances) come closer than `min_width`, the
pour is cut back to the straight lines joining them, within about 1.5 `min_width` of the gap, so no
stub or hairline waist is left pointing into it. The same holds between a clearance area and the
board edge's clearance, a cutout, or the clearance around another zone's fill.

Zone fills are cached: each fill is keyed by a hash of everything it depends on and kept in
`~/.cache/agentee/fills`, so a load only fills the zones whose copper, clearances or neighbours
changed (set `AGENTEE_NO_FILL_CACHE` to skip the cache). `agentee fill NAME` (MCP `fill`) also
stores the fills at the end of the layout file, one `[[fills]]` table per zone and layer, so a
fresh checkout loads without filling; a stored fill whose hash no longer matches is ignored, and
check notes it with `--info`. Importing a board stores its fills. Leave the `[[fills]]` tables to
the tool.

A track that only grazes a pad (its centre line misses the pad) is flagged; run it into the pad. Two
segments of one net that lie on top of each other on a layer (parallel, overlapping by more
than a track width) are an error, since the copper is doubled; a bend sharper than 90 degrees is
flagged as an acid trap.
A track may neck down below its class width, to no less than the fab minimum, for up to 0.5 mm
(the class `neckdown`) where it meets a small pad; the router draws such necks itself (see
Autorouting). Drilled holes, vias and plated pads alike, must
keep the board's `min_hole_to_hole` apart; check counts the pairs that do not and names the first.
Two vias of one net at the same spot are an error too: the fab would drill the hole twice.
Mask openings are the pad outlines, with no expansion, and vias are tented. Two openings of
different nets (or no net) that overlap or leave a mask web under `min_mask_web` are an error,
counted per part pair with the first place named. Pads of one fine pitch part are checked too: fix
it in the footprint with narrower pads, or set a smaller `min_mask_web` when the fab allows it. A
footprint with `mask_web = false` has its mask opened as one window over its pads (a gang
opening), so pairs of its own pads are skipped; its pads are still checked against other parts.

Artwork on a bottom layer is mirrored so it reads correctly from below. SVG fills and strokes are
flattened to polygons; text in an SVG is ignored, so convert it to paths first. Silk text and
artwork get the same checks as reference labels: overlap, pads, silk outlines, board edge.

### Placement

`agentee place NAME [--parts 'U*,C1?'] [--keep-placed] [--side F|B|both] [--seed N] [--dry-run]`
(MCP `place`) places the parts of the layout's schematic inside the board outline and writes each
one's `at`, `rotation` and `side` into its `[[footprints]]` entry (adding entries for new parts,
dropping a moved label's `at`), moves the reference labels that now fail a silk check, then
refreshes the stored fills. It is a quick start for a whole
board: place, look at it, then move what matters by hand and lock it. `--parts` places only the
matching parts and leaves the rest where they are; `--keep-placed` leaves every part that already
has a placement; `locked = true` on a `[[footprints]]` entry is never moved. `--side both` lets
decoupling caps of a BGA go under it on the back and passives spill to the back when the top is
full; the default puts everything on the top. The result depends only on the files and `--seed`,
so a different seed is a different start. It reports the half-perimeter wirelength (HPWL) of the
signal nets and of every net, ratsnest crossings, courtyard overlaps and the mean and worst
decoupling distance, for the placement it started from (when every part had one) and for its own,
plus the clusters it built and the edge each connector went to. Tracks and vias already in the
file stay where they are; place an unrouted board, or `route --reroute` after. The labels are
moved with the same search as `agentee silk` (a clear spot beside the part, up to 8 passes and 3
tries a label), run on the placed layout in memory, so the project is loaded once more, not once
a pass; `labels_moved` counts them and `labels_failing` names the ones left without a clear spot,
for `agentee silk NAME --hide` or a hand move.

What goes where, strongest first, and why:

- Connectors (parts with `edge = true` pads, `overhang = true`, or a `J`/`P` reference) go on the
  board edges, each edge spread evenly, with the mating side out: the edge line is the
  footprint's `Dwgs.User` line next to a text saying `edge` (KiCad's `PCB Edge` mark), else the
  outer side of its `edge` pads, else its pads plus `min_part_to_edge` for an overhanging part,
  else its courtyard plus `min_body_to_edge`. An RF connector (SMA, SMB, BNC, U.FL, MMCX, coax)
  and a USB connector never share an edge unless no other split fits, since USB 3 noise lands in
  the 2.4 GHz band (Intel, "USB 3.0 Radio Frequency Interference Impact on 2.4 GHz Wireless
  Devices", 2012). `[place] edges` pins a connector to an edge. Mounting holes go in the corners
  (diagonal first), fiducials in the free corners with their pads 3 mm or more from the edge,
  clear of the conveyor rails and clamps (SMEMA Fiducial Mark Standard 3.1).
- An RF path is laid in a straight line. Starting at each connector with a pad on an RF net (a
  class with `impedance` and no `diff_gap`), the placer follows RF nets through the parts that
  carry two or more of them and have at most 16 pins (DC blocks, attenuators, switches, LNAs,
  baluns), and takes the longest path that ends at another connector or at a larger chip with
  RF pins; a switch with a bypass goes through the amplifier, not the bypass. When both ends
  are connectors they go on opposite edges of the board's long axis and slide along them onto
  one line, through the board centre where both fit. Each part on the path turns so its input
  and output pads sit on that line, in order, packed toward the input with at most 1.5 mm of
  extra gap each, and the rest of the room goes to the last run. A two or three pin part with
  one pad on a node of the path (an ESD clamp, a bias choke, a shunt cap) sits beside the line at
  that node, turned so its other pads point away, close enough that its RF pad meets the track and
  far enough that its other pads keep the class clearance; two shunts at one node take opposite
  sides. The path is fixed before the rest is placed around it, and a part that would leave the
  board or land on another is left to the general placer.
- Large chips (BGA, 16+ pin packages over 25 mm2, else the highest pin count part) are pulled to
  the board centre and kept within `off_centre` of it, where they have room to escape their pins on every side (Xilinx UG1099,
  Recommended Design Rules and Strategies for BGA Devices) and away from the edges and
  mounting holes, where handling and depaneling bend the board and crack BGA joints
  (IPC/JEDEC-9704A strain guidelines).
- Clusters come from the netlist: each IC takes the passives whose signal nets reach it alone,
  the capacitors between a supply it uses and ground (the nearest IC in the schematic when
  several share the supply, spread over its supply pins), and its crystal. A cluster's decoupling
  caps sit next to the supply pin they serve (Analog Devices MT-101, Decoupling Techniques; TI
  SCAA082, High-Speed Layout Guidelines), or under it on the back with `--side both` (Xilinx UG483,
  7 Series PCB Design Guide, puts the small caps under the BGA); the crystal next to its clock
  pins (ST AN2867, Oscillator design guide). Clusters are placed to minimise weighted HPWL:
  nets of an impedance or pair class, `[[pairs]]` and `[[interfaces]]` count three times, supply
  and ground nets not at all (they are planes).
- A switcher (an IC with an inductor on one of its pins whose other end is a supply) keeps its
  inductor and input caps tight against it, and switchers keep 8 mm (centre to centre) from
  sensitive parts:
  crystals and parts on single-ended impedance (RF) nets (TI SNVA021, AN-1149 Layout Guidelines
  for Switching Power Supplies; Linear Technology AN139, Power Supply Layout and EMI).
- Hot parts (sources of 0.25 W or more in a thermal sim of this layout, and large packages over
  49 mm2) are pushed 10 mm apart so their heat does not stack (TI SNVA419, AN-2020 Thermal Design
  by Insight, not Hindsight; IPC-2221B, thermal management). Where the hot parts, each grown by
  `hot_distance`, take at most a quarter of one side of the board, their courtyards must also
  keep `hot_distance` apart, as `placement-hot-parts-close` measures it (a part that fits nowhere
  else drops the rule); on a tighter board wirelength wins. `hot_spread` in the output gives that
  share and whether the rule applied.
- Ceramic capacitors of 0805 or larger stay out of `flex_zone`, and smaller ones inside it lie
  along the edge, corner or mounting hole they are nearest (Murata and TDK MLCC mounting guidance
  on board flexure; Knowles). The zone is measured as the `mlcc-flex-zone` checks measure it,
  from the two pads to the outline, the board cutouts and the mounting hole drills; a last pass
  turns or moves (up to 12 mm) any capacitor still in the wrong spot. Every body keeps
  `min_body_to_edge` from the outline and from each `[[outline.cutouts]]` hole of the board file,
  which is off the board, courtyards never overlap on a side (through-hole parts block both
  sides), and nothing enters a `[place] keepouts` polygon (IPC-7351B courtyards: the courtyard is
  the least area a part and its land pattern need).
- Rotation follows the main nets: each part takes the quarter turn that brings its pads nearest
  the other ends of its nets, and the passives of a cluster share one axis.

The method: each cluster (an IC and its members) and each loose part becomes a block with the
area of its parts. A spectral layout of the weighted clique graph of the blocks (its two lowest
non-trivial eigenvectors, by rank) orders them on the board. Then 24 rounds alternate a quadratic
wirelength solve and a spreading step: the solve pulls pins, not centres, for chips, and picks
each chip's quarter turn every round (by wirelength, and in the later rounds by the crossings of
its two-pin nets too); connectors are pulled onto their nearest edge and large chips to the
centre; the spreading step bisects the free board area (inside the outline and clear of cutouts,
keepouts and placed parts) in turn along its longer side, giving each block a region the size of
its area, and each round pulls the blocks harder to their regions. Connectors are then assigned
to edges (every assignment tried up to seven connectors, greedy past that) and slid along them,
flush with the outline where it runs at that point (a notch or a step counts as edge), trying
the other edges nearest first when no spot on the assigned one is free,
and the solve runs again with them fixed. Legalisation places each cluster as a whole: its anchor
at each quarter turn (for chips) and five nudges of a third of the cluster's width, its members
around it, keeping the cheapest by wirelength and crossings; each part goes to the nearest spot on
a 0.05 mm grid where its courtyards clear everything and it keeps the edge rules. Whole clusters
are then placed again turned or nudged, and pairs of chip clusters of a similar size swapped,
where that lowers the cost. A simulated annealing refinement follows (shifts, quarter turns,
swaps of same-size parts, whole cluster moves, side flips with `--side both`; 1500 moves a part,
then 500 more at a low temperature), scored by weighted HPWL, crossings of two-pin nets (4 per
crossing), and the rules above as penalties. Eight starts (the eight reflections of the spectral
layout) are legalised on parallel threads, and the three cheapest go on to the cluster moves and
the annealing, so the result depends on the files and `--seed` only, not on the number of threads.

Parts keep off the board's own silk: `[[graphics]]` lines on a silk layer, silk text with
`locked = true` and silk `[[artwork]]` block the courtyards on their side (a part that fits
nowhere else may still cover them). Silk text that is not locked does not block parts: after
placing, each such text a part (or its reserved label) covers moves to the nearest clear spot on
the board within 10 mm, clear of parts, labels, locked silk and the other texts; `texts_moved`
lists the moves written back into `[[graphics]]` and `texts_stuck` the texts with no clear spot.
Connectors, mounting holes and fiducials reserve a label too, on the outer side of the part or
turned to the inner side, and go without one where neither fits at the spot the part takes. Each part's reference label needs room too: where the labels take at most a quarter of the
free board area, a box the size of the reference text (0.2 mm around it, at the footprint's
reference spot pushed clear of its own pads and silk) is kept clear like a courtyard, and place
writes that spot as `label.at`; on a fuller board with room left the overlap of those boxes with
other parts and labels is a small cost in the annealing instead, and on a full board labels are
left to the label pass (`agentee silk --hide` for the ones with no spot).

On a copy of `examples/sdr` with the tracks and vias removed (251 parts, 65 x 45 mm), top only,
seeds 1 to 4 give 3276 to 3436 mm of signal HPWL and 261 to 293 crossings, in about 2 s of solve
on 4 cores (about 6 s of CPU) plus loading and the label pass. With `--side both` they give about
1955 to 2025 mm and 150 to 165 crossings.

### Autorouting

`agentee route NAME --nets 'FX_D*,SPI_*' --layers F.Cu,In2.Cu,B.Cu` (MCP `route`) routes the
ratsnest of the named nets on a grid (`--grid`, default 0.05 mm) and appends the tracks and vias
to the layout file as ordinary `[[tracks]]` and `[[vias]]`, so they are yours to edit afterwards.
It keeps each net class's width, clearance and `layers` against every pad, track, via, hole and
the board edge, sizes the room for each class via by its own pad, drill and layers (a microvia fits
where a through via does not, and a microvia may sit beside a buried via on other layers), keeps new
vias `min_hole_to_hole` from every drill whose span shares a dielectric with theirs and their holes
`min_via_hole_to_copper` from other nets' pads, tracks and vias, keeps them off SMD pads of every
net, its own too (the via copper may not touch one, `via-cuts-pad`, and the hole stays
`min_hole_to_smd_pad` from it, `hole-to-smd-pad`, unless `--via-in-pad`, MCP `via_in_pad`, lets it sit
wholly inside an SMD pad of its net with a drill of at most `max_filled_via_drill`), uses the class vias to change
layer, the cheapest whose span holds both layers (`--via uv-top,core` or MCP `via` to choose from other
names instead, `--via-cost` in mm of
track, default 3, times the via's `cost`), charges `--bend-cost` mm of track for
each 45 degree bend (default 0.1, three times that for 90), and never moves what is already there
unless `--reroute` is given, which deletes the named nets' tracks and vias first. A connection that finds no free path rips up the routed nets
it would cross or whose via holes it would bring within `min_hole_to_hole` of its own (holes whose
spans share a dielectric), remembers the spot as congested, and those nets go back in the queue. The search
steps in 45 degree directions and charges for every bend, so paths come out as straight runs with
45 degree bends; afterwards runs are pulled tight with two-segment 45 degree doglegs and any 90
degree corner left is chamfered where it clears. Only the stub into an off-grid pad centre may sit
at another angle. Where the class width does not fit into an end pad (wider than the pad's smaller
side, or too close to the neighbouring pads to keep clearance), the route necks down: the class
width track stops short of the pad and a separate `[[tracks]]` entry with an explicit `width` runs
straight into the pad centre, at most the class `neckdown` long and no narrower than
`min_track_width`. Its width is the smallest of the class width, the pad's smaller side and the
widest that keeps clearance, rounded down to 0.01 mm (or exactly `min_track_width` where rounding
would drop under it and the unrounded width does not); the wide track starts at the first spot out
from the pad where its full width keeps clearance (and, for a pad narrower than the track, outside
the pad). Pads of one net that touch, like a thermal pad built from several pad entries, count as
one wide pad. Pairs routed with `--pairs` do not neck down. A connection of a net that already has fresh copper starts from that copper. Once everything is in,
each routed connection that uses vias is tried again on one layer at a time with the rest held
fixed, and the one-layer route replaces it when it is at most 25% plus 1 mm longer. Then the
vias of neighbouring parallel connections that change layer near each other are slid along their
own tracks onto a common row or column on the grid, where the spot is legal and the connection
keeps its length (pair legs held by `--pairs` and nets with several connections stay put). A net in an
interface with `max_vias` keeps the trace (every net of the lane, through series parts) within
it: a route with too many vias is tried again with dearer vias, then on one layer, and fails with
the reason if neither fits. The second net of a pair is drawn toward its
partner at the pair gap; `--pairs` tries to route both halves together as one coupled track
first. When a few connections fail, route them again together with the nets around them and
`--reroute`, so the router can rip up and reorder the whole area, or drop to `--grid 0.025`. `--dry-run` reports without writing. Route the nets that matter by hand
first, then let the router fill in the rest, a class at a time.

A net with a `[[zones]]` entry is a plane net. A glob in `--nets` never matches it, so
`--nets '*'` routes every signal and supply that has no pour and leaves ground to the planes;
name the net exactly to route it with tracks anyway. Vias placed by `[[stitching]]` rules are not
obstacles to the router: the rule places them again around the new copper.

`agentee tie NAME [--nets GND] [--dry-run]` (MCP `tie`) connects each SMD pad of a plane net to
the nearest layer its zone covers with a short stub and a via beside the pad, which is how ground
and supply pads join a plane instead of tracks between them. The via goes on the side of the pad
away from the part's centre, as close as it can: its copper clear of every SMD pad (its own too,
so no via-in-pad), its hole `min_hole_to_smd_pad` from them, `min_hole_to_hole` from other holes,
`min_via_hole_to_copper` and the class clearance from other nets, `min_copper_to_edge` in from
the outline, and the stub clear of other nets on the pad's layer. It tries 16 directions, nearest
the outward one first, up to 1.2 mm past the pad. A pad that already has a via of its net within
0.8 mm of its edge is left alone, through-hole pads and BGA balls (`escape` handles those) are
skipped, and a pad with no legal spot is listed under `failed`. The stubs and vias are appended
as ordinary `[[tracks]]` and `[[vias]]`.

`agentee neck NAME [--nets 'VBUS,RF_*'] [--taper] [--dry-run]` (MCP `neck`) necks down copper that
is already there. It looks at both ends of every track of the named nets (default all) that end
inside a pad of their net on their layer, and takes the ones that are wider than the pad's smaller
side, or whose full width breaks clearance within the class `neckdown` length (plus one track
width) of the pad. Such an end is cut: the track keeps its width from the cut outward and a new
`[[tracks]]` entry with an explicit `width` runs from the old end point in the pad to the cut,
along the old path. The neck width is the smallest of the track width, the pad's smaller side and
the widest that keeps clearance along the neck, rounded down to 0.01 mm (or exactly
`min_track_width` as above). The cut is the first spot, in 0.01 mm steps, where the full width
keeps clearance for the next `neckdown` plus one track width (and, for a pad narrower than the
track, lies outside the pad); a neck is never longer than `neckdown` and never under
`min_track_width`, otherwise the end is listed under `failed` with the reason and left alone. A
pad built from several same-net pad entries that touch counts as one wide pad. `--taper` steps
the width back up over a short chain instead of one neck: past the narrow neck come up to two
more entries, each a third of the way from the neck width to the track width, together no longer
than `neckdown` (three times the neck, at most). The copper only ever gets narrower, so no new
clearance error can appear; the file is edited in place (every other line is kept) and stored
zone fills are refreshed. The report lists each neck (track index, net, pad, why, width, length,
entries) and counts the tracks changed and added.

Pairs and length rules live in the layout too:

```toml
[[pairs]]                      # optional: nets ending _P/_N, _DP/_DN, +/-, P/N in a class
p = "USB_DP"                   # with diff_gap are paired on their own
n = "USB_DN"
max_skew = "0.1mm"             # or `max_skew` on the net class

[[match_groups]]
name = "ddr-dq0"
nets = ["DQ?", "DQS0_*"]       # * and ? globs
tolerance = "0.5mm"
# target = "42mm"              # default the longest member
```

Check reports each pair's skew in mm and ps (from the layer's effective permittivity), a stretch
of the pair run at the wrong gap, and pairs that spend less than 80% of their length side by side.
Match groups say which net is short or over and by how much. A pair that runs through series
two-pin parts, like the AC caps on a USB lane, is measured end to end: its skew is the sum over
every pair it joins, reported as `FX_TX1_P+SS_TX1_P/FX_TX1_N+SS_TX1_N`.

### Interfaces

An interface says what a link must meet, and check measures the copper against it: impedance,
skew, length, vias, via stubs, the reference plane under every track, the return vias, bus
timing, and the S-parameters and eye of the sims that model it.

```toml
[[interfaces]]
name = "usb-ss"
preset = "usb3-gen2"           # usb3-gen1, usb3-gen2, usb2-hs, lvds, rf-50, cmos
nets = ["SS_TX*", "SS_RX*"]    # globs; differential presets pair them up, through series parts
# differential = true
# impedance = "90ohm"          # the member classes' targets must sit inside this
# impedance_tolerance = "7%"
# max_skew = "5mil"            # within each pair, a length or a time ("1ps")
# max_length = "3in"           # end to end, through series parts
# max_vias = 2                 # per trace
# max_stub = "15mil"           # the via barrel past the last layer the trace uses
# max_unreferenced = "0.5mm"   # total run with no plane of `reference` under it
# reference = ["GND"]
# return_via = "200mil"        # a reference via this close to every signal via

[[interfaces]]
name = "ad-rx"
preset = "cmos"
nets = ["AD_P1_D*", "AD_RX_FRAME", "AD_DATA_CLK"]
clock = "AD_DATA_CLK"          # a pair's clock is named by either leg
max_bus_skew = "30ps"          # spread of arrival across the data lines
clock_window = ["-50ps", "150ps"]   # data minus clock arrival, from the setup and hold budget

[[interfaces.measure]]         # read from a finished sim; missing or stale results are errors
sim = "sdr-usb"                # an FDTD run
pair = ["TX1_FX+", "TX1_FX-", "TX1_J+", "TX1_J-"]   # IN+, IN-, OUT+, OUT-; or through = [IN, OUT]
up_to = "5GHz"                 # Nyquist, the band the limits apply over
max_loss = 1.5                 # dB, worst Sdd21 (or S21) in the band
min_return_loss = 10.0         # dB, worst Sdd11
max_mode_conversion = -30.0    # dB, worst Scd21

[[interfaces.measure]]
sim = "sdr-usb-eye"            # a channel sim
min_eye_height = "70mV"
min_eye_width = "0.47UI"       # or ps

[[interfaces.measure]]
sim = "i2c-bus"                # a logic sim: no limits, it passes when the sim passes
```

A measure on a logic sim fails while the sim has any failure (an assertion, contention, a
timing violation or a stopped run), naming the count and the first; it is stale when the sim's
spec or schematic netlist changed, as the sim itself reports, not when the copper did, since a
logic sim never sees the layout.

| preset | impedance | skew | vias | stub | other |
|---|---|---|---|---|---|
| `usb3-gen1`, `usb3-gen2` | 90 ohm +/-7% diff | 5 mil | 2 | 15 mil | 3500 / 3000 mil long, GND return via within 200 mil, 0.5 mm unreferenced |
| `usb2-hs` | 90 ohm +/-10% diff | 50 mil | 4 | | 12000 mil long, 1 mm unreferenced |
| `lvds` | 100 ohm +/-10% diff | | | | 1 mm unreferenced |
| `rf-50` | 50 ohm +/-10% | | 0 | | 0.5 mm unreferenced |
| `cmos` | | | | | 2 mm unreferenced |

The USB numbers are TI's High-Speed Interface Layout Guidelines (SPRAAR7J, Appendix A), the Gen 2
length is congatec AN37. Anything a preset sets, the interface can override.

Delays are the track delay from each layer's effective permittivity plus the via barrel the
signal crosses. A plane counts as the reference where the first copper found walking away from
the track, past cut-out layers, is a zone of a `reference` net that covers the track's full
width; the antipad around the net's own vias does not count. Every sim result records a hash of the copper it saw (the sim's `region` only, plus the
stackup); a result whose copper has changed since is stale, and a measure on it fails.

`agentee tune NAME` (MCP `tune`) fixes these: for every pair over its skew limit and every match
group member short of its target it meanders the short side, on its longest straight segments,
anywhere along a series chain, with bumps that keep every other net's clearance and the board
edge rule, and writes the new points into the tracks. `--nets` limits it, `--amplitude` caps the
bump height and `--pitch` fixes the bump pitch (default three track widths, tighter where that is
all that fits). On one leg of a pair it bumps the stretches where the legs already run apart
first (breakouts and bends, where the mismatch comes from) and never leaves the leg running beside
its partner off the class gap; a coupled stretch only takes bumps tall enough to clear the pair.
Interfaces count too, in time as well as length: a pair over
its `max_skew` in ps gets the short side lengthened by that delay; for `max_bus_skew` and
`clock_window` the clock is lengthened until the latest data line falls inside its window and
every data line short of the bus spread or the window's early edge is lengthened to the latest
one (inside the window), both legs of a pair together. Delays turn into millimetres at each net's
own ps per mm. When one leg of a pair cannot take all of it, the other leg is held to what it got.
A net that is over its group target is reported, not shortened. `agentee calc
serpentine --from x,y --to x,y --add 2.5mm` (MCP `serpentine`) returns the points of one such
meander on a segment you pick. Net lengths and delays are in `agentee show pcb:NAME`.

The viewer's layout page has a `3d` tab (`agentee view --3d` opens on it): the board in its
stackup thickness, mask and silk colours and finish, copper under the mask, bare pads, plated and
bare drill walls, and each part's 3D model. The `parts` toggle hides the models. Drag to orbit,
shift-drag to pan, scroll to zoom, double-click to reset. The viewer draws with OpenGL
([three-d](https://github.com/asny/three-d)); `agentee render pcb:NAME --show 3d` (or `3d-top`,
`3d-bottom`) draws the same scene in software, `--hide parts` leaves the models out and `--region`
aims the camera at that area.

### Editing in the viewer

The 2D layout page of `agentee view` edits the layout by hand. Edits stay in the window until
`save` (ctrl+S) writes them into the `.pcb.toml` through the same comment and order preserving
writer the CLI uses; `revert` drops them. Every edit is checked in the background the way `check`
would, so the diagnostics panel and the net table follow along, and the header counts unsaved
layouts. When the file changes on disk under unsaved edits the page asks whether to keep yours or
load the file. Closing the window with unsaved edits asks first.

| key | does |
|---|---|
| click | select a part, track, via or ratsnest line; clicking the selection again picks the next item under the pointer |
| drag | move it: a part with its label, a via, a track corner, or a whole track segment; the selected item wins over what lies under it |
| right or middle drag, wheel | pan, zoom |
| R, shift+R | rotate the selected part by 90 degrees |
| Del | delete the selected track or via |
| X | route tool: click a pad, via or track to start, click to add 45 degree corners, `/` flips the bend, V drops a via and moves to the next layer the net's class vias reach, Backspace steps back, Enter, a double click or a click on copper of the net ends |
| V | via tool: click copper with a net to place a class via of that net |
| PgUp, PgDn | active layer |
| ctrl+Z, ctrl+shift+Z | undo, redo |
| Esc | cancel the track, back to select, clear the selection |

Moves snap to the grid picked in the toolbar. New tracks take the class width (no `width` key);
the properties panel sets the net, layer and an own width of a track, the position, net and type
of a via, and the position, rotation, side and `locked` of a part. A selected ratsnest line can be
handed to the autorouter as one connection or as its whole net, with the same rules as
`agentee route`. A via that a `[[fanouts]]` or `[[stitching]]` rule placed, or one element of a
`count` row, is taken out of its rule when you move, change or delete it: the rule gets a
`skip_at` entry for the spot and the via is written as its own `[[vias]]`.

`reset` in the toolbar clears what you tick: routing (tracks, vias and the engine's plan
sections), the `[[fanouts]]` and `[[stitching]]` rules, zones with their stored fills, and silk
label positions, and can place every unlocked part again with the placer (`agentee place`, with a
seed). Like every edit it stays unsaved until you save. `new layout` on the layouts tab asks for a
name, board and schematic, writes `NAME.pcb.toml` next to the project and, unless you untick it,
places every part with the placer first. The new file starts with a zone on every copper layer
for each ground net of the schematic and, when the schematic has RF nets, a `[[stitching]]` grid
of ground vias at 2.5 mm plus a fence along the RF tracks when their class has a
`coplanar_gap`. The engine card's `tie plane pads` runs `agentee tie` on the window's layout, and
a selected ratsnest line of a plane net offers `tie pads to the plane` before a route.

The `layout engine` card runs the phases of `agentee layout` (see `docs/layout-engine.md`) on the
layout as it stands in the window, unsaved edits included: `run` takes the phases from..to,
`run X only` reruns one, `stop` ends the run after the phase that is running. Editing pauses while
it runs. The result lands in the window like any other edit, unsaved and one undo away, with the
time and score of each phase and the score terms that weigh most. `phases to run` and `settings`
write `[engine]` into the layout (`phases`, `tile`, `rounds`, `[engine.place] seed`,
`[engine.escape]`, `[engine.global] via_cost`, `[engine.detail]`), and `default` drops a setting
again. A selected part with 16 or more pads has `pinswap`, which searches swaps of its I/O like
`agentee pinswap` and writes them into the schematic files on `write to the schematic`.

Common parts are drawn from the footprint without loading a model: chip resistors, capacitors,
inductors, LEDs and diodes (names with a `Metric` size), vertical pin headers and sockets, SOIC,
SOT, QFP, QFN, DFN, SON, TSLP and BGA packages, crystals and oscillators, edge mount SMA
connectors, and shield frames (`Shield` in the name or description), drawn without a cover. The body is the `F.Fab` outline, the height comes from
`height`, an `_h1.25mm` part of the name, or the package family, and leads sit on the pads outside
the body. Mounting holes, fiducials, pad test points, solder jumpers and Tag-Connect footprints
get no body. A model file in the project (`3dmodels/` or a path relative to it) still wins over a
generated part; `agentee models` skips the generated ones.

Models are STEP or VRML, placed the way KiCad places them (`model_offset`, `model_rotate`,
`model_scale`). A model path is looked up as given, then under `KICAD9_3DMODEL_DIR` and friends,
`3dmodels/` in the project, `~/.cache/agentee/3dmodels` and `/usr/share/kicad/3dmodels`, trying
`.step`, `.stp` and `.wrl`. KiCad library models that are not on disk are downloaded from the
kicad-packages3D repository into the cache: `agentee models` (MCP `models`) fetches them all and
reports what it found, and the viewer fetches in the background. A part without a model is a box
over its fab outline, `height` tall, else a height guessed from the footprint name. STEP files are
meshed with [truck](https://github.com/ricosjp/truck), with colours from their styled items and
assembly placements applied. Edges use the 3D curve of each surface curve, not its pcurves.
Unclamped B-spline curves and surfaces are cut to their valid knot range. An edge curve that
still does not evaluate to finite points is meshed as a straight line, and a face whose surface
does not is left out.

Pads take their nets from the schematic (pad number = pin number). Zones are filled with the
clearance to every other net and to the board edge, and islands that reach nothing are removed.
Check reports unrouted connections (with the ratsnest), shorts, clearance violations, tracks
narrower than their class or off their impedance width, copper near the edge, courtyard
overlaps, unplaced parts, track ends that connect to nothing, and silk text that overlaps other
text, crosses a silk outline, sits on a pad or runs off the board. Courtyards are the closed
outlines drawn on `F.CrtYd` / `B.CrtYd`, placed with the part (a bottom part's land on the other
side); two parts whose courtyards overlap on the same side are an error, and so is a courtyard on
either side over another part's NPTH hole or over a `MountingHole*` footprint's courtyard (its pad
outline when it has none). Name an item with its kind when
names collide: `agentee render pcb:lna`, `sch:lna`, `board:lna`.

## Simulation (`*.sim.toml`)

A full-wave FDTD run of a layout on the GPU (wgpu), giving S-parameters between ports placed on
pads. `agentee sim NAME` runs it and writes `NAME.result.json` and a Touchstone `NAME.sNp` next
to the spec; the viewer and `render` then show the magnitude plot and a Smith chart.

```toml
name = "lna-rf"
layout = "lna"
cell = 0.05                    # finest mesh cell in mm, default 0.05; the mesh grades out from copper edges
excite = ["IN", "AMP_OUT"]     # ports to drive, one run each; default all
# region = [0, 0, 30, 20]      # crop to x0, y0, x1, y1 in mm, default the board
# max_steps = 150000
# end_db = 50                  # stop once the field energy is this far under its peak, as openEMS does

[frequency]
start = "100MHz"
stop = "4GHz"
points = 391

[[ports]]
name = "IN"
pad = "J1.1"                   # a lumped port over the whole pad, down to the next copper layer
reference = "In2.Cu"           # or name the layer it returns on, e.g. past a cutout
# impedance = 50

[[models]]                     # C, L and R parts take their schematic value by default
ref = "D3"
capacitor = "0.23pF"           # or inductor = "47nH", resistor = "1k", open = true
```

Copper is modelled as zero-thickness sheets on each copper layer (pads, tracks, vias as plated
columns, zone fills with their clearances), dielectrics from the board stackup with their loss
tangent, and CPML boundaries. Parts other than ports and lumped models are open. Validated on a
50 ohm microstrip: return loss under -25 dB, insertion loss under 0.2 dB, and phase velocity within
1.2% of Kirschning-Jansen.

### Channel (`kind = "channel"`)

Pushes a bit stream through a path of an FDTD or cascade result and draws the eye.

```toml
name = "usb-eye"
kind = "channel"
board = "usb-link"             # the sim with the S-parameters
through = ["J1", "J2"]         # single ended FROM, TO
# pair = ["J1+", "J1-", "J2+", "J2-"]   # or differential, uses Sdd21
bit_rate = "5Gbps"
rise = "40ps"                  # 10-90% edge, default 0.35 UI
swing = "0.8V"                 # peak to peak
prbs = 7                       # 7, 9, 11 or 15
ctle = { dc_gain = -6.0, zero = "1GHz", poles = ["5GHz", "10GHz"] }
dfe_taps = 2                   # ideal decision feedback on the first post cursors
```

Real drivers and receivers come from their IBIS files:

```toml
tx = { file = "models/sn74lvc1g04.ibs", model = "LVC1G04_OUT_33", component = "LVC1G04_DBV", pin = "4" }
rx = { file = "models/sn74lvc1g04.ibs", model = "LVC1G04_IN_33", component = "LVC1G04_DBV", pin = "2" }
```

The driver becomes a Thevenin source: the small-signal output resistance from the [Pulldown] and
[Pullup] tables near their rails, C_comp at the die, then the pin's (or the component's) package
R, L and C; its [Ramp] sets the edge (20-80% converted to 10-90%) and [Voltage Range] the swing,
unless `rise` or `swing` are given. The receiver loads the far end with its package and C_comp,
and the eye is read at its die. The path is solved with the full two-port, so a high-impedance
receiver sees the line's reflections. On TI's SN74LVC1G04 model the ramp and the V-I tables agree
on the swing into 50 ohm to 2.5%. IBIS works on single-ended paths for now.

The pulse response comes from the path's S21 (or Sdd21), times the CTLE, with a Gaussian edge,
and the eye is the PRBS superposed on it at 64 phases per unit interval. Readings: eye height at
the best phase, eye width, the peak-distortion worst case (main cursor less every other cursor),
loss at Nyquist and the cursor count. An edge faster than the S-parameters reach is slowed to
1.3 / the top frequency, with a note. On an RC channel the worst case lands within 1.2% of
1 - 2e^(-T/tau).

### PDN (`kind = "pdn"`)

The impedance of a power rail, from ports the FDTD sim puts on the rail's power pins, decap pads
and the VRM. Every port of the board sim must be driven and must be a sink, a decap or the VRM.
Ports should carry a low `impedance` (0.5 to 1 ohm), since the rail's impedance sits far below 50
ohm and the conversion from S11 loses precision at a 50 ohm reference. Below the sim's own band
the board is extrapolated as L plus the DC resistance of its S matrix at the first point.

```toml
name = "vcc-pdn"
kind = "pdn"
board = "rail"                 # the FDTD sim with the rail's ports
sinks = ["U1_VCC"]             # the load pins, left open like current sources
target = { voltage = "5V", ripple = 1.0, transient = "50mA" }   # or impedance = "20mohm"

[[decaps]]
port = "C3"
ref = "C3"
file = "models/grm155r61a105ke15.s2p"   # a measured one-port, series or shunt fixture
mount = "series"

[[decaps]]
port = "C4"
c = "1uF"
esl = "0.4nH"
esr = "15mohm"

[[vrm]]
port = "VRM"
r = "2mohm"
l = "20nH"

[frequency]                    # the sweep to report over
start = "100kHz"
stop = "1GHz"
points = 200
```

The viewer draws |Z| per sink against the target on a log-log plot. Readings give the peak
impedance and how it compares with the target, plus the largest anti-resonances (which is where
to move a decap or change its value). On an ideal junction with one VRM and one decap the result
matches Z_VRM || Z_decap to machine precision.

### Analysing a result

`agentee sparam NAME` (MCP `sparam`) works on any FDTD or cascade result:

```
agentee sparam lna-rf --tdr IN --rise 50ps     # impedance against time from one port
agentee sparam link --pair 1,2,3,4             # Sdd21, Sdd11, Scc21, Scd21, Sdc21
agentee sparam link --xtalk 1,3                # coupling in dB and the step crosstalk
```

It always reports passivity (the largest singular value of S over the band) and reciprocity
(the largest |Sij - Sji|) when every port was driven. The TDR resamples onto a uniform grid,
extrapolates the real part to DC, applies a Gaussian edge (default 10-90% rise of 1.3 / the top
frequency) and integrates the impulse response; it warns when the edge is faster than the data
supports. The viewer shows the same TDR for every driven port under the `tdr` tab.

### Losses in the FDTD

Dielectrics follow the same Djordjevic-Sarkar model as the 2D solver (their `er` and
`loss_tangent` taken at 1 GHz), as a set of Debye poles log-spaced from a thirtieth of the lowest
frequency to thirty times the highest, one polarisation current per pole on each dielectric edge
(within 3% of the model's loss tangent and 0.01 of its er across the band). The loss therefore
grows with frequency the way it should, and on a 50 ohm microstrip it lands within 4% of the
Hammerstad filling-factor formula at 1.5, 2.2 and 3 GHz. Edges that carry a port, a lumped part
or copper keep a conductivity fixed at the band centre instead. Copper layers are sheets with a
surface impedance that varies with frequency, so both the resistance and the internal inductance
follow the skin effect, the roughness and the finish across the band, updated implicitly so the
sheet stays stable at any time step. A zero-thickness sheet has one current, while real copper
carries it on two faces, so each sheet edge tracks the magnetic field just above and below it
through the run and takes its impedance as (Jtop^2 Ztop + Jbottom^2 Zbottom) / (Jtop + Jbottom)^2:
one half of the face impedance for a centred stripline, close to the bottom face's for a trace
over a plane.

Each face has its own surface impedance, a function of frequency:

- A copper face is a copper slab of the layer's thickness, sqrt(j w mu rho) coth(gamma t), which
  settles to rho / t at DC, times the stackup roughness as a causal complex factor (Dmitriev-Zdorov,
  Simonovich and Kochikov, "A Causal Conductor Roughness Model and its Effect on Transmission
  Line Characteristics", DesignCon 2018, table 1). Its loss part is exactly the Hammerstad-Jensen
  1 + (2/pi) atan(1.4 (rms / skin depth)^2) or the Huray 1 + 1.5 ratio / (1 + d/a + d^2 / 2a^2)
  at every frequency, and the same function adds the inductance that causality demands.
- With `finish = "ENIG"` the faces of the outer layers that face away from the board are gold over
  nickel over the copper slab, each metal a transmission line section of its own impedance
  sqrt(j w mu rho) and thickness. Under a stackup mask layer only the pads are plated. The nickel
  is the plated nickel that Shlepnev and McMorrow identified from ENIG microstrip
  measurements ("Nickel characterization for interconnect analysis", IEEE EMC Symposium 2011):
  resistivity 1.0e-7 ohm m and a Landau-Lifshitz permeability falling from 6 to 2 through a
  resonance at 2.6 GHz with damping 0.18 f0. Gold is 2.44e-8 ohm m. With the default 4.5 um of
  nickel and 0.075 um of gold the plated face has 6 to 8 times the resistance of smooth copper
  from 0.3 to 3 GHz, peaking at the resonance, and about 3 times from 6 to 40 GHz. Faces against
  a dielectric stay rough copper, and the plated face takes no roughness.

Each distinct face (per copper thickness) is fitted with a common set of about 24 poles by vector
fitting (Gustavsen and Semlyen, IEEE Trans. Power Delivery 14(3), 1999) from a thousandth of the
band centre to 100 times the top frequency (within 0.02% of the functions above for a 20 GHz
band centre), and made passive by raising the series resistance if the fit dips below zero
anywhere. The sheet edge steps one state per real pole and one complex state per pole pair,
shared between its two faces, with the trapezoidal rule, so the discrete sheet is the bilinear
image of the fitted impedance (a CPU replay of the table matches it to 5e-6).

A grid puts a zero-thickness edge about a third of a cell past its last mesh line, so a strip
drawn on the grid reads wider than it is, and real copper of thickness t reads wider again by
(t / 2 pi)(1 + ln(4h / t)) per edge, h being the distance to the nearest other copper layer
(the Hammerstad-Jensen thickness correction). The mesher sets the last line of each trace, pad
and pour edge so the two land together: the grid's own offset comes from a finite difference
solve of a slit on the local cell shape. A 0.3 mm microstrip on 0.15 mm of air reads 81.9 and
82.2 ohm at 0.05 and 0.025 mm cells against 82.4 ohm from Hammerstad-Jensen with 35 um copper;
drawing the edges on mesh lines gave 82.6 and 85.1.

The current crowding at a trace edge is narrower than a cell, so the cells within four of an
edge get their resistance from the thin strip edge solution instead: the same slit solve gives
how much current the grid puts in each cell, and the loss of each band is the thin strip
integral cut short of the edge (Lewin and Vainshtein's stopping distance method). For a square
edge of thickness t that distance is t e^-pi / (4 pi), from a conformal map of the slab edge in
the strong skin effect limit. The 0.3 mm microstrip at 3.5 GHz reads 11%, 3% and 0.5% over
the 2D field solver at 0.1, 0.05 and 0.025 mm cells, and a 0.2 mm stripline 3% at 0.05 mm.

### Field maps and emissions

An FDTD sim can also record the field in the prepreg between the first two copper layers and
the far field, at up to four frequencies, for each excited port.

```toml
fields = ["1090MHz", "2.4GHz"]
far_field = true
```

Maps: E (dBV/m) and H (dBA/m) for 1 mW incident on the port, 60 dB deep. The far field comes from
a near-to-far transform on a box just inside the absorbing boundary. Readings: radiated power as a
fraction of the input, directivity, and the peak field at 3 m for 1 mW in, against FCC class B.
Ports are matched loads at the pads, so cables and connectors are not part of the radiator.

### Cascade (`kind = "cascade"`)

Drops measured devices into the S-parameters of a board sim. Every port of the board sim must
have been driven (leave `excite` out). The ports not joined to a device are the ports of the
result, which is written as `<name>.result.json` and a Touchstone file like any FDTD run.

```toml
name = "lna-cascade"
kind = "cascade"
board = "lna-rf"               # the FDTD sim of the board around the devices

[[devices]]
ref = "U1"
file = "spf5189z.s2p"          # Touchstone 1.x, S parameters, any of MA / DB / RI
ports = ["AMP_IN", "AMP_OUT"]  # board port for device port 1, 2, ...

[[devices]]                    # a two-terminal part from its vendor fixture data
ref = "L1"
file = "models/coilcraft_bcr-162.s2p"
ports = ["L1"]                 # one board port, at the part's RF pad or across its pads
mount = "shunt"                # how the vendor measured it: "shunt" to ground or "series"
```

A mounted part's impedance is taken from whichever of S11 and S21 is better conditioned, then
joined to the board port as a one-port.

Noise and linearity come along when the data is there:

```toml
bandwidth = "2MHz"             # for the noise floor and SFDR; without it the floor is per Hz
report = ["1090MHz"]           # frequencies for the budget readings, default the datasheet rows
ambient = 25.0                 # board temperature for its thermal noise, C

[after]                        # the stage behind the output, e.g. the receiver
nf = 6.0                       # dB
# iip3 = -10.0                 # dBm

[[devices]]
ref = "U1"
file = "models/qorvo_spf5189z.s2p"
ports = ["AMP_IN", "AMP_OUT"]

[[devices.datasheet]]          # one row per frequency, every column optional
freq = "0.9GHz"
nf = 0.55                      # dB with a 50 ohm source
oip3 = 38.5                    # dBm
p1db = 22.4                    # output P1dB, dBm
```

Noise is carried as noise-wave correlation matrices: the passive board and mounted parts from
their S-matrices at the board temperature, an amplifier from the Touchstone noise block when the
file has one (NFmin, Gamma opt, Rn), otherwise from the datasheet `nf` taken as NFmin with
Gamma opt = 0, which is exact only when the board presents 50 ohm. Outside the data the noise
figure is left blank rather than guessed. Linearity refers each device's OIP3 and P1dB to the
input through the gain the network gives it and adds them as reciprocals; `after` joins by
Friis. Readings give gain, NF, noise floor (-174 dBm/Hz + NF + 10 log B), SFDR (2/3 of IIP3 over
the floor), IIP3, OIP3 and output P1dB at each reported frequency, and the viewer plots the
noise figure and mu next to the Smith chart.

Device data is interpolated linearly in real and imaginary parts, and the band is cut to where
every device has data. A two-port result reports gain peak and low, worst S11, S22 and S12, and
the lowest Edwards-Sinsky mu and Rollett K (mu above 1 at every frequency is unconditionally
stable). The result goes stale when the spec, the board result or a device file changes.

### DC drop (`kind = "dc"`)

Solves the copper of every layer as a resistive sheet (vias as plated barrels), with supply pads
held at their voltage and loads drawing current, by sparse Cholesky in f64. Parts carry DC
through `[[links]]`: resistors take their value, inductors and ferrite beads (`FB`) default to
0.1 ohm, anything else is open unless linked. A link joins pads 1 and 2 unless `a` and `b` name
others, so a load switch is `{ ref = "U5", resistance = "80mohm", a = "1", b = "6" }`.

```toml
name = "lna-dc"
kind = "dc"
layout = "lna"
cell = 0.05                    # raster cell in mm

[[supplies]]
pad = "D2.1"
voltage = "4.7V"

[[supplies]]
pad = "J3.2"
voltage = "0V"

[[loads]]
pad = "U1.3"
current = "90mA"
return = "U1.2"                # where the load current comes back

[[links]]
ref = "L2"
resistance = "1.4ohm"          # inductor DCR from its datasheet
```

Maps: drop from each copper island's supply (mV), current density (A/mm2), voltage. Readings:
supply currents, load voltages, peak density and its place, the busiest vias.

### Thermal (`kind = "thermal"`)

Steady-state conduction through FR4 (0.8 W/mK in plane, 0.3 through), copper sheets and via
barrels, with convection from both faces, solved on the GPU by preconditioned conjugate gradient.

```toml
name = "lna-thermal"
kind = "thermal"
layout = "lna"
cell = 0.2                     # default 0.2 mm
ambient = 25.0
# h_top = 10.0                 # W/m2K, still air; 25+ with a fan
# h_bottom = 10.0

[[sources]]
ref = "U1"
power = "0.45W"
theta_jc = 65.0                # adds a junction estimate: pads + P x theta
pads = ["2"]                   # the pads the heat leaves through, default all
```

Maps: temperature per copper layer. Readings: board peak, each source's pad and junction
temperature.

### Logic (`kind = "logic"`)

An event-driven simulation of the digital parts of a schematic, straight from its netlist; no
layout is needed. `agentee new sim NAME --kind logic` (MCP `new_item` with `kind = "sim"` and
`sim_kind = "logic"`) writes a starter with a clock, a reset and a clocked assertion to rename
to your nets; `--kind fdtd`, the default, starts an FDTD run instead. `agentee sim NAME` (MCP `run_sim`) writes `NAME.result.json` and a VCD
waveform `NAME.vcd` next to the spec; the viewer draws a trace per recorded net against time.

In the waveform view the mouse wheel (or pinch, or ctrl and the wheel) zooms about the pointer,
a drag or a sideways scroll pans, a double click fits the whole run, and a click sets the time
cursor, snapped to an edge of the row under it within a few pixels. `+` and `-` zoom about the
cursor, the arrow keys pan, `F` or `Home` fits, `Escape` drops the cursor, and `N` and `P` jump
the cursor to the next and previous marker. With more rows than fit, the wheel over the names,
a drag or click on the bar at the right edge, and `Page Up` and `Page Down` scroll the rows
(`top=N` in `--show` starts at row N). The column beside the names reads every row at the
cursor (at the pointer with no cursor, else at the right edge of the view), and the hover tip gives the
time from the cursor. Nets named `X[0]` to `X[n]`, or `X0` to `Xn` (also `X0_N` to `Xn_N`),
fold into one bus row drawn in hex, MSB first, with a nibble of x for any unknown bit and z for
a floating one; so does a `record` entry `{ name, nets }`. Click a bus name to open its bits.
The VCD holds the same buses as vectors in place of their bits: `Q0` to `Q3` as
`$var wire 4 # Q [3:0] $end`, `Y0_N` to `Y7_N` as `Y_N [7:0]` and a `record` bus of n nets as
`NAME [n-1:0]`, MSB first.
Markers along the top show each failed assertion (red, at the time it was checked), timing
violation (amber) and contention (violet), with a dot on the rows of the nets involved; hover
one for its message, click it to put the cursor there. A run keeps its first 10000 markers,
and the header says capped when it reached that. `agentee render NAME --show
from=300ns,to=900ns,cursor=550ns,open=Q[3:0]` draws the same view headless.

```toml
name = "counter"
kind = "logic"
schematic = "counter"          # default: the only top-level schematic
duration = "2us"
ignore = ["J1", "J2"]           # parts with no logic model to leave out
record = ["CLK", "Q3", "Q2", "Q1", "Q0"]   # default every net but the rails
# record = ["CLK", { name = "COUNT", nets = ["Q3", "Q2", "Q1", "Q0"] }]   # a bus, MSB first
# on_violation = "keep"         # default "x": a timing violation makes the flip-flop x

[[stimulus]]
net = "CLK"
clock = { period = "100ns", duty = 0.5, phase = "50ns" }

[[stimulus]]
net = "RST_N"
steps = [["0ns", 0], ["120ns", 1]]   # [time, level] in time order; levels 0, 1, "x", "z"

[[stimulus]]
net = "MODE"
constant = 1

[[expect]]                      # a level at a time
net = "TC"
at = "1600ns"
value = 1

[[expect]]                      # a bus, MSB first: a number or a string of 0 1 x z and - (any)
nets = ["Y7_N", "Y6_N", "Y5_N", "Y4_N", "Y3_N", "Y2_N", "Y1_N", "Y0_N"]
at = "600ns"
value = "11011111"

[[expect]]                      # a sequence sampled on clock edges
nets = ["Q3", "Q2", "Q1", "Q0"]
clock = "CLK"
edge = "rising"                 # or "falling"
from = "0ns"                    # edges before this are skipped
sequence = [0, 0, 1, 2, 3, "01--"]
```

A clock is low until `phase`, then high for `duty` of each `period`; with no phase it starts
high at 0. A stimulus drives its net as a strong driver; on a supply net it replaces the rail.
An `at` check reads the net once everything at that time has settled; a sequence reads the
nets just before each clean edge (0 to 1, or 1 to 0) of the clock, so a registered output is
seen as it was when the edge came. Nets are named as in the schematic, and a net joined to
another through a 0 ohm link answers to either name.

The netlist becomes cells as follows:

- Nets with `style = "power"` and power symbols (a symbol with `power = true`) are constant
  sources: a name with `GND`, or starting `VSS`, `VEE` or `0V`, is 0, any other is 1
  (`PWR_FLAG` is skipped).
- A resistor (`R`) of 0 ohm, a jumper (`JP`, `SJ`, pins 1 and 2) and a net tie (`NT`) join
  their nets into one. Any other resistor between a rail and a net is a pull-up or pull-down, a
  weak driver of the rail's level; between two signal nets it joins them; between two rails it
  is left out.
- Capacitors, inductors, beads, filters, diodes, LEDs, test points, holes, fiducials and
  crystals (`C`, `L`, `FB`, `FL`, `D`, `LED`, `TP`, `H`, `MH`, `FID`, `Y`, `X`) are left out.
- Parts whose value (or else symbol name) holds a 74 series number take the built-in model by
  pin number: `74HC00`, `SN74LVC1G08DBVR` and `CD74HCT04E` all match. The library covers 00,
  01, 02, 03, 04, 05, 06, 07, 08, 10, 11, 14, 20, 21, 27, 32, 74, 86, 125, 126, 132, 138, 157,
  161, 163, 164, 244, 245, 573, 574 and 595, the single gates 1G00, 1G02, 1G04, 1G06, 1G07,
  1G08, 1G14, 1G17, 1G32, 1G34, 1G74, 1G79, 1G80, 1G86, 1G125, 1G126 and 1G157, and the dual
  gates 2G00, 2G02, 2G04, 2G08, 2G14, 2G17, 2G32, 2G34, 2G74, 2G86, 2G125 and 2G126. A unit of a
  multi-unit symbol that is not placed is left out; a placed pin with no net reads as floating
  (z).
- The 14 (and 1G14, 2G14) Schmitt triggers are inverters; the sim has no slow analog edge for
  the hysteresis to act on. The 573 is eight `dlatch` and the 574 eight `dff`, each with the shared `OE`
  (pin 1, active low) floating the outputs. The 245 is eight `xcvr`: with `OE` (pin 19) low it
  drives B from A while `DIR` (pin 1) is high and A from B while it is low. The 01, 03 (quad
  NAND), 05, 06, 1G06 (inverters) and 07, 1G07 (buffers) are open drain: they pull low or let
  go (z), and a pull-up resistor makes the 1. Every 01 has the 7402 pinout (outputs on 1, 4,
  10, 13, inputs after each), per TI SDLS026 (SN7401, SN74LS01) and Renesas REJ03D0532
  (HD74HC01); the 03 has the 7400 one, per TI SCLS077 (SN74HC03).
- A gate whose output pins are all `open_collector` in its symbol is open drain as well.
- Parts whose value or symbol is a primitive name (`AND`, `NAND3`, `OR`, `NOR`, `XOR`, `XNOR`,
  `NOT`, `INV`, `BUF`, `TRIBUF`, `DFF`, `JKFF`, `SRLATCH`, `DLATCH`, `MUX2`) map their pins by
  name: `D`, `CLK` (also `C`, `CP`, `CK`), `S` / `SET` / `PRE`, `R` / `RST` / `CLR`, `EN`, `OE`,
  `Q`, `~{Q}`; a gate's output is its `output` pin (or `Y`, `Q`, `OUT`) and every other signal
  pin is an input. An overbar (`~{R}`), a trailing `#` or `_N`, or an inverted pin shape makes
  the pin active low.
- Any other part is an error naming it, unless it is in `ignore` or has a `[[parts]]` model.

Family timing, from the family letters after 74 (propagation delay, setup, hold, and the
recovery and removal of an asynchronous set or reset):

| Family | Delay | Setup | Hold | Recovery | Removal |
|---|---|---|---|---|---|
| HC, HCT | 10 ns | 15 ns | 3 ns | 8 ns | 0 |
| AHC, AHCT, VHC, VHCT | 6 ns | 5 ns | 1 ns | 3.5 ns | 0 |
| AC, ACT | 6 ns | 4 ns | 1 ns | 2.4 ns | 0 |
| LVC | 4 ns | 3 ns | 1 ns | 2 ns | 0 |
| ALVC, LVT, ALVT, AUC, AVC | 4 ns | 2 ns | 1 ns | 2 ns | 0 |
| AUP, LV, LVX | 6 ns | 3 ns | 1 ns | 3 ns | 1 ns |
| LS and plain 74 | 15 ns | 20 ns | 5 ns | 25 ns | 3 ns |
| others | 10 ns | 10 ns | 2 ns | 10 ns | 2 ns |

Recovery and removal are datasheet minimums over -40 to 85 C at 4.5 V (5 V families) or
3.3 V (LVC), the larger of TI and Nexperia where both list one:

- HC, HCT: 8 ns trec of `nSD`, `nRD` to `nCP`, Nexperia 74HC_HCT74 rev 9 (TI SN74HC74
  SCLS094F gives 6 ns, TI CD74HC74 SCHS124E 8 ns, named trem there).
- AHC, AHCT: 3.5 ns trec of `nRD` to `nCP` for the 74AHCT74 (3.0 ns for the 74AHC74), Nexperia
  74AHC_AHCT74 rev 11 (TI SN74AHC74 SCLS255N gives 3 ns).
- AC, ACT: 2.4 ns trec at 5 V, TI CD74AC74 SCHS231E and CD74ACT74 SCHS321A (TI SN74AC74
  SCAS521H gives 0).
- LVC: 2 ns setup of `PRE` or `CLR` inactive before `CLK`, TI SN74LVC74A SCAS287W (Nexperia
  74LVC74A gives 1.0 ns).
- LS: 25 ns setup of `CLR` inactive and 3 ns hold at any input, TI SN74LS161A SDLS060 (the
  SN74LS74A in SDLS119 lists neither).

None of the CMOS sheets above (nor TI SN74HC161 SCLS297D or Nexperia 74LVC161, whose hold
covers the synchronous inputs only) lists a removal time, so it is 0 there. The AUP, LV, LVX
and other rows take their setup and hold.

Primitives by name and generic symbols take 1 ns and no setup or hold.

A custom part gets a model inline, matched by `ref` or by `value` (every part with that
value). Several entries for one part add a cell each (one per gate of a quad, say). An entry
with only timing keeps the built-in model and changes its timing.

```toml
[[parts]]
ref = "U7"
primitive = "nand"              # a primitive; pins map its pin keys to the part's pins
pins = { A = "1", B = "2", Y = "3" }
delay = "3ns"                   # every output; setup, hold, recovery and removal as well
delays = { Y = "5ns" }          # or per output key

[[parts]]
value = "MYBUF8"
primitive = "74HC244"           # borrow a library pinout

[[parts]]
ref = "U9"
inputs = ["1", "2", "3"]         # pins by number or unique name
outputs = ["4"]
truth = ["000 1", "1-- 0", "01- z"]   # first matching row wins; no match (or an x input) gives x
```

Primitive pin keys (inputs, then outputs); append `_N` to a key for an active-low pin or an
inverted output, and leave out an optional input to hold it inactive:

| Primitive | Inputs | Outputs |
|---|---|---|
| `and`, `or`, `xor`, `nand`, `nor`, `xnor` | any keys but `Y` | `Y` |
| `not`, `buf` | one key | `Y` |
| any gate with `_od` (`nand_od`, `not_od`, `buf_od`) | as the gate | `Y`, open drain: 0 or z |
| `tri` | `A`, `OE` | `Y` (z when `OE` is low) |
| `dff` | `D`, `CLK`, optional `S`, `R`, `OE` | `Q`, `QN` (z when `OE` is low) |
| `jk` | `J`, `K`, `CLK`, optional `S`, `R` | `Q`, `QN` |
| `sr` | `S`, `R` | `Q`, `QN` |
| `dlatch` | `D`, `EN`, optional `R`, `OE` | `Q`, `QN` (z when `OE` is low) |
| `xcvr` | `A`, `B`, `DIR`, optional `OE` | `A`, `B`: the same pins, driven from the other side by `DIR` |
| `mux2` | `I0`, `I1`, `S`, optional `EN` | `Y` |
| `dec138` | `A0`, `A1`, `A2`, `E1`, `E2`, `E3` | `Y0` to `Y7` |
| `counter161`, `counter163` | optional `R`, `CLK`, optional `D0` to `D3`, `CEP`, `LOAD`, `CET` | `Q0` to `Q3`, `TC` |
| `shift164` | `A`, optional `B`, `CLK`, optional `R` | `Q0` to `Q7` |
| `shift595` | `D`, `CLK`, `LATCH`, optional `R`, `OE` | `Q0` to `Q7`, `QS` |

Flip-flops clock on the rising edge (key `CLK_N` for the falling one); `S` and `R` are
asynchronous and both active give Q = QN = 1, as on a 74HC74. The 161 resets at once, the 163
on the next edge. A 595 shifts on `CLK`, copies to its outputs on `LATCH` (the value from before
a shift at the same instant), and floats them while `OE` is low.

The engine keeps four levels, 0, 1, x (unknown) and z (floating). Each net resolves its
drivers: strong drivers (outputs, rails, stimuli) that agree set the level, 0 against 1 gives x
and a reported contention, and pull resistors count only when no strong driver is on. A z
input reads as x. Gates only go to x when the unknown input matters (0 into an AND is 0), and a
flip-flop clocked by an unsure edge (0 to x, or x to 1) goes to x unless it would keep its
value. Every output has its own delay (transport, so pulses shorter than the delay pass);
events run in time order, and changes with zero delay settle in delta cycles at the same
instant. A zero-delay loop that is still changing after 1000 delta cycles stops the run and
reports the nets, and so does a run past 50 million events.

Flip-flops, counters and shift registers check setup and hold on their clocked inputs against
the model's timing: a data input that changed less than `setup` before a rising clock edge, or
less than `hold` after it, is a violation (not checked while an asynchronous reset or set is
active). A D latch checks its `D` against the closing (falling) edge of `EN` the same way. An
asynchronous reset or set released less than `recovery` before a clock edge, or less than
`removal` after one, is a violation too, with the family table's recovery and removal. A 595
also wants its `LATCH` (RCLK) rising edge at least `setup` after the last `CLK` (SRCLK) rising
edge; the two clocks tied together (the same instant) is fine and latches the value from before
the shift.

On a violation the flip-flop, latch or register the check covers goes to x, as the Verilog
models do with their notifiers, until it is clocked, set or reset cleanly again. Set
`on_violation = "keep"` on the sim to report the violation and keep the sampled value instead.

Readings: assertions passed and failed, contentions, timing violations, cells and events. Check
lists every failed assertion, contention, timing violation or stopped run as an error on the
sim, while the result is current. The result goes stale when the spec or the schematic's
netlist changes. `examples/logic` holds two: `counter`, a 74HC161 counting into a 74HC138,
and `i2c`, an open-drain bus where a controller (two 74LVC1G07) and a target (a 74LVC1G06
pulling SDA low for its ACK) share 4.7k pull-ups, and a 74HC595 clocked by SCL captures the
byte, checked for START, the bits, the ACK, the byte (a `record` bus) and STOP.

## Graphics

Shared by symbols and footprints. `kind` picks the shape and the fields it needs:

| kind | fields |
|---|---|
| `line` | `start`, `end` |
| `rect` | `start`, `end` (opposite corners) |
| `polyline` | `points` (open) |
| `polygon` | `points` (closed) |
| `circle` | `center`, `radius` |
| `arc` | `start`, `mid`, `end` (`mid` is any point on the arc between them) |
| `text` | `text`, `at`, `size`, `rotation`, `anchor` (`left` / `center` / `right`) |

Plus `width` (stroke), `fill` (`none` / `solid` / `background`), and `layer` (footprints) or
`unit` (symbols). On layout `[[graphics]]`, `locked = true` keeps `agentee place` from moving a
text.

## Workflow

1. `agentee search symbol lm358` then `agentee import symbol Amplifier_Operational:LM358 --with-footprint`
   when KiCad has the part. Imports land in `symbols/` and `footprints/`.
2. Otherwise `agentee new symbol NAME` / `agentee new footprint NAME` and edit the starter file.
3. `agentee new board NAME`, pick a stackup preset and fab, add net classes.
4. Quick-start placement: once the schematic checks, `agentee place NAME` puts every part on the
   board (connectors on edges, big chips central, decoupling caps at their pins). Look at it with
   `agentee render pcb:NAME`, move and `locked = true` the parts that matter, run `agentee place
   NAME --keep-placed` or with another `--seed` for the rest, then `agentee silk NAME`.
5. `agentee check` until there are no errors, `agentee render NAME -o out.png` to look.
6. `agentee view` keeps a live window open for a human, who can also move parts, route tracks
   and place vias there and save them into the layout.
7. `agentee fab NAME -o fab/` writes the manufacturing package once the layout has no errors.

## Importing a KiCad board

`agentee import board path/to/NAME.kicad_pcb --dir DIR` turns a whole KiCad board into a project:

| file | from |
|---|---|
| `NAME.board.toml` | the Edge.Cuts outline (closed loops inside it become `[[outline.cutouts]]`, loops outside it are left out and reported), the stackup with thickness, er and loss tangent, the copper finish and mask colour, the design rules and net classes from `NAME.kicad_pro` |
| `NAME.pcb.toml` | footprint placements, tracks (arcs as short segments), vias and zones, with agentee's zone fills stored, and the silk and fab text and lines drawn on the board, with `${TITLE}`, `${DATE}` and the project's text variables filled in |
| `NAME.sch.toml` | every part with its value, and each net as a list of pins, drawn with net labels |
| `footprints/` | each footprint as it sits on the board, bottom-side ones flipped back to the top, pad drill offsets kept |
| `symbols/` | one box symbol per footprint with a pin per pad number |

KiCad vias keep their type: `micro` becomes a microvia, `blind` a blind or buried via by its span,
and each distinct drill, pad, span, fill and backdrill becomes a board `[[vias]]` type. KiCad 9
stores only a per-via `tenting`, solder mask rather than a via fill, which is left out, and has no
backdrill or depth drill. The later format's `filling`, `capping`, `covering` and `plugging` map to
the IPC-4761 `fill`, and its `backdrill` (from the top) and `tertiary_drill` (from the bottom)
become `backdrill` at their `size`; KiCad lists the layers the backdrill removes, so `to` is the
copper layer past the last of them. Neither has a controlled depth via, so an imported blind via
is drilled mechanically: set `drill_kind = "controlled_depth"` on its type where the fab drills it
by depth.

Coordinates move so the outline starts at 0, 0. Teardrops and keepout areas are left out and
reported. A class whose tracks run narrower than its width takes the narrowest one, since KiCad
treats the class width as a default and agentee as a minimum. Clearances are checked with
KiCad's 0.5 um tolerance. On KiCad's `video` and `complex_hierarchy` demos every pad of the
imported layout lands where KiCad's own IPC-D-356 export puts it (2089 and 165 pads).

## Fab package

`agentee fab pcb:NAME -o DIR` (MCP `fab`) writes:

| file | what |
|---|---|
| `F_Cu.gbr` ... `B_Cu.gbr` | copper per layer, RS-274X with X2 file attributes, zone fills as regions |
| `F_Mask.gbr`, `B_Mask.gbr` | mask openings at the pad outlines, vias tented |
| `F_Paste.gbr`, `B_Paste.gbr` | paste on SMD pads |
| `F_SilkS.gbr`, `B_SilkS.gbr` | silk lines, artwork and text in the Hershey stroke font, with the `agentee vX.Y.Z-HASH` watermark |
| `Edge_Cuts.gbr` | the board outline and each board cutout as a closed profile; the fab routes cutouts from it, so they stay out of the drill files and `fab-notes.txt` counts them |
| `drill-PTH.drl`, `drill-NPTH.drl` | Excellon, metric, slots as G85; the plated file holds the pad holes and the through vias, unchanged for a board with through vias only |
| `drill-FROM-TO.drl` | one Excellon file per blind, buried or microvia span, as KiCad splits them, e.g. `drill-F.Cu-In1.Cu.drl`, with `TF.FileFunction,Plated,1,2,Blind` (`Buried` for inner spans, layers numbered from 1 at the top) and a `; span F.Cu to In1.Cu, microvia vias` comment |
| `drill-FROM-TO-controlled-depth.drl` | the controlled depth vias of a span, kept apart from the vias drilled in a sub-stack or by laser over the same span since the fab drills them in a different pass, after the last press and by depth; same file function, with the drilled side and stop layer in a `; span In4.Cu to B.Cu, controlled depth blind vias drilled from B.Cu after the last press, stopping on In4.Cu` comment |
| `drill-backdrill-FROM-TO.drl` | the backdrill holes of each side and stop layer, non-plated, at the backdrill diameter, with the stop layer and `max_stub` in a comment |
| `bom.csv`, `bom-jlcpcb.csv` | grouped by value, footprint, `mpn` and `lcsc` fields |
| `cpl.csv` | placement, JLCPCB columns |
| `fab-notes.txt` | the agentee version and watermark spot, stackup, finish, impedance classes, each via type with its span, laser, mechanical or controlled depth drill, sizes, hole count, IPC-4761 fill and drill file, and each backdrill with its side, stop layer and stub, vias in pads to fill, edge pads to keep |
| `NAME.d356` | IPC-D-356A netlist for the fab's bare-board electrical test, columns as KiCad writes them; test point pads are end points with the probe side access code (`A01` top, the copper layer count for the bottom, `A02` on 2 layers and `A08` on 8), vias are tented mid points unless `[test] vias = true` makes them probe side access; an unprobed via's access code follows the layers its copper reaches (a backdrill takes its side away): `A00` through, `A01` from the top, the bottom layer number from the bottom, the first layer's number for a buried via |
| `testpoints.csv` | every test point pad for the fixture builder: ref, pad, net, X, Y (mm, Y up), side, pad diameter |
| `NAME-gerbers.zip` | every Gerber and drill file, ready to upload to the fab |
| `assembly-top.png`, `assembly-bottom.png` | fab and silk layers for the line |

Coordinates are mm with Y up, the same in the Gerbers and the placement file. Parts marked `dnp`,
with `assembly = "no"` in their fields, or on MountingHole and Fiducial footprints stay off the
BOM and placement file. The silk font is the one the viewer draws, so text sizes and overlaps
read the same on screen as on the board.

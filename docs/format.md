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
fab = "jlcpcb"                 # rule preset: generic | jlcpcb

[outline]                      # a rectangle...
size = [50, 30]
origin = [0, 0]                # top-left corner, default [0, 0]
corner_radius = 1
# points = [[0,0], [50,0], [50,30], [0,30]]   # ...or a polygon

[stackup]
preset = "jlcpcb-4l-1.6mm-7628"
finish = "ENIG"
mask_color = "green"
silk_color = "white"

[rules]                        # overrides the fab preset, any subset
min_track_width = "0.15mm"

[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
# from = "F.Cu"                # default: outermost copper on each side
# to = "B.Cu"

[[netclasses]]
name = "Default"               # always define Default
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"

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

### Stackup presets

| preset | build |
|---|---|
| `jlcpcb-2l-1.6mm` | 2 layer FR4, 1 oz outer |
| `jlcpcb-4l-1.6mm-7628` | JLC04161H-7628, 7628 prepreg |
| `jlcpcb-4l-1.6mm-3313` | JLC04161H-3313, 3313 prepreg |

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

### Rules

All lengths: `min_track_width`, `min_clearance`, `min_drill`, `min_via_drill`, `min_via_diameter`,
`min_annular_ring`, `min_hole_to_hole`, `min_copper_to_edge`, `min_silk_width`,
`min_silk_text_height`. Footprints are checked against the rules of the board when the project has
exactly one board, otherwise against `generic`.

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
```

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
height = "1.5mm"               # box height for the 3D view when there is no model
model = "${KICAD9_3DMODEL_DIR}/Package_SO.3dshapes/SOIC-8_3.9x4.9mm_P1.27mm.step"
model_offset = ["0mm", "0mm", "0mm"]   # optional, as in KiCad: model frame, Y up
model_rotate = [0, 0, 0]               # optional, degrees about X, Y, Z
model_scale = [1, 1, 1]                # optional

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

Check looks for overlapping pads, pads closer than the fab clearance, drills and annular rings
under the rules, a courtyard that encloses the pads, and silk that runs over exposed copper.

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
class = "RF"                   # a netclass of the board, default "Default"
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
the design. A sheet on its own skips the single-pin warning (the rest of the net is elsewhere);
the top schematic runs every check on the joined design and draws the sheets stacked top to
bottom. The top file may hold parts and nets of its own too.

Check reports pins in two nets, pins in no net, single-pin nets, several outputs on one net,
hand wires that miss a pin or touch another net's pin, overlapping parts, and footprints whose
pads do not cover the symbol's pins. `agentee show sch:lna` prints every pin's position.

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

[[tracks]]
net = "RF_OUT"
layer = "F.Cu"
points = [[18.68, 10], [25.5, 10]]
# width = 0.36                 # default: the net class width

[[vias]]
net = "GND"
at = [7.2, 8.9]
# via = "std"                  # a [[vias]] name from the board, default the class via
# count = 6                    # a row, like pads
# pitch = [1.2, 0]

[[fanouts]]                    # a via in every connected pad of a BGA
ref = "U3"                     # * and ? globs: ref = "*" with nets = [...] fans out every plane pad
# via = "bga"                  # default: each net's class via
# skip_rings = 2               # leave the two outer rings for escape on the outer layer
# always = ["GND", "3V3"]      # nets that get a via even in those rings
# skip = ["A1", "B7"]          # pads to leave alone
# nets = ["GND", "3V3"]        # only pads on these nets, globs allowed
# exclude = ["C2?", "J1"]      # refs to leave out when ref is a glob

[[stitching]]                  # ground vias wherever they clear every other net
net = "GND"
# via = "std"                  # default the net's class via
# pitch = "2.5mm"              # grid pitch, or the spacing along a fence (default 1mm)
# outline = [[x, y], ...]      # default the board outline
# margin = "0.6mm"             # extra distance from the board edge
# fence = ["RF_*"]             # instead of a grid: a row either side of these nets' tracks
# offset = "0.5mm"             # fence row distance from the track centre, default just past
                               # the class coplanar gap

[[zones]]
net = "GND"
layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
# outline = [[x, y], ...]      # default: the board outline
# clearance = 0.25             # default: the net class clearance
# priority = 1                # higher fills first; other nets' zones on the layer pour around it

[[cutouts]]                    # keep zones off an area, e.g. under an SMA centre pin
layers = ["In1.Cu"]
points = [[0, 9], [5.4, 9], [5.4, 11], [0, 11]]

[[graphics]]                   # board text and lines, same keys as footprint graphics
kind = "text"
layer = "F.SilkS"              # F.SilkS, B.SilkS, F.Fab or B.Fab
at = [7.4, 7.6]
text = "RF IN"
size = 1.0                     # mm, the fab minimum is in the board rules

[[artwork]]                    # a filled logo or icon
layer = "F.SilkS"
icon = "arrow"                 # built in: arrow, warning, ground, antenna, lightning, pin1, ce
# file = "logo.svg"            # or any SVG, relative to this file
at = [7.4, 6.3]                # centre of the artwork
height = 0.8                   # mm, the width follows the aspect ratio
# rotation = 90
```

Silk text must keep 0.4 mm from other silk text and 0.2 mm from silk outlines, stay off pads,
vias and other parts' bodies, and stay on the board. When a reference label fails, check names a
spot that passes every rule, as a `label = { at = [...] }` line to paste. `agentee silk NAME`
(MCP `silk`) pastes them all for you and repeats until the labels settle; `--hide` hides the
references that have no clear spot, typically small passives under a BGA.

Zones on the same layer fill in order of `priority`, then smallest first, and each keeps its
clearance from the fills already placed, so a small switch-node or supply pour inside a
board-wide ground pour is poured around rather than shorted to it.

Stitching vias go only where the via clears every other net's copper on each layer it spans,
keeps `min_hole_to_hole` from every drill and the edge rule from the outline, and lands inside a
zone of its net; check reports how many it placed. They are drilled and plotted like any via.

Zone fills are exact polygons: the zone outline less every other net's copper grown by its
clearance, with round corners, so pours render and plot without stair steps. Necks and slivers
narrower than the zone's `min_width` (default 0.25 mm) are removed, the way a fab would etch them.

A track that only grazes a pad (its centre line misses the pad) is flagged; run it into the pad.
A track may neck down below its class width, to no less than the fab minimum, for up to 0.5 mm
(the class `neckdown`) where it meets a small pad. Drilled holes, vias and plated pads alike, must
keep the board's `min_hole_to_hole` apart; check counts the pairs that do not and names the first.

Artwork on a bottom layer is mirrored so it reads correctly from below. SVG fills and strokes are
flattened to polygons; text in an SVG is ignored, so convert it to paths first. Silk text and
artwork get the same checks as reference labels: overlap, pads, silk outlines, board edge.

### Autorouting

`agentee route NAME --nets 'FX_D*,SPI_*' --layers F.Cu,In2.Cu,B.Cu` (MCP `route`) routes the
ratsnest of the named nets on a grid (`--grid`, default 0.05 mm) and appends the tracks and vias
to the layout file as ordinary `[[tracks]]` and `[[vias]]`, so they are yours to edit afterwards.
It keeps each net class's width, clearance and `layers` against every pad, track, via, hole and
the board edge, keeps new vias `min_hole_to_hole` from every drill, uses the class via to change
layer (`--via` to override, `--via-cost` in mm of track), and never moves what is already there
unless `--reroute` is given, which deletes the named nets' tracks and vias first. A connection that finds no free path rips up the routed nets
it would cross, remembers the spot as congested, and those nets go back in the queue. Paths are
pulled tight into straight runs afterwards. The second net of a pair is drawn toward its
partner at the pair gap; `--pairs` tries to route both halves together as one coupled track
first. When a few connections fail, route them again together with the nets around them and
`--reroute`, so the router can rip up and reorder the whole area, or drop to `--grid 0.025`. `--dry-run` reports without writing. Route the nets that matter by hand
first, then let the router fill in the rest, a class at a time.

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

`agentee tune NAME` (MCP `tune`) fixes these: for every pair over its skew limit and every match
group member short of its target it meanders the short side, on its longest straight segments,
anywhere along a series chain, with bumps that keep every other net's clearance and the board
edge rule, and writes the new points into the tracks. `--nets` limits it, `--amplitude` caps the
bump height and `--pitch` fixes the bump pitch (default three track widths, tighter where that is
all that fits). A net that is over its group target is reported, not shortened. `agentee calc
serpentine --from x,y --to x,y --add 2.5mm` (MCP `serpentine`) returns the points of one such
meander on a segment you pick. Net lengths and delays are in `agentee show pcb:NAME`.

The viewer's layout page has a `3d` tab (`agentee view --3d` opens on it): the board in its
stackup thickness, mask and silk colours and finish, copper under the mask, bare pads, plated and
bare drill walls, and each part's 3D model. The `parts` toggle hides the models. Drag to orbit,
shift-drag to pan, scroll to zoom, double-click to reset. The viewer draws with OpenGL
([three-d](https://github.com/asny/three-d)); `agentee render pcb:NAME --show 3d` (or `3d-top`,
`3d-bottom`) draws the same scene in software, `--hide parts` leaves the models out and `--region`
aims the camera at that area.

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
text, crosses a silk outline, sits on a pad or runs off the board. Name an item with its kind when
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
or copper keep a conductivity fixed at the band centre instead. Copper layers are sheets
with the skin-effect surface impedance, sqrt(j w mu / sigma), so both the resistance and the
internal inductance follow sqrt(f) across the band. In the time domain that impedance is a sum
of about 20 RL branches per sheet edge (poles log-spaced from the frequency where the skin depth
reaches the copper thickness up to 100 times the top frequency, within 1% of sqrt(f)), updated
implicitly so the sheet stays stable at any time step. The stackup roughness factor is taken at
the band centre. A zero-thickness sheet has one current, while real copper carries it on two faces, so
each sheet edge tracks the magnetic field just above and below it through the run and scales its
resistance by (Jtop^2 + Jbottom^2) / (Jtop + Jbottom)^2: one half for a centred stripline, close to
one for a trace over a plane.

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
`unit` (symbols).

## Workflow

1. `agentee search symbol lm358` then `agentee import symbol Amplifier_Operational:LM358 --with-footprint`
   when KiCad has the part. Imports land in `symbols/` and `footprints/`.
2. Otherwise `agentee new symbol NAME` / `agentee new footprint NAME` and edit the starter file.
3. `agentee new board NAME`, pick a stackup preset and fab, add net classes.
4. `agentee check` until there are no errors, `agentee render NAME -o out.png` to look.
5. `agentee view` keeps a live window open for a human.
6. `agentee fab NAME -o fab/` writes the manufacturing package once the layout has no errors.

## Importing a KiCad board

`agentee import board path/to/NAME.kicad_pcb --dir DIR` turns a whole KiCad board into a project:

| file | from |
|---|---|
| `NAME.board.toml` | the Edge.Cuts outline (inner loops become cutouts), the stackup with thickness, er and loss tangent, the copper finish and mask colour, the design rules and net classes from `NAME.kicad_pro` |
| `NAME.pcb.toml` | footprint placements, tracks (arcs as short segments), vias and zones |
| `NAME.sch.toml` | every part with its value, and each net as a list of pins, drawn with net labels |
| `footprints/` | each footprint as it sits on the board, bottom-side ones flipped back to the top, pad drill offsets kept |
| `symbols/` | one box symbol per footprint with a pin per pad number |

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
| `F_SilkS.gbr`, `B_SilkS.gbr` | silk lines, artwork and text in the Hershey stroke font |
| `Edge_Cuts.gbr` | the board outline |
| `drill-PTH.drl`, `drill-NPTH.drl` | Excellon, metric, slots as G85 |
| `bom.csv`, `bom-jlcpcb.csv` | grouped by value, footprint, `mpn` and `lcsc` fields |
| `cpl.csv` | placement, JLCPCB columns |
| `fab-notes.txt` | stackup, finish, impedance classes, vias in pads to fill |
| `NAME.d356` | IPC-D-356A netlist for the fab's bare-board electrical test, columns as KiCad writes them |
| `NAME-gerbers.zip` | every Gerber and drill file, ready to upload to the fab |
| `assembly-top.png`, `assembly-bottom.png` | fab and silk layers for the line |

Coordinates are mm with Y up, the same in the Gerbers and the placement file. Parts marked `dnp`,
with `assembly = "no"` in their fields, or on MountingHole and Fiducial footprints stay off the
BOM and placement file. The silk font is the one the viewer draws, so text sizes and overlaps
read the same on screen as on the board.

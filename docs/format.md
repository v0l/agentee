# agentee file format

A project is a directory. Every file below it is loaded by its suffix:

| suffix | holds |
|---|---|
| `*.board.toml` | one board spec: fab rules, stackup, outline, vias, net classes |
| `*.sym.toml` | one schematic symbol |
| `*.fp.toml` | one footprint |

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
impedance (Hammerstad-Jensen microstrip, Wheeler stripline, uncoated), the width that meets the
target, and the IPC-2221 current capacity. `agentee show <board>` prints all of it as JSON.

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
model = "${KICAD9_3DMODEL_DIR}/Package_SO.3dshapes/SOIC-8_3.9x4.9mm_P1.27mm.step"

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
two. A custom pad takes `points`, a polygon relative to `at`.

Layers: `F.Cu`, `B.Cu`, `F.SilkS`, `B.SilkS`, `F.Mask`, `B.Mask`, `F.Paste`, `B.Paste`, `F.Fab`,
`B.Fab`, `F.CrtYd`, `B.CrtYd`, `Edge.Cuts`, `*.Cu`, `*.Mask`.

Check looks for overlapping pads, pads closer than the fab clearance, drills and annular rings
under the rules, a courtyard that encloses the pads, and silk that runs over exposed copper.

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

import os
import sys

import ezdxf

DXF = sys.argv[1]
OUT = sys.argv[2]
MM = 25.4
MIL = 0.0254

BOARDS = {
    "dut6cm": ("907-000-10140.dxf", "6 cm microstrip DUT"),
    "vias2": ("907-000-10144.dxf", "6 cm test fixture with two vias in series"),
    "thru3cm": ("907-000-10148.dxf", "3 cm 2x-thru"),
}

LAYERS = {"L01": "F.Cu", "L02": "In1.Cu", "L03": "In2.Cu", "L04": "B.Cu"}


def fmt(v):
    return f"{round(v, 4)}"


def pts(ps):
    return "[" + ", ".join(f"[{fmt(x)}, {fmt(y)}]" for x, y in ps) + "]"


def bulge_arc(p, q, b):
    import math

    theta = 4 * math.atan(b)
    c = math.hypot(q[0] - p[0], q[1] - p[1])
    if c < 1e-12:
        return []
    r = c / (2 * math.sin(theta / 2))
    mx, my = (p[0] + q[0]) / 2, (p[1] + q[1]) / 2
    d = r * math.cos(theta / 2)
    nx, ny = -(q[1] - p[1]) / c, (q[0] - p[0]) / c
    cx, cy = mx + d * nx, my + d * ny
    a0 = math.atan2(p[1] - cy, p[0] - cx)
    n = max(2, int(abs(theta) / math.radians(5)) + 1)
    return [(cx + abs(r) * math.cos(a0 + theta * k / n), cy + abs(r) * math.sin(a0 + theta * k / n)) for k in range(1, n)]


def poly(e, height):
    raw = list(e.get_points("xyb"))
    dense = []
    for i, (x, y, b) in enumerate(raw):
        dense.append((x, y))
        if abs(b) > 1e-9:
            nx, ny, _ = raw[(i + 1) % len(raw)]
            dense.extend(bulge_arc((x, y), (nx, ny), b))
    out = []
    for x, y in dense:
        p = (x * MM, height - y * MM)
        if out and abs(out[-1][0] - p[0]) < 1e-4 and abs(out[-1][1] - p[1]) < 1e-4:
            continue
        out.append(p)
    return out


def load(name):
    d = ezdxf.readfile(os.path.join(DXF, name))
    msp = d.modelspace()
    outline = next(e for e in msp if e.dxf.layer == "OUTLINE")
    ys = [p[1] * MM for p in outline.get_points("xy")]
    height = max(ys)
    polys, plated, pads = [], [], []
    for e in msp:
        layer = e.dxf.layer
        if e.dxftype() == "LWPOLYLINE" and layer in LAYERS:
            polys.append((LAYERS[layer], poly(e, height)))
        elif e.dxftype() == "CIRCLE" and layer == "PLATED":
            plated.append((e.dxf.center[0] * MM, height - e.dxf.center[1] * MM, e.dxf.radius * MM))
        elif e.dxftype() == "CIRCLE" and layer in LAYERS:
            pads.append((LAYERS[layer], e.dxf.center[0] * MM, height - e.dxf.center[1] * MM, e.dxf.radius * MM))
    return poly(outline, height), polys, plated, pads


def signal(layer, ring, trace_y):
    ys = [p[1] for p in ring]
    return min(ys) - 0.05 < trace_y < max(ys) + 0.05 and max(ys) - min(ys) < 1.0


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


def write_board(key, length):
    write(os.path.join(OUT, f"{key}.board.toml"), f'''name = "{key}"
description = "Signal Microwave / IEEE P370 plug and play kit coupons: Rogers RO4003C and RO4450F, NiAu finish, 17.3 mil microstrip on 8 mil (User's Guide rev 1, figure 2)"

[outline]
size = [{fmt(length)}, 18.009]

[stackup]
finish = "ENIG"
mask_color = "none"

[[stackup.layers]]
kind = "copper"
name = "F.Cu"
thickness = "{fmt(2.1 * MIL)}mm"

[[stackup.layers]]
kind = "core"
name = "diel1"
material = "RO4003C"
thickness = "{fmt(8 * MIL)}mm"
er = 3.55
loss_tangent = 0.0027

[[stackup.layers]]
kind = "copper"
name = "In1.Cu"
thickness = "{fmt(0.67 * MIL)}mm"

[[stackup.layers]]
kind = "prepreg"
name = "diel2"
material = "RO4450F"
thickness = "{fmt(4 * MIL)}mm"
er = 3.52
loss_tangent = 0.004

[[stackup.layers]]
kind = "core"
name = "diel3"
material = "RO4003C"
thickness = "{fmt(32 * MIL)}mm"
er = 3.55
loss_tangent = 0.0027

[[stackup.layers]]
kind = "prepreg"
name = "diel4"
material = "RO4450F"
thickness = "{fmt(4 * MIL)}mm"
er = 3.52
loss_tangent = 0.004

[[stackup.layers]]
kind = "copper"
name = "In2.Cu"
thickness = "{fmt(0.67 * MIL)}mm"

[[stackup.layers]]
kind = "core"
name = "diel5"
material = "RO4003C"
thickness = "{fmt(8 * MIL)}mm"
er = 3.55
loss_tangent = 0.0027

[[stackup.layers]]
kind = "copper"
name = "B.Cu"
thickness = "{fmt(2.1 * MIL)}mm"

[rules]
min_clearance = "0.05mm"
min_track_width = "0.1mm"
min_copper_to_edge = "0mm"
min_drill = "0.2mm"
min_via_drill = "0.2mm"
min_via_diameter = "0.3mm"
min_annular_ring = "0.05mm"
min_hole_to_hole = "0.2mm"

[[vias]]
name = "gnd8"
drill = "0.2032mm"
diameter = "0.3048mm"

[[vias]]
name = "gnd12"
drill = "0.3048mm"
diameter = "0.4064mm"

[[vias]]
name = "sig10"
drill = "0.254mm"
diameter = "0.508mm"

[[netclasses]]
name = "Default"
track_width = "0.1mm"
clearance = "0.05mm"
via = "gnd8"

[[netclasses]]
name = "RF"
impedance = "50ohm"
track_width = "0.4394mm"
clearance = "0.05mm"
solver = "field"
layers = ["F.Cu", "B.Cu"]
via = "sig10"
''')

os.makedirs(os.path.join(OUT, "footprints"), exist_ok=True)
os.makedirs(os.path.join(OUT, "symbols"), exist_ok=True)



write(os.path.join(OUT, "footprints", "P370_Port.fp.toml"), '''name = "P370_Port"
description = "Where the 1.85 mm edge connector pin lands on the tapered trace end, for a lumped port"
mount = "smd"

[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [0.1, 0.0]
size = [0.2, 0.279]
layers = ["F.Cu"]

[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [2.5, -2.5]
size = [1.0, 1.0]
layers = ["F.Cu"]
''')

write(os.path.join(OUT, "symbols", "Port.sym.toml"), '''name = "Port"
reference = "P"
description = "RF port"

[[bodies]]
left = [{ number = "1" }, { number = "2" }]
''')

for key, (fname, desc) in BOARDS.items():
    outline, polys, plated, pads = load(fname)
    length = max(p[0] for p in outline)
    write_board(key, length)
    trace_y = 18.009 - 9.0045
    zones, sig_zones = [], []
    for layer, ring in polys:
        if len(ring) < 3:
            continue
        is_sig = signal(layer, ring, trace_y) and layer in ("F.Cu", "B.Cu")
        net = "SIG" if is_sig else "GND"
        zones.append((net, layer, ring))
    lines = [f'name = "{key}"', f'board = "{key}"', f'schematic = "{key}"', ""]
    lines += ['[[footprints]]', 'ref = "P1"', f'at = [0.0, {fmt(trace_y)}]', 'label = { hide = true }', ""]
    lines += ['[[footprints]]', 'ref = "P2"', f'at = [{fmt(length)}, {fmt(trace_y)}]', 'rotation = 180', 'label = { hide = true }', ""]
    for net, layer, ring in zones:
        lines += ['[[zones]]', f'net = "{net}"', f'layers = ["{layer}"]', f'outline = {pts(ring)}', 'clearance = "0.05mm"', 'min_width = "0.05mm"']
        if net == "SIG":
            lines += ['priority = 2']
        lines += [""]
    signal_vias = {(round(x, 3), round(y, 3)) for layer, x, y, r in pads if abs(y - trace_y) < 0.05}
    kinds = {0.1016: "gnd8", 0.1524: "gnd12", 0.127: "sig10"}
    for x, y, r in plated:
        via = next((v for k, v in kinds.items() if abs(r - k) < 1e-3), None)
        if via is None:
            continue
        net = "SIG" if (round(x, 3), round(y, 3)) in signal_vias else "GND"
        lines += ['[[vias]]', f'net = "{net}"', f'at = [{fmt(x)}, {fmt(y)}]', f'via = "{via}"', ""]
    write(os.path.join(OUT, f"{key}.pcb.toml"), "\n".join(lines))
    write(os.path.join(OUT, f"{key}.sch.toml"), f'''name = "{key}"
description = "{desc}"
board = "{key}"

[[parts]]
ref = "P1"
symbol = "Port"
footprint = "P370_Port"
at = [25.4, 25.4]

[[parts]]
ref = "P2"
symbol = "Port"
footprint = "P370_Port"
at = [76.2, 25.4]
mirror = true

[[nets]]
name = "SIG"
class = "RF"
style = "label"
pins = ["P1.1", "P2.1"]

[[nets]]
name = "GND"
style = "label"
pins = ["P1.2", "P2.2"]
''')
    print(key, len(zones), "zones", len(plated), "holes")

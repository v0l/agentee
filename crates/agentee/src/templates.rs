pub fn board(name: &str) -> String {
    format!(
        r#"name = "{name}"
description = ""
fab = "jlcpcb"

[outline]
size = [50, 30]
corner_radius = 1

[stackup]
preset = "jlcpcb-2l-1.6mm"
finish = "ENIG"
mask_color = "green"

[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"

[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Signal"
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Ground"
track_width = "0.4mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Power"
track_width = "0.6mm"
clearance = "0.2mm"
current = "1A"
max_temp_rise = "10C"
via = "std"
"#
    )
}

pub fn symbol(name: &str) -> String {
    format!(
        r#"name = "{name}"
reference = "U"
description = ""

[[bodies]]
left = [
  {{ number = "1", name = "VDD", type = "power_in" }},
  {{ gap = 1 }},
  {{ number = "2", name = "IN", type = "input" }},
]
right = [
  {{ number = "3", name = "OUT", type = "output" }},
]
bottom = [
  {{ number = "4", name = "GND", type = "power_in" }},
]
"#
    )
}

pub fn footprint(name: &str) -> String {
    format!(
        r#"name = "{name}"
description = ""
tags = []

[[pads]]
number = "1"
kind = "smd"
shape = "roundrect"
at = [-0.8, 0]
size = [0.9, 1.0]
count = 2
pitch = [1.6, 0]

[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-1.5, -0.75]
end = [1.5, 0.75]

[[graphics]]
kind = "rect"
layer = "F.Fab"
start = [-0.8, -0.4]
end = [0.8, 0.4]

[[graphics]]
kind = "text"
layer = "F.SilkS"
text = "${{REFERENCE}}"
at = [0, -1.6]
"#
    )
}

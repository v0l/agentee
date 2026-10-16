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
voltage = "3.3VDC"
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Signal"
voltage = "3.3VDC"
track_width = "0.2mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Ground"
voltage = "0VDC"
track_width = "0.4mm"
clearance = "0.2mm"
via = "std"

[[netclasses]]
name = "Power"
voltage = "3.3VDC"
track_width = "0.6mm"
clearance = "0.2mm"
current = "1A"
max_temp_rise = "10C"
via = "std"
"#
    )
}

pub fn fdtd_sim(name: &str) -> String {
    format!(
        "name = \"{name}\"\n\n[frequency]\nstart = \"100MHz\"\nstop = \"4GHz\"\n\n[[ports]]\nname = \"IN\"\npad = \"J1.1\"\n"
    )
}

pub fn logic_sim(name: &str) -> String {
    format!(
        r#"name = "{name}"
kind = "logic"
description = ""
# schematic = "top"             # default: the only top-level schematic
duration = "2us"
# ignore = ["J1"]               # parts with no logic model to leave out
# record = ["CLK", {{ name = "COUNT", nets = ["Q3", "Q2", "Q1", "Q0"] }}]   # default every net
# on_violation = "keep"         # default "x": a setup, hold, recovery or removal miss makes x

# Rename the nets below to nets of your schematic.
[[stimulus]]
net = "CLK"
clock = {{ period = "100ns", duty = 0.5, phase = "50ns" }}

[[stimulus]]
net = "RST_N"
steps = [["0ns", 0], ["120ns", 1]]

[[expect]]
net = "Q0"
clock = "CLK"
edge = "rising"
from = "200ns"
sequence = [1, 0, 1, 0]

# [[parts]]                      # a part with no built-in model
# ref = "U7"
# primitive = "nand"
# pins = {{ A = "1", B = "2", Y = "3" }}
"#
    )
}

pub fn circuit_sim(name: &str) -> String {
    format!(
        r#"name = "{name}"
kind = "circuit"
description = ""
# schematic = "top"             # default: the only top-level schematic
# ignore = ["J*", "U5"]         # parts to leave out; connectors and test points are left out anyway
# record = ["OUT"]              # nets in the curves; default every net
# temperature = 27              # deg C, for the noise
# ground = "P_COM"              # another reference net besides GND, AGND, 0V and VSS

# Rename the nets below to nets of your schematic. Only drives drive a net, so give each supply
# the circuit does not make itself a drive of its own.
[[drives]]
net = "VIN"
dc = "1V"
ac = 1
# pulse = {{ low = "0V", high = "1V", rise = "1us", width = "1ms", period = "2ms" }}
# sine = {{ amplitude = "1V", frequency = "1kHz" }}
# resistance = "50ohm"

[ac]
from = "10Hz"
to = "10MHz"
points = 20                     # per decade

# [tran]
# stop = "10ms"
# step = "10us"

# [noise]
# output = "OUT"
# input = "VIN"                 # a drive with `ac`, for the input referred curve
# from = "10Hz"
# to = "100kHz"

[[checks]]
v = "OUT"                       # at the operating point
min = 0.9
max = 1.1

[[checks]]
v = "OUT"
at = "1kHz"                     # dB against the ac drive
min = -1

# [[blocks]]                     # a part's model, every value from its datasheet
# ref = "U1"
# kind = "opamp"                 # opamp, ldo, vcvs, vccs, or a resistor, diode, npn, nmos ... override
# gain = 1e5
# gbw = "1MHz"
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

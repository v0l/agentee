# IEEE P370 plug and play kit

The Signal Microwave kit that the IEEE P370 committee used to check de-embedding: Rogers RO4003C
and RO4450F, 17.3 mil microstrip on 8 mil, NiAu finish, 1.85 mm edge connectors. The layouts come
from Signal Microwave's DXF files (in the P370 repository, `TG2/SMW P370 Single Ended Layout DXF`)
and the stackup from the kit's User's Guide rev 1, figure 2. The measurements are in the IEEE P370
open source repository.

```sh
python make_project.py "Layout DXF 16 Feb 2018" .      # needs ezdxf; the output is committed
sh fetch_measured.sh measured
agentee sim dut6cm-s
python compare.py measured/M1_dut6cm.s2p dut6cm.s2p     # scikit-rf
```

| coupon | layout | measured |
|---|---|---|
| 6 cm microstrip DUT (907-000-10140) | `dut6cm` | M1: F/M adapter + DUT + M/F adapter |
| 6 cm fixture with two vias in series (10144) | `vias2` | only inside M10 / M17, not published |
| 3 cm 2x-thru (10148) | `thru3cm` | not published |

agentee's ports are lumped, at the tapered trace ends where the connector pins land. Every
published measurement includes the edge connectors and the calibration-grade adapters, so the
comparison is on what they do not change much.

## 6 cm microstrip DUT

The board file gives the copper roughness and the finish: 2.8 um rms on every copper face against
a dielectric (Rogers' "Copper Foils for High Frequency Materials", table 1: half ounce
electrodeposited foil on RO4003C, dielectric side), and ENIG on the outer faces with the default
4.5 um of nickel and 0.075 um of gold. The User's Guide gives the gold as "minimum of 75 um",
which cannot be right for an immersion finish; 0.075 um (3 uin) is the IPC-4552 range. The
nickel thickness is not published.

| | measured (M1) | agentee FDTD | agentee 2D solver |
|---|---|---|---|
| line impedance (TDR plateau, 350 to 650 ps) | 50.3 ohm | 49.7 ohm | 48.4 ohm at 1 GHz |
| line delay | 321 ps between the two launch reflections | 338 ps port to port | 329 ps (5.49 ps/mm) |
| S21 at 1 / 10 / 40 GHz | -0.47 / -1.93 / -5.99 dB | -0.27 / -0.92 / -2.49 dB | -0.19 / -0.60 (6.3 GHz) / -2.00 dB, no nickel |

S21 of the FDTD with each loss on its own, 1 / 10 / 40 GHz:

| copper model | S21 | gap to M1 |
|---|---|---|
| smooth copper, no finish (before) | -0.12 / -0.55 / -1.73 dB | 0.34 / 1.38 / 4.27 dB |
| roughness only | -0.20 / -0.85 / -2.41 dB | 0.27 / 1.08 / 3.58 dB |
| ENIG only | -0.19 / -0.64 / -1.92 dB | 0.28 / 1.29 / 4.08 dB |
| roughness and ENIG | -0.27 / -0.92 / -2.49 dB | 0.20 / 1.01 / 3.50 dB |

The roughness and the nickel close 43%, 27% and 18% of the gap at 1, 10 and 40 GHz. The
nickel matters most below its 2.6 GHz resonance, where its permeability is 6 and it takes the
top face current into a thin resistive skin; above it the plated face settles near 3 times
smooth copper and the roughness, which saturates at twice smooth copper under Hammerstad-Jensen,
does most of the work. The added internal inductance raises the impedance by 1 ohm and leaves
the delay alone. The roughness on its own raises the loss by 0.075 dB at 1 GHz in the FDTD and
0.071 dB in the 2D solver, which applies the same Hammerstad-Jensen factor to its resistance.

What remains, 0.20, 1.01 and 3.50 dB, is in the two 1.85 mm edge connectors and the two
adapters (M1's 515 ps group delay is about 180 ps more than the line), and in what the copper
model leaves out: the nickel thickness and its permeability are the published values for another
board, and Hammerstad-Jensen caps the roughness at 2 where Rogers reports it under-predicting
rough foil at high frequencies. The connectors cannot be separated with the published data: the
M9 2x-thru is the two fixtures, which carry their own lines and connectors, and M16 minus M9
reproduces M1 to 0.25 dB. The measured loss already reaches 0.47 dB at 1 GHz, where four coaxial
parts add little, so part of the remaining gap is likely still the line.

The impedance still reads about 1% low and the delay 4% long, both what a lower er gives: with
RO4003C's process er of 3.38 instead of the design er of 3.55 the 2D solver gives 49.0 ohm and
5.35 ps/mm on smooth copper. Rogers quotes both; the measurement sits nearer the lower one.

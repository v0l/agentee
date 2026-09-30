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

| | measured (M1) | agentee FDTD | agentee 2D solver |
|---|---|---|---|
| line impedance (TDR plateau, 350 to 650 ps) | 50.3 ohm | 48.7 ohm | 48.2 ohm at 1 GHz |
| line delay | 321 ps between the two launch reflections | 336 ps port to port | 330 ps (5.49 ps/mm) |
| S21 at 1 / 10 / 40 GHz | -0.47 / -1.93 / -5.99 dB | -0.12 / -0.55 / -1.73 dB | -0.12 / -0.56 dB (1, 11.7 GHz) |

The FDTD and the 2D solver agree with each other on this line to about 1% in impedance and
loss. Against the measurement:

- The impedance reads 3% low and the delay 4% long, both what a lower er gives: with
  RO4003C's process er of 3.38 instead of the design er of 3.55 the 2D solver gives 49.0 ohm and
  5.35 ps/mm. Rogers quotes both; the measurement sits nearer the lower one.
- The measured loss is 3.5 to 4 times the model's. The model has smooth copper and no nickel,
  and the measurement includes two edge connectors and two adapters (its 515 ps group delay is
  about 180 ps more than the line). The loss already reaches 0.47 dB at 1 GHz and grows as
  sqrt(f) from 6 MHz, so most of it is conductor loss, which is where the ENIG nickel and the
  foil roughness act. Neither is modelled, so this data cannot check agentee's loss.

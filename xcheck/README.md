# Cross-checks against openEMS

`cases.json` describes microstrip test structures. `agentee` and [openEMS](https://openems.de)
simulate each one with the same geometry and lumped 50 ohm ports, and `compare.py` puts the two
sets of S-parameters side by side with scikit-rf.

```sh
cargo run --release -p agentee-sim --example xcheck -- xcheck/cases.json target/xcheck
source ~/opt/openEMS/venv/bin/activate          # openEMS built with --python
python xcheck/openems_cases.py xcheck/cases.json target/xcheck
python xcheck/compare.py target/xcheck msl50 msl50_lossy stub stub_fine thin_lossy
```

The openEMS side uses a wider air box (8 mm), PML_8, the thirds rule on the strip edges,
`AddConductingSheet` for lossy copper (frequency dependent) and PEC otherwise.

## Results

| case | what | agentee | openEMS |
|---|---|---|---|
| `msl50` | 30 mm, 2.9 mm on 1.51 mm er 4.5, PEC | S21 within 0.07 dB, phase within 0.5 deg to 6 GHz | |
| `msl50` | power not in S11 or S21 at 5 GHz (radiation) | 0.036 dB | 0.089 dB |
| `msl50_lossy` | same with tan 0.02 and 35 um copper, S21 at 1 / 3 / 5 GHz | -0.315 / -0.331 / -0.363 dB | -0.315 / -0.359 / -0.424 dB |
| `thin_lossy` | 0.3 mm on 0.15 mm air, 35 um copper, loss at 1 / 3 / 5 GHz | 0.042 / 0.048 / 0.052 dB | 0.045 / 0.050 / 0.054 dB |
| `thin_lossy` | the same with the sheet resistance fixed at 3.5 GHz (before `dispersive`) | 0.078 / 0.053 / 0.044 dB | |
| `stub` | 12 mm open stub, notch, 0.2 mm cells | 3.625 GHz | 3.525 GHz |
| `stub_fine` | the same, 0.1 mm cells | 3.675 GHz | 3.625 GHz |

Findings:

- Delay and match on plain lines agree closely. The lossy line's gap grows with frequency by
  about as much as the lossless line's radiation gap, so dielectric and copper loss agree to
  about 0.01 dB. agentee reads less radiation than openEMS; moving agentee's air box out from
  1.5 mm to 8 mm does not change it, so the cause is still open.
- Copper loss first matched only at the band centre, off by sqrt(f) away from it, because the
  FDTD sheet resistance was fixed there while openEMS's sheet model is dispersive. The sheets
  now carry the full sqrt(j w) surface impedance and agree within 7% across the band.
- On the 0.3 mm line S21 differs by up to 0.15 dB while the loss agrees, and S11 differs by
  0.5 dB: the codes see a different line impedance on a strip six cells wide. Not resolved.
- The first stub run put the notch 7% low: track ends and pour edges were not mesh lines, so an
  open stub's length snapped to a coarse cell. They are mesh lines now. The remaining 1.4% is
  within how far openEMS itself moves with its mesh (3.70, 3.525, 3.625 GHz).

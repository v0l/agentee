# Cross-checks against openEMS

`cases.json` describes microstrip test structures. `agentee` and [openEMS](https://openems.de)
simulate each one with the same geometry and lumped 50 ohm ports, and `compare.py` puts the two
sets of S-parameters side by side with scikit-rf.

```sh
cargo run --release -p agentee-sim --example xcheck -- xcheck/cases.json target/xcheck
source ~/opt/openEMS/venv/bin/activate          # openEMS built with --python
python xcheck/openems_cases.py xcheck/cases.json target/xcheck
python xcheck/compare.py target/xcheck msl50 msl50_lossy stub stub_fine thin_lossy via
```

`openems_cases.py` takes the engine from `XCHECK_ENGINE` (e.g. `gpu` for the GPU engine of openEMS PR
225, `multithreaded`), `XCHECK_EXACT=1` evaluates the end criteria every Nyquist period (GPU branch
only), and `XCHECK_TAG` names the results (`<case>.<tag>.s2p`, default `openems`); `compare.py`
compares the tags in `XCHECK_A` and `XCHECK_B` (default `agentee` and `openems`).

Engine throughput on the free-space grid of openEMS's `FreeSpace_Benchmark.py` (n^3 cells of 1 mm,
PML 8 or PEC walls), the difference between two step counts giving the cost per step without the
setup of a run:

```sh
./target/release/examples/xcheck throughput 300 8 3 200 800   # n, pml (0: PEC), repeats, steps...
python xcheck/openems_freespace.py gpu 300 800 PML_8 target/xcheck [noprobe]
```

The openEMS side uses a wider air box (8 mm), PML_8, the thirds rule on the strip edges,
`AddConductingSheet` for lossy copper (frequency dependent) and PEC otherwise. Lossy substrates
are an openEMS Debye material with the same poles agentee uses for its Djordjevic-Sarkar
dielectric (tan and er given at 1 GHz, poles from f_start / 30 to 30 f_stop, fitted at the
geometric mean of the band); the script checks the pole sum against Djordjevic-Sarkar before it
runs.

## Results

| case | what | agentee | openEMS |
|---|---|---|---|
| `msl50` | 30 mm, 2.9 mm on 1.51 mm er 4.5, PEC | S21 within 0.08 dB, phase within 1.3 deg to 6 GHz | |
| `msl50` | power not in S11 or S21 at 5 GHz (radiation) | 0.032 dB | 0.089 dB |
| `msl50` | the same, square strip ends and the port current over the whole port height | 0.067 dB | 0.089 dB |
| `msl50` | Z0 at 1.5 / 3.5 GHz from a 30 and a 45 mm line, 0.2 and 0.1 mm cells | 50.0 / 50.3, 49.9 / 50.3 ohm | 48.8 / 49.4, 48.9 / 49.5 ohm |
| `msl50_lossy` | same with tan 0.02 and 35 um copper, S21 at 1 / 3 / 5 GHz | -0.304 / -0.329 / -0.368 dB | -0.315 / -0.359 / -0.424 dB |
| `msl50_lossy` | agentee Djordjevic-Sarkar, openEMS conductivity still fixed at 3.25 GHz | -0.103 / -0.307 / -0.535 dB | -0.315 / -0.359 / -0.424 dB |
| `msl50_lossy` | Djordjevic-Sarkar on both sides, the same 15 Debye poles | -0.103 / -0.307 / -0.535 dB | -0.115 / -0.353 / -0.617 dB |
| `msl50_lossy` | the same, openEMS Debye box pulled 0.1 um below the substrate top | | -0.102 / -0.317 / -0.557 dB |
| `thin_lossy` | 0.3 mm on 0.15 mm air, 35 um copper, loss at 1 / 3 / 5 GHz | 0.041 / 0.048 / 0.052 dB | 0.045 / 0.050 / 0.054 dB |
| `thin_lossy` | the same, earlier model with the sheet resistance fixed at 3.5 GHz | 0.078 / 0.053 / 0.044 dB | |
| `stub` | 12 mm open stub, notch, 0.2 mm cells | 3.725 GHz | 3.525 GHz |
| `stub_fine` | the same, 0.1 mm cells | 3.725 GHz | 3.625 GHz |
| `via` | 20 mm, 0.95 mm strips on F.Cu and B.Cu, In1.Cu plane, 2 x 0.5 mm er 4.5, 0.3 mm via, S21 at 3 / 5 GHz | -0.003 / -0.024 dB, -135.5 / 133.8 deg | -0.007 / -0.059 dB, -137.1 / 131.3 deg |
| `via` | the same, the via as one line of edges and port 2 read upside down | -0.037 / -0.116 dB, 180 deg off | |

Hammerstad-Jensen gives 49.4 ohm for `msl50` on an infinite substrate at DC.

Findings:

- Both sides now model the substrate as Djordjevic-Sarkar with the same 15 Debye poles
  (16.7 MHz to 180 GHz). Against Djordjevic-Sarkar the pole sum is within 0.001 in er and 1.3%
  in tan from 0.5 to 6 GHz. With openEMS's conductivity fixed at the band centre, S21 differed by
  up to 0.26 dB and 5.3 deg; with the Debye material the gap is 0.012 / 0.046 / 0.082 dB at
  1 / 3 / 5 GHz and the phase is within 1.5 deg.
- Part of that gap is the radiation gap the lossless line already shows, and most of the rest
  comes from how openEMS places the poles. It gives an edge the Debye branch of
  whatever material sits at the edge's position, with no averaging, so edges on the substrate
  top, half in air, get the whole pole; its plain epsilon and conductivity are averaged over
  the cells around the edge, and agentee averages the poles the same way. Pulling the Debye box
  0.1 um below the surface gives those edges no pole instead, and S21 moves by 0.013 / 0.036 /
  0.060 dB. Halfway between the two runs, which approximates the averaged pole, the gap to
  agentee is 0.005 / 0.028 / 0.053 dB, within 0.004 dB of the lossless line's radiation gap
  (0.004 / 0.025 / 0.049 dB). Dielectric and copper loss therefore agree to about 0.004 dB.
- Delay on plain lines agrees closely. agentee read less radiation than openEMS; moving
  agentee's air box out from 1.5 mm to 8 mm, doubling its PML to 16 cells or capping its
  cells at 0.4 mm does not change it. Two causes were found. The strips here were tracks with
  round ends reaching 1.45 mm past the ports and off the board edge, where openEMS has square
  ends at the ports; they are boxes now. And the port current was read on one loop at the
  middle edge of the port while the voltage sums every edge; it is now the length weighted mean
  of a loop round every edge. The current moved the 5 GHz figure by 0.022 dB and the ends by
  0.013 dB, together 0.032 to 0.067 dB against openEMS's 0.089 dB (0.034 against 0.037 dB at
  3 GHz).
- The first via run read S21 180 deg off (a port whose reference plane is above it measured
  reference minus signal) and put the via in as one line of edges, which added about 0.1 dB of
  mismatch loss at 5 GHz. Every node inside the drill is PEC now.
- Copper loss first matched only at the band centre, off by sqrt(f) away from it, because the
  FDTD sheet resistance was fixed there while openEMS's sheet model is dispersive. The sheets
  now carry the full sqrt(j w) surface impedance and agree within 8% across the band.
- The first stub run put the notch 7% low: track ends and pour edges were not mesh lines, so an
  open stub's length snapped to a coarse cell. They are mesh lines now.
- agentee's edges now sit where the grid's effective edge meets the copper edge, which makes
  its impedance and the stub notch independent of the cell size (49.9 to 50.0 ohm and 3.725 GHz
  at both cells). openEMS still moves with its mesh (notch 3.70, 3.525, 3.625 GHz) and reads
  the 50 ohm line 2% lower than agentee and 1% under Hammerstad-Jensen. Which of the two is
  closer on this line is not settled.
- `thin_lossy` S11 and S21 differ by 0.7 dB and 0.2 dB because agentee widens the strip for its
  35 um thickness and openEMS's conducting sheet has none.

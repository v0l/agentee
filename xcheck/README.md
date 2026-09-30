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

`via.py` takes the via on its own: `via_line` (the 20 mm line, no via) as an IEEE P370 2x-thru
(scikit-rf's NZC de-embedding), the result renormalised to the line Z0 from `via_line` and
`via_line30`, and the series L and shunt C of its ABCD matrix:

```sh
python xcheck/via.py target/xcheck via via_line via_line30 agentee,openems 1,3,5
```

`z0.py` takes Z0 and the effective permittivity of a line from two lengths of it (the eigenvectors
and eigenvalues of one line's ABCD matrix times the inverse of the other's; the geometric mean of
the forward and backward wave impedances cancels a port's series or shunt error to first order):

```sh
python xcheck/z0.py target/xcheck msl50 msl50_45 agentee,openems 1.5,3.5,5
```

`openems_cases.py` takes the engine from `XCHECK_ENGINE` (e.g. `gpu` for the GPU engine of openEMS PR
225, `multithreaded`), `XCHECK_EXACT=1` evaluates the end criteria every Nyquist period (GPU branch
only), and `XCHECK_TAG` names the results (`<case>.<tag>.s2p`, default `openems`), `XCHECK_SHEET_DZ` adds
mesh lines that far above and below each copper plane (mm), `XCHECK_FINE` caps the cells at that
size (mm) across the strip and 0.2 mm past it in y, and over the same span round the via in x, with
the thirds rule on that size; `compare.py`
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
| `msl50` | Z0 at 1.5 / 3.5 GHz from `msl50` and `msl50_45` (`z0.py`), 0.2 and 0.1 mm cells | 50.0 / 50.7, 50.0 / 50.7 ohm | 48.8 / 49.5, 48.9 / 49.5 ohm |
| `msl50` | the same, openEMS with z cells of 0.05 / 0.02 mm beside the copper (`XCHECK_SHEET_DZ`) | | 49.4 / 50.1, 49.3 / 50.0 ohm |
| `msl50` | eeff at 1.5 GHz from the same pair | 3.487 | 3.487 |
| line | 0.95 mm on 0.5 mm er 4.5 (the `via` strips), Z0 at 1.5 GHz from 20 and 30 mm, 0.1 mm cells | 49.5 ohm | 48.2 ohm |
| line | the same (`via_line`, `via_line30`), agentee 0.05 / 0.025 mm cells | 49.56 / 49.62 ohm | |
| line | the same, openEMS z cells 0.05 / 0.02 mm, then z 0.02 mm with `XCHECK_FINE` 0.05 / 0.025 mm | | 48.69 / 48.86, 49.08 / 49.22 ohm |
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
| `via` | S11 at 1 / 3 / 5 GHz, 0.1 mm cells | -57.1 / -37.3 / -29.9 dB | -33.7 / -41.0 / -21.8 dB |
| `via` | the same, openEMS with z cells of 0.02 mm beside the copper | | -38.9 / -35.0 / -22.8 dB |
| `via` | S11 renormalised to each side's own line Z0 (49.5 / 48.2 ohm), 1 / 3 / 5 GHz | -43.9 / -33.7 / -32.1 dB | -45.5 / -29.7 / -25.2 dB |
| `via` | agentee S11 at 5 GHz, cells 0.1 / 0.05 / 0.025 / 0.0125 mm, strips run 0.05 mm past the port centres | -29.9 / -30.6 / -27.1 / -28.5 dB | |
| `via` | agentee S11 at 5 GHz, 0.1 mm cells with mesh lines at the drill edge (x and y) | -25.1 dB | |
| `via` | agentee S11 at 5 GHz, cells 0.1 / 0.05 / 0.025 / 0.0125 mm, port columns on the copper only, square strip ends at the ports | -29.9 / -32.6 / -28.3 / -29.9 dB | |
| `via` | the same with mesh lines at the drill, pad and antipad edges (from 0.05 mm) | -29.9 / -31.5 / -30.5 / -31.1 dB | |
| `via` | the same lines at 0.1 mm too, pad and antipad lines moved like strip edges / not moved | -27.5 / -25.5 dB | |
| `via` | S11 at 1 / 3 / 5 GHz, openEMS z cells 0.05 mm | | -37.1 / -36.2 / -22.5 dB |
| `via` | the same, z 0.02 mm and `XCHECK_FINE` 0.05 / 0.025 mm; agentee 0.025 mm | -56.0 / -35.2 / -30.5 dB | -41.3 / -37.6 / -26.2, -42.2 / -38.2 / -27.1 dB |
| `via` | S21 at 3 / 5 GHz, the same runs | -0.006 / -0.029 dB, -135.1 / 134.5 deg | -0.010 / -0.042 dB, -135.8 / 133.4 deg |
| `via` | S11 renormalised to each side's line Z0, openEMS `XCHECK_FINE` 0.05 / 0.025 mm | | -46.2 / -31.8 / -28.9, -48.4 / -32.8 / -29.7 dB |
| `via` | the via alone (`via.py`), 1 / 3 / 5 GHz, agentee 0.1 / 0.05 / 0.025 mm | -43.6 / -34.9 / -32.0, -41.1 / -34.9 / -34.7, -41.6 / -35.5 / -34.8 dB | |
| `via` | the via alone, openEMS base / z 0.02 mm / z 0.02 and `XCHECK_FINE` 0.05 / 0.025 mm | | -42.9 / -31.6 / -26.5, -42.5 / -31.6 / -26.7, -43.3 / -34.5 / -31.6, -45.0 / -35.9 / -32.5 dB |
| `via` | the via alone, series L and shunt C at 0.5 / 5 GHz, finest runs | 339 / 288 pH, 79 / 92 fF | 294 / 283 pH, 85 / 86 fF |
| `via` | agentee via alone with `via_line` and `via_line30` on the via case's own y and z lines (x too for `via_line`), 0.1 / 0.05 / 0.025 mm | -43.6 / -34.4 / -31.1, -44.3 / -35.2 / -31.9, -44.0 / -34.9 / -31.7 dB | |
| `via` | the same, series L and shunt C at 0.5 / 5 GHz | 333 / 324, 315 / 305, 311 / 300 pH; 93 / 94, 88 / 89, 85 / 86 fF | |

Hammerstad-Jensen gives 49.4 ohm for `msl50` on an infinite substrate at DC. agentee's 2D field
solver (`xsection`) on the cross-section as simulated, a 12 mm board and ground, gives 49.8 ohm
and eeff 3.454 at DC, and 49.4 to 49.5 ohm for the 0.95 mm line on its 10 mm board.

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
  at both cells). openEMS still moves with its mesh (notch 3.70, 3.525, 3.625 GHz).
- The 50 ohm line reads 50.0 ohm in agentee and 48.8 in openEMS at 1.5 GHz. Taken from two line
  lengths, the ports drop out (forward and backward wave impedances agree within 0.03 ohm on both
  sides), and eeff agrees to 0.01%, so the dielectric at the interface is averaged alike; the
  difference is in L and C together, as from a wider strip. It is openEMS's mesh at the
  zero-thickness strip: its z cells beside the copper are 0.25 mm, and cutting them to 0.05 or
  0.02 mm moves its Z0 up by 0.5 to 0.6 ohm, while agentee, which places its strip edges for the
  cells around them, reads the same at 0.2 and 0.1 mm and sits within 0.5% of the 2D solver.
  The last 0.6 ohm of openEMS's gap is not traced; dropping its mesh line on the strip edge,
  keeping only the thirds lines, moved Z0 by 0.1 ohm but eeff by 8%, so that run was not used.
  Nothing changed on the agentee side.
- The via's S11 at 5 GHz (agentee -29.9, openEMS -21.8 dB) is two things. openEMS's strips read
  48.2 ohm against agentee's 49.5 and the 2D solver's 49.4 to 49.5, the same z mesh issue as
  `msl50`, and that mismatch rides on S11 as a ripple (-33.7 dB at 1 GHz, where agentee reads
  -57 dB) that adds in phase at 5 GHz. Renormalised to each side's own line impedance, both read
  -44 to -46 dB at 1 GHz, and at 5 GHz the via itself reflects -25.2 dB in openEMS and -32.1 dB in
  agentee. That remainder is how each mesh sees the barrel, pad and antipad: agentee at finer
  cells reads -27 to -30.6 dB (-28.3 to -29.9 renormalised), and at 0.1 mm the same model moves
  from -29.9 to -25.1 dB when mesh lines are put on the drill edge, while openEMS has lines on
  the drill and antipad edges and 0.15 mm cells there. The 2D Laplace equivalent radius of the
  drill's node set is 0.125 mm for agentee's 3 x 3 block at 0.1 mm cells and 0.141 mm for
  openEMS's cross of five nodes (drill radius 0.15 mm), which does not order the two results,
  so the barrel radius alone is not the cause. With the port columns fixed (below) and square
  strip ends, the fine runs read -28.3 to -32.6 dB, and the 0.1 mm run sits inside that spread.
  Mesh lines at the drill, pad and antipad edges bring the 0.05 to 0.0125 mm runs within 1 dB
  of each other (-30.5 to -31.5 dB, largest S11 difference to the finest run over the band
  0.0048 against 0.0090 without them), but at 0.1 mm they put lines inside the 0.95 mm strip that
  crosses the via, its edge cells stop being a whole cell, and S11 moves away from the fine runs
  (0.019 against 0.007 largest difference, -45 dB at 1 GHz where the line alone reads -57 dB).
  The lines are therefore only added for a drill radius of at least two cells.
- The via gap was openEMS's mesh, and the renormalised S11 overstated it. Refining openEMS in z
  alone (0.05, 0.02 mm beside the copper) lifts its line Z0 from 48.2 to 48.9 ohm but leaves the
  via where it was (-25.2 dB renormalised at 5 GHz). Cells of 0.05 and 0.025 mm across the strips
  and round the via (`XCHECK_FINE`, 6.8 and 8.9 M cells) move it to -28.9 and -29.7 dB and its
  line Z0 to 49.1 and 49.2 ohm, towards agentee's 49.6. The renormalised figure still carries
  each side's ports: the 20 mm line alone, renormalised the same way, reads -38 dB at 5 GHz in
  openEMS at every mesh and -42 to -48 dB in agentee. `via.py` removes them by de-embedding
  `via_line` as a 2x-thru, which puts both reference planes on the via axis. The via alone then
  reads -26.5 dB in openEMS at its first mesh, -31.6 and -32.5 dB at 0.05 and 0.025 mm, and
  -32.0 to -34.8 dB in agentee at 0.1 to 0.025 mm; at 1 and 3 GHz the finest runs read -45.0 /
  -35.9 dB and -41.6 / -35.5 dB.
- The agentee via-alone figures from 0.05 mm down were not flat in frequency: series L fell from
  339 to 288 pH and shunt C rose from 79 to 92 fF over 0.5 to 5 GHz, where a 1 mm structure
  should hold within about 2%. That is the de-embedding, not the via: agentee's mesh lines at the
  drill, pad and antipad edges run the length of the board, so the strips in `via` sit on other
  y lines than `via_line`, the two Z0 differ, and the 9.5 mm fixtures do not cancel. With
  `via_line` and `via_line30` meshed on the via case's own lines (a scratch override of the
  mesh, not in the tree), L and C hold within 4% (311 to 300 pH, 85 to 86 fF at 0.025 mm) and
  the via alone reads -44.0 / -34.9 / -31.7 dB at 1 / 3 / 5 GHz, and -31.1 to -31.9 dB at 5 GHz
  from 0.1 to 0.025 mm cells. openEMS's first mesh has the same flaw the other way (L rising
  from 352 to 368 pH); at 0.025 mm its L and C hold within 4% (294 to 283 pH, 85 to 86 fF).
- The two tools now agree on the via to 0.8 dB (agentee -31.7, openEMS -32.5 dB at 5 GHz), the
  same shunt C (85 to 86 fF) and a series L within 6% (300 to 311 against 283 to 294 pH).
  agentee moves by 5 pH from 0.05 to 0.025 mm and openEMS by 15 to 20 pH, still falling, so the
  0.8 dB is inside openEMS's own convergence. S21 at 5 GHz is -0.029 against -0.042 dB and
  134.5 against 133.4 deg (openEMS -0.059 dB, 131.3 deg at its first mesh). In agentee the drill
  as a filled node set against one whose inner x and y edges are PEC too (a hollow plated tube and
  a filled drill are the same to a PEC FDTD, only the inner edges differ) moves the via alone by
  0.5 dB at most, and pad and antipad edges left where they are instead of moved like strip edges
  by 0.8 dB at most, at 0.05 and 0.025 mm. Nothing changed in the agentee model.
- Against closed forms: Goldfarb and Pucel give 80 pH for a 0.5 mm barrel of 0.15 mm radius over
  its plane, 161 pH for the two halves, which alone would reflect wL / 2 Z0, -26 dB at 5 GHz. The
  de-embedded via carries about 300 pH and 85 fF: the barrel plus the strips' last 0.6 mm over the
  antipad hole in L, and the pad and barrel to the antipad edge less what those strips lose over
  the hole in C. As a lumped T, (L / Z0 - C Z0) / 2 is 0.88 ps in agentee and 0.74 ps in openEMS,
  -31.2 and -32.7 dB at 5 GHz, matching the de-embedded S11. Closed forms for a strip over a hole
  and for the fringe C are not good to the 5% that separates the tools, so they bound the via
  (below the -26 dB of the barrel alone) rather than pick a side.
- At cells finer than 0.05 mm the xcheck ports (0.1 mm long, centred on the strip end) got node
  columns off the copper, and a 20 mm line read S11 near -13 dB. A port now keeps only the node
  columns that touch copper on its sheet; the same line at 0.025 mm reads -43.4 / -54.4 / -38.6
  dB at 1 / 3 / 5 GHz.
- `thin_lossy` S11 and S21 differ by 0.7 dB and 0.2 dB because agentee widens the strip for its
  35 um thickness and openEMS's conducting sheet has none.

# ladder-vcf

A 12HP Eurorack voltage-controlled filter built around the transistor ladder from the old Moog
modules: four poles at 24 dB per octave, cutoff at 1 V/oct, and resonance that runs up to
self-oscillation. Every part is through-hole. The pots and jacks mount on the back of the
58 x 108 mm board and come through the panel; everything else sits on the front.

The top schematic `ladder-vcf.sch.toml` joins four sheets: `power`, `cv`, `ladder` and
`output`. The layout is `ladder-vcf.pcb.toml`, the board spec `ladder-vcf.board.toml`, and the
sims are `vcf-*.sim.toml`. All of it passes `agentee check`.

## Panel

| control | part | does |
|---|---|---|
| CUTOFF | RV1 | 0 to 10 octaves of base cutoff, 57.6 Hz to about 61 kHz |
| RES | RV2 | Resonance; by the loop gain sum it self-oscillates past about 0.77 of its travel |
| FM | RV3 | Attenuator on the FM jack |
| V/OCT | J2 | 1 V/oct cutoff CV, trimmed with RV4 |
| FM | J3 | Second CV input, through RV3 |
| IN | J4 | Audio in, 10 Vpp |
| OUT | J5 | Audio out, through 1k |

## Circuit

**Power** (`power.sch.toml`): J1 is the standard 10-pin Eurorack header, with -12 V on pins
1-2 (the red stripe) and +12 V on pins 9-10. D1 and D2 are series Schottkys, so a ribbon cable
plugged in backwards conducts nothing.

**Expo converter** (`cv.sch.toml`):
- **Summer:** U1A sums V/OCT (R1), FM (R2) and CUTOFF (R3) inverting, at -(91k + RV4)/100k per
  volt. R5 and R6 divide that down to about 17.9 mV per volt at Q1's base, which is
  Vt x ln 2 at 27 C.
- **Reference:** U1B holds Q1's collector at 0 V, so Q1 carries the 2.55 uA that R7 (4.7M)
  draws from +12 V.
- **Tail current:** Q2 shares Q1's emitter, so it sinks 2.55 uA x 2^CV from the ladder.
  Q1 and Q2 are a hand-matched BC550C pair, placed flat face to flat face to keep them at one
  temperature.
- **Limit:** the summer clips at about -10.5 V, which caps the tail current near 3.3 mA.

**Ladder** (`ladder.sch.toml`):
- **Input pair:** Q3 and Q4 split TAIL between the two sides.
- **Rungs:** four rungs (Q5-Q12) stack above them, each pair's bases tied to one tap of the
  R9-R14 bias string (V0 1.48 V up to V4 7.38 V, 1.5 V a step, each tap held by 10 uF).
  The 68 nF across each rung's emitters makes one pole at Ic/(8 pi Vt C) = Ic x 22.6 MHz/A.
  The top rung's collectors go straight to +12 V.
- **Audio in:** comes from J4 through R17 (100k), C19 and R15 (1k to V0), about 50 mV peak
  for 10 Vpp.
- **Resonance:** OUT_PRE comes back through RV2, R18 (18.2k), C20 and R16 into Q4's base, the
  inverting side.

**Output** (`output.sch.toml`):
- **Buffers:** U2A and U2B are JFET followers on the top rung's emitters, LAD_L4 and LAD_R4.
  Their picoamp bias current doesn't load even the smallest tail currents.
- **Gain:** U3A takes the difference x10 and U3B inverts it x10, so the passband gain from IN
  to OUT is close to 1. That keeps OUT in phase with the input and makes RV2's feedback
  negative.
- **Protection:** R25 isolates the jack.

## Parts

| ref | part | why |
|---|---|---|
| Q1-Q12 | onsemi BC550CTA, TO-92 lead-formed | Low noise, hFE 420-800. The lead-formed version has 2.54 mm pitch for hand soldering (case 135AR) |
| U1-U3 | TI TL072IP, DIP-8 | JFET input, so the picoamp bias current doesn't load the ladder. GBW 5.25 MHz |
| D1, D2 | Vishay 1N5819-E3/54 | 40 V Schottky, for a low drop on each rail |
| J1 | Molex 90130-1210 | Shrouded and polarized 2x5 header. Uses KiCad's IDC-Header footprint, the same 2.54 mm pin grid |
| J2-J5 | Thonkiconn PJ398SM | The Eurorack standard jack. Sold by Thonk only; the datasheet is Thonk's drawing |
| RV1-RV3 | Bourns PTV09A-4020F-B104 | 9 mm, 100k linear, rear mount |
| RV4 | Bourns 3296W-1-203LF | 25-turn 20k trimmer for the V/oct scale |
| C10-C13 | KEMET R82 68n PET film | The ladder poles. Film for low dielectric absorption and drift |
| C19, C20 | KEMET R82 1u PET film | Coupling into the ladder |
| R | Yageo MFR-25 1% metal film | |

## Simulation

The models in every sim come from datasheets:

- **TL072:** TI SLOS080W tables 5.8 and 5.9, typ: AOL 200 V/mV, GBW 5.25 MHz, en 37 nV/rtHz
  taken as flat, VOS 3 mV. Headroom is 1.5 V, from the ±13.5 V swing at RL 10 kohm.
- **BC550C:** onsemi BC550/D. Is 16.4 fA, fitted to VBE(on) 660 mV at 2 mA; the same value
  also lands on figure 2's 0.58 V at 0.1 mA. Bf is 420, the C group minimum.
- **Rails:** driven at ±12 V, with D1 and D2 left out.
- **Pots:** RV1 is a drive on CUTOFF. RV2 and RV3 are ideal dividers, since a vcvs block has
  no wiper resistance.
- **Trimmer:** RV4 is set to 10.45k. That's the bench trim done in the sim: at that value
  6 V of CV gives 64x the tail current.

| sim | shows | result |
|---|---|---|
| `vcf-ac-0v` | Cutoff at CV 0 V | -12.4 dB at 57.6 Hz, -80.7 dB a decade above |
| `vcf-ac-3v` | Bias points and cutoff at CV 3 V | V0 1.48 V, V4 7.38 V, TAIL 0.95 V, passband -0.08 dB, -12.4 dB at 461 Hz (8x), -80.7 dB at 4.61 kHz |
| `vcf-ac-6v` | Cutoff at CV 6 V | -12.4 dB at 3.69 kHz (64x) |
| `vcf-res` | Resonance at 0.6, CV 3 V | Passband drops to -11.7 dB and a peak rises to +0.1 dB at fc. That's the Moog bass loss |
| `vcf-osc` | Resonance at full, 60 ms transient after a kick | Oscillates at 441 Hz, about 1.1 V peak at OUT |

Four equal poles are each 3 dB down at fc, so the filter is 12 dB down there. The three AC sims
land within 0.07 dB of each other at 57.6 Hz, 461 Hz and 3.69 kHz, so the expo tracks 1 V/oct
across six octaves.

## Layout

- **Panel parts:** the pots and jacks are locked on B with their shafts and barrels on a
  12HP grid. J1 is locked on F between the lower jacks, with its pins facing back.
- **Ladder:** locked as two columns of rotated TO-92s with the four film caps between them,
  so each rung's wiring stays a few millimetres long.
- **Expo pair:** Q1 and Q2 are locked side by side. D1 and D2 flank J1.
- **Engine:** `agentee place` then `agentee layout ladder-vcf --search 32 --keep 8` placed and
  routed the rest. One +12 V track was straightened by hand to clear an acute-turn warning.
  B.Cu is a GND pour.

## Known problems

- **Untrimmed offsets:** an op-amp offset and Q3/Q4 mismatch reach the output amplified about
  100x. With the 3 mV VOS of each TL072 the sim shows -0.3 V DC at OUT, and real parts will
  differ. A real build wants matched Q3/Q4 and an offset trim, or AC coupling at J5.
- **Temperature:** the expo isn't temperature compensated, so tracking drifts about 0.3% per
  kelvin.
- **Decoupling:** the 100n caps sit 7 to 14 mm from their op-amp pins. `agentee drc` notes
  `placement-decoupling-distance`.
- **Unverified:**
  - Self-oscillation amplitude depends on Q3/Q4 matching and real hFE. The sim has identical
    transistors.
  - The panel drilling isn't designed here.

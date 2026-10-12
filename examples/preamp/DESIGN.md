# preamp

A small-signal audio amplifier on a 9 V battery: 3.5 mm jack in, x11 (20.8 dB) gain, a
second-order Butterworth low-pass at 1 kHz, 3.5 mm jack out. Every part is through-hole, on a
60 x 40 mm two-layer board. It exists to exercise the circuit sim on an op-amp design: operating
point, AC, noise and transient, each with checks.

The schematic is `preamp.sch.toml`, the layout `preamp.pcb.toml`, the board spec
`preamp.board.toml`, and the sims `preamp-*.sim.toml`. All of it passes `agentee check`.

## Circuit

| net | connects | class |
|---|---|---|
| VBAT | J1.1, D1 anode | Power |
| VCC | D1 cathode, C1+, C2, R1, U1.8 | Power |
| VREF | R1, R2, C3+, R3 | Signal |
| IN | J2 tip, C4 | Signal |
| IN_B | C4, R3, U1.3 | Signal |
| FB | U1.2, R4, R5 | Signal |
| FB_AC | R5, C5+ | Signal |
| GAIN_OUT | U1.1, R4, R6 | Signal |
| SK1 | R6, R7, C6 | Signal |
| SK2 | R7, C7, U1.5 | Signal |
| FILT_OUT | U1.7, U1.6, C6, C8+ | Signal |
| OUT | C8-, R8, J3 tip and ring | Signal |
| GND | J1.2, J2 and J3 sleeve, C1-, C2, C3-, C5-, C7, R2, R8, U1.4 | Ground |

Power: D1 blocks a battery clip put on backwards. C1 is the bulk cap and C2 decouples U1.

Bias: R1 and R2 split VCC in half and C3 holds it. With D1's drop, VREF sits at 4.2 V.

Gain stage, U1A: the input is AC coupled by C4 and biased to VREF through R3 (HPF corner 1.6 Hz).
R4 and R5 set the gain to 1 + 100k/10k = 11. C5 keeps the DC gain at 1, so the output sits at
VREF with no trimming.

Filter, U1B: a unity-gain Sallen-Key, R6 = R7 = 10k, C6 = 22n, C7 = 10n. That gives
fc = 1/(2 pi R sqrt(C6 C7)) = 1.07 kHz and Q = sqrt(C6/C7)/2 = 0.74, close to Butterworth.

Output: C8 blocks the DC and R8 holds the jack side at 0 V. J3's tip and ring both carry the
signal, so a stereo plug hears it on both channels.

## Parts

Every part is in the parts library, stocked at Mouser or Farnell when added.

| ref | part | why |
|---|---|---|
| U1 | TI LM358P, DIP-8 | Its input range and output swing reach the negative rail, so it works from one 9 V battery. TI SLOS068AB: GBW 0.7 MHz, AOL 100 V/mV, en 40 nV/rtHz, IQ 350 uA per amplifier |
| D1 | onsemi 1N4148TR, DO-35 | IR 25 nA max at 20 V, so a reversed battery leaves only nanoamps in a 200k load. The 1N5817 I tried first leaks up to 1 mA, enough to pull the rail several volts negative |
| R1-R8 | Yageo MFR-25FBF52, 1/4 W 1% metal film, axial 10.16 mm | |
| C4, C6, C7 | KEMET R82 PET film, 5 mm pitch | Film for the coupling and filter caps: no piezo or DC bias effects in the audio band |
| C2 | KEMET C320C104K5R5TA, X7R radial | |
| C1, C3, C5, C8 | KEMET ESK electrolytic | 25 V and 50 V parts on a 9 V rail |
| J2, J3 | Same Sky SJ1-3523N | |
| J1 | JST B2B-PH-K-S | For a 9 V snap with a PH plug |

SJ1-3523N footprint: KiCad's footprint has the slot sizes from before Same Sky re-engineered the
jack in 2022 (rev 1.05). Those slots are 0.4 mm, which the fab can't drill. The current
datasheet (rev 1.06) asks for 1.5 x 0.8 mm. The project footprint uses 1.5 x 0.7 mm, because a
slot shorter than twice its width gets drilled as a round hole. The pad positions match the
datasheet.

## Simulation

The U1 and D1 models in every sim come from their datasheets:

- **U1:** AOL 100 V/mV, GBW 0.7 MHz, en 40 nV/rtHz (taken as flat), VOS 3 mV. Its headroom is
  1.5 V, the worst positive swing at RL >= 2 kohm. The model applies the same headroom to both
  rails, so the bottom clip is pessimistic: the real part swings to within millivolts of ground.
- **D1:** fitted to the typical curve in onsemi 1N914/D figure 4: 500 mV at 0.1 mA, 617 mV at
  1 mA (n 1.97, Is 5.6 nA).

| sim | shows | result |
|---|---|---|
| `preamp-ac` | Operating point, gain and phase from a 100 ohm source, noise from 20 Hz to 20 kHz | VCC 8.40 V, VREF 4.20 V. Gain 20.8 dB at 100 Hz, 18.8 dB at 1 kHz, -18.1 dB at 10 kHz. 374 nV/rtHz at the output at 1 kHz, 17.5 uV RMS over the band |
| `preamp-power` | Battery switched on, 8 s transient | GAIN_OUT clips at 7.0 V while C5 charges. OUT swings 6.1 V at 0.2 s, then settles to 0 V within about 2 s |
| `preamp-clip` | 1 kHz, 400 mV peak sine | Clips at 6.9 V and 1.5 V. The most the input passes cleanly is 245 mV peak |
| `preamp-reverse` | Battery reversed | VCC -1 mV, D1 leaks 5.6 nA |

Above 100 kHz the gain flattens at about -60 dB instead of falling. Signal leaks forward through
C6 as U1B's loop gain runs out, which a real Sallen-Key filter also does.

## Layout

The jacks are locked on the left and right edges, with their front faces on the board edge so
the bushings stand proud of it. J1, D1, C1 and C2 are locked along the top. The engine
(`agentee layout preamp --search 40 --keep 8`) places and routes the rest. B.Cu is a GND pour.

## Known problems

- **Turn-on thump:** switching on puts a 6 V swell on the output jack. Fixes would be a larger
  R8 with a slower start, or a muting FET.
- **Clip level:** at gain 11 the input clips at 173 mV RMS, below consumer line level. This
  board suits sources around 10 to 100 mV.
- **Unverified:** the simulation leaves out slew rate (0.3 V/us), input bias current and PSRR.

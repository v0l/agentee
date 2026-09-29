# lna

A wideband LNA to sit at the antenna end of an RTL-SDR style receiver: SMA in, SMA out,
SPF5189Z in the middle, powered either over the output coax by the receiver's bias tee or from a
5 V header. 50 MHz to 4 GHz, about 19 dB of gain at 900 MHz with a 0.55 dB noise figure
(SPF5189Z datasheet, 5 V, 90 mA).

The schematic is `lna.sch.toml`, the layout `lna.pcb.toml`, the board spec `lna.board.toml`, and
every part is under `symbols/` and `footprints/`. All of it passes `agentee check`.

## Circuit

| net | connects | class |
|---|---|---|
| RF_IN | J1.1, D3.1, C1.1 | RF |
| RF_AMP_IN | C1.2, U1.1 | RF |
| RF_AMP_OUT | U1.3, C2.1, L1.1 | RF |
| RF_OUT | C2.2, J2.1, L3.1 | RF |
| CHOKE_A | L1.2, L2.1 | Default |
| CHOKE_B | L3.2, L4.1 | Default |
| VBIAS | L4.2, C5.1, D1.2 (anode) | Power |
| VEXT | J3.1, D2.2 (anode) | Power |
| VCC | D1.1, D2.1 (cathodes), L2.2, C3.1, C4.1, C6.1, R1.1 | Power |
| LED_A | R1.2, D4.2 (anode) | Default |
| GND | U1.2 (pin and tab), J1.2, J2.2, J3.2, D3.2, C3.2, C4.2, C5.2, C6.2, D4.1 | |

Signal path: J1, ESD clamp D3, DC block C1, U1, DC block C2, J2.

Bias: U1 draws its drain current through pin 3. VCC reaches pin 3 through L2 then L1. L1 is
the 47 nH choke from Qorvo's 1.9 GHz evaluation board and sits at the RF node, where it has to
stay inductive up to 4 GHz. L2 is the 150 nH choke from their 900 MHz board and adds the
reactance L1 lacks down at VHF.

Power in: the receiver's bias tee puts DC on J2's centre pin. L3 and L4 (the same pair) lift it
off the RF line into VBIAS, and D1 carries it to VCC. The header J3 feeds VCC through D2. The
two Schottkys mean either source works, and neither can push current back into the other; in
particular the header never drives the receiver's bias tee.

With a 4.5 V bias tee VCC sits near 4.2 V, inside the SPF5189Z's 3 V to 5.25 V operating range.
The part is actively biased, so it runs at a little less current and P1dB there, not out of spec.

## Parts

| ref | value | symbol | footprint | part |
|---|---|---|---|---|
| U1 | SPF5189Z | `SPF5189Z` | `SOT-89-3` | Qorvo SPF5189Z |
| J1, J2 | SMA | `Conn_Coaxial` | `SMA_Amphenol_132289_EdgeMount` | Amphenol 132289, edge launch for 1.6 mm |
| J3 | 5 V in | `Conn_01x02` | `PinHeader_1x02_P2.54mm_Vertical` | 2.54 mm header |
| C1, C2, C3, C5 | 100 pF C0G | `C` | `C_0402_1005Metric` | Murata GRM1555C1H101JA01D |
| C4 | 100 nF X7R | `C` | `C_0402_1005Metric` | Murata GRM155R71C104KA88D |
| C6 | 10 uF X5R 10 V | `C` | `C_0402_1005Metric` | any |
| L1, L3 | 47 nH | `L` | `L_0603_1608Metric` | Coilcraft 0603HC-47NXJRW |
| L2, L4 | 150 nH | `L` | `L_0603_1608Metric` | Coilcraft 0603CS-R15XJLW |
| D1, D2 | BAT60A | `BAT60A` | `D_SOD-323` | Infineon BAT60A |
| D3 | ESD131-B1-W0201 | `ESD131-B1-W0201` | `Infineon_SG-WLL-2-3_0.58x0.28_P0.36mm` | Infineon, 0.23 pF |
| D4 | green | `LED` | `LED_0603_1608Metric` | any 0603 |
| R1 | 1 k | `R` | `R_0402_1005Metric` | any 0402 |

All symbols and footprints were imported from the KiCad libraries with `agentee import`. Silk on
the footprints was widened from KiCad's 0.12 mm to JLCPCB's 0.15 mm minimum.

## Board

`lna.board.toml`: 30 x 20 mm, JLC04161H-7628 4 layer, ENIG, black mask.

- The RF class is grounded coplanar on F.Cu over the In1.Cu plane: 0.30 mm track, 0.2 mm gap to
  the pour, 50.0 ohm with solder mask from the GPU field solver (`solver = "field"`). The
  zero-thickness closed form said 51.2 ohm at 0.36 mm; the solver puts that width at 45.8 ohm once
  the 35 um copper walls facing the gap and the mask are counted. The thin 0.21 mm prepreg is why
  the line is narrow enough to meet 0402 pads without a taper.
- Power is 0.4 mm on the outer layers, good for 1.2 A at a 10 C rise against a 105 mA worst case.
- In1.Cu is solid ground under the whole RF path. In2.Cu carries VCC and the rest of ground.

## Layout notes for whoever places it

- Keep the RF path a straight line J1, D3, C1, U1, C2, J2 along the board's long axis, with J1 and
  J2 on opposite short edges.
- The SMA centre pad is 1.5 mm wide, four times the line. Cut In1.Cu out under that pad, the
  usual edge-launch compensation, or the pad is a capacitive step at 4 GHz.
- Stitch the coplanar pour to In1.Cu with vias no more than 2 mm apart along both sides of the
  line (a twentieth of a wavelength at 4 GHz on this line is about 2 mm).
- U1's tab is ground and its heat path; put at least four vias in it.
- L1 and L3 go right at the RF line, with L2 and L4 behind them away from it. Place C3 at the VCC
  end of L2 and C5 at the VBIAS end of L4, both with their own ground via.
- D3 goes as close to J1 as its pads allow.

## Still to do

- Measure S21 and S22 from 50 MHz to 4 GHz on the first boards. The L1/L2 choke pair is a
  starting point built from two single-band evaluation boards, not a verified wideband network.
- Check that JLCPCB assembly can place the SG-WLL-2-3 package for D3, or swap D3 for a larger
  low-capacitance clamp.

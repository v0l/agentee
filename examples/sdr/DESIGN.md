# sdr

A USB-C software defined radio: one receive and one transmit channel from 70 MHz to 6 GHz,
61.44 MS/s full duplex, streamed over USB 3.2 Gen 1 (5 Gbps). An AD9364 does the RF, an Artix-7
XC7A50T moves the samples, and an EZ-USB FX5 (CYUSB3084) is the USB device. It is bus powered
from the USB-C port.

61.44 MS/s full duplex as sc12 (12 bit I and Q) is 2.95 Gbps, inside the 3.7 Gbps Infineon quotes
as the FX5's maximum USB throughput. As sc16 it would be 3.93 Gbps, which does not fit, so full
rate full duplex needs sc12 packing in the gateware.

The schematic is `sdr.sch.toml` with four sheets (`power`, `fpga`, `rf`, `usb`), the layout is
`sdr.pcb.toml` and the board spec is `sdr.board.toml`. Every part is under `symbols/` and
`footprints/`. The whole project passes `agentee check` with no errors.

## Block diagram

```
SMA J3 -> T1 balun -> AD9364 RX  \                        / FX5 LVDS P0 (4 lanes, FPGA -> host)
                                   AD9364 CMOS P1 -> FPGA
SMA J2 <- T2 balun <- AD9364 TX  /  AD9364 CMOS P0 <- FPGA \ FX5 GPIF P1 (16 bit DDR, host -> FPGA)
                                                             FX5 <-> USB-C J1 (5 Gbps, on-chip flip mux)
```

## Data path

- **AD9364 to FPGA.** The AD9364 runs its dual-port CMOS interface at 61.44 MHz DDR, the mode
  the USRP B210 uses: P0 carries TX samples in (AD_P0_D0..11), P1 carries RX samples out
  (AD_P1_D0..11), plus AD_DATA_CLK (to FPGA F4), AD_FB_CLK, AD_RX_FRAME and AD_TX_FRAME. That is
  enough for 1R1T at 61.44 MS/s. All 28 lines sit on FPGA bank 35. U2.G10 and U2.H9 are tied to
  GND and U2.G7 and U2.H11 are left open in the schematic.
- **FPGA to host.** The FX5's LVDS port P0 can only receive (8 lanes at up to 1.25 Gbps DDR, clock
  74.25 to 625 MHz), so it takes the RX stream: four data lanes, a clock and a control pair from
  FPGA bank 34 (FX_LVDS_D0..3, FX_LVDS_CLK, FX_LVDS_CTL), routed as 100 ohm pairs on In2.Cu.
  61.44 MS/s of 16 bit I and Q is 1.97 Gbps, 1.47 Gbps as sc12.
- **Host to FPGA.** The FX5's LVCMOS port P1 carries the TX stream as a 16 bit DDR bus
  (FX_D0..15, FX_PCLK, FX_CTL0..3) into FPGA bank 14. The FX5 transmits DDR at up to 80 MHz.
- **Control.** The FX5 has an SPI link to the FPGA (FX_SPI_*), drives PROG_B and reads INIT_B
  and DONE, so the host can load a bitstream. The FPGA also boots from the W25Q128 flash (U4).
  The AD9364 is configured over AD_SPI from the FPGA.

### Gateware must know

- FX_LVDS_D0, FX_LVDS_D1 and FX_LVDS_CTL are routed with P and N swapped at the FPGA (D0 on
  M4/L4, D1 on P1/N1, CTL on N4/M5, positive leg first). Invert those three in the gateware.
- The FX_D bus and the AD CMOS buses are pinned for a planar escape, not in bit order. The pin
  map is in `fpga.sch.toml`.
- SuperSpeed RX1 has its polarity crossed at the connector (U3.M3 to J1.B10, U3.N3 to J1.B11).
  USB 3 receivers correct polarity during link training, so this needs nothing from firmware.
- The FX5 links at 5 Gbps on one lane pair; its flip mux picks lane 1 (M3/N3, M5/N5) or lane 2
  (M7/N7, M9/N9) from the plug orientation it reads on CC1/CC2 (Rd is R31, R32).

## FX20 to FX5

U3 was the EZ-USB FX20 CYUSB4024-FCAXI (USB 3.2 Gen 2x2). It is now the EZ-USB FX5
CYUSB3084-FCAXI, datasheet 002-40850 Rev. *D. The two datasheets' pin tables (Tables 9 to 12)
were compared ball by ball:

- The ball map is the same for all 169 balls: power, ground, LVDS P0 (K1/K2 D0 to B1/B2 D7, F1/F2
  clock, A1/A2 control), LVCMOS P1, SuperSpeed M3/N3 RX1, M5/N5 TX1, M7/N7 TX2, M9/N9 RX2,
  USB 2.0 N11/M11, USB FS N1/M1, CC1 K13, CC2 J13, XTALIN N13, XTALOUT M13, XRES K6, RESREF K5,
  VBUS detect G5 (P4.0), PMODE F11 (P13.0), SWD K7 to K10 (P11.0 to P11.3), SPI on SCB1 (H5
  MISO, H6 SCK, H7 MOSI, H9 CS) and the GPIOs J7, J11, J12 for DONE, PROG_B and INIT_B. No net
  moved for the part change.
- The rails are the same: VDDD 1.8 V, USB3V18 1.8 V, V33 3.3 V, VDDIO_P0 and VDDIO_P1 at 3.3 V
  (required for LVDS), 4.7 uF on VCCD, 6.04 k 1% on RESREF, a 24 MHz crystal. VDDD must come up
  with or before V33 for the CC Rd detection; 3V3 is enabled by PG_1V8, so it does.
- The USB side is Gen 1x1: one 5 Gbps lane pair at a time, picked by the on-chip flip mux, where
  the FX20 ran both at 10 Gbps.
- LVCMOS: P0CTL7..9 and P1CTL7..9 are input only; TX is 100 MHz SDR or 80 MHz DDR at most.
  The design uses P0CTL0..1 and P1CTL0..4 only.
- Power: 525 mW typical, 810 mW max in Gen 1 with LVDS (Table 23, VDDD 1.8 V), against the 1.3 W
  the thermal sims assume for the FX20. Theta_jc is 8.9 C/W, theta_ja 41.2 C/W, Tj max 125 C.

One error found on the way, in both parts: Table 9 gives P1 three columns. A8 is P1CLK and A5 is
P1D9 only in the "DDR mode (RX only)" column. In the "SDR mode (RX / TX), DDR mode (TX)" column,
which is this design (the FX5 drives the host to FPGA bus), P1CLK is A5 and A8 is P1D9. The
schematic had FX_PCLK on A8 and FX_D9 on A5. The symbol now names A5 P1CLK and A8 P1D9, FX_PCLK
is on U3.A5 and FX_D9 on U3.A8, and the FPGA end is unchanged (FX_PCLK on the MRCC pin U1.N14).
The two nets' tracks and vias were deleted and are unrouted.

## Clocks

| ref | frequency | feeds |
|---|---|---|
| Y1 | 40 MHz VCTCXO, Taitien TXEADLSANF | AD9364 reference; AUXDAC1 trims it through R30/C67 |
| X1 | 50 MHz oscillator | FPGA SYSCLK on N11 |
| Y2 | 24 MHz crystal, 10 pF | FX5 |

## Power

```
VBUS 5 V -> U5 TPS22917 load switch -> VSYS
VSYS -> U6 TPS62130 -> 1V0   FPGA VCCINT, VCCBRAM
VSYS -> U7 TPS62130 -> 1V8   FPGA VCCAUX, FX5 core
                              U9 TPS7A91 -> 1V3A   AD9364 analog
                                            FB1 -> 1V3D   AD9364 digital
                                            FB2 -> 1V3TX  TX bias through L4, L5
                              FB4 -> 1V8_USB (FX5 USB PHY), FB5 -> VCCADC
VSYS -> U8 TPS62130 -> 3V3   FPGA banks 14, 15, FX5 IO, flash, LEDs; FB3 -> 3V3_TCXO (Y1)
3V3  -> U10 AP2112K-2.5 -> 2V5   FPGA banks 34, 35, AD9364 VDD_INTERFACE; R36 -> VDD_GPO
```

U6 starts first. U7's EN is PG_1V0 and U8's is PG_1V8, the order the Artix-7 asks for. The three
bucks share one cell layout (U6/U7/U8 at y 9, 19, 29): switch node and input as small F.Cu
pours, VOS and EN on short necks to plane vias, the ground pins run into the exposed pad.

## Parts

| ref | part | notes |
|---|---|---|
| U1 | AMD XC7A50T-1FTG256C | FTG256 |
| U2 | Analog Devices AD9364BBCZ | custom footprint `AD_BC-144-7_CSP_BGA_10x10mm_P0.8mm` |
| U3 | Infineon CYUSB3084-FCAXI (EZ-USB FX5, USB 3.2 Gen 1) | custom footprint `Infineon_PG-TFBGA-169_10x10mm_P0.75mm` |
| U4 | Winbond W25Q128JVSIQ | FPGA config flash |
| U5 | TI TPS22917DBVR | VBUS load switch |
| U6, U7, U8 | TI TPS62130RGTR | 1V0, 1V8, 3V3 bucks, L1-L3 Murata DFE322520F-1R0M |
| U9 | TI TPS7A9101DSKR | 1V3A LDO |
| U10 | Diodes AP2112K-2.5TRG1 | 2V5 |
| U13 | TI TPD4E05U06DQAR | ESD on USB 2.0, CC and SBU |
| T1, T2 | Mini-Circuits TCM1-63AX+ | 1:1 baluns, footprint `MiniCircuits_DB1627` |
| J1 | Molex 105450-0101 | USB-C, mid-mount at the bottom edge |
| J2, J3 | Amphenol 132289 | edge SMAs on the left edge, TX and RX |
| J4, J5 | Tag-Connect TC2050 / TC2030 | FPGA JTAG and FX5 SWD |
| L4, L5 | Murata LQW18ANR33G00 | 330 nH TX bias chokes to 1V3TX |
| C68-C71 | Murata GJM1555C1H180GB01 | 18 pF RF DC blocks |
| C88-C91 | 220 nF 0201 | SuperSpeed TX AC caps |

The rest are 0201, 0402, 0603 and 0805 passives; `agentee fab` writes the full BOM.

## Board

`sdr.board.toml`: 90 x 60 mm, 8 layers (JLC08161H-1080, 1.68 mm), ENIG, black mask.

| layer | use |
|---|---|
| F.Cu | parts, fanout, RF feeds, SuperSpeed TX1 and RX2, FX_D bus, GND pour |
| In1.Cu | GND |
| In2.Cu | AD CMOS buses, FPGA to FX5 LVDS, a GND island under the baluns |
| In3.Cu | GND |
| In4.Cu | 1V0, 1V8, 1V8_USB, VSYS planes, a GND island at the SMA edge |
| In5.Cu | 1V3A, 2V5, 3V3 planes |
| In6.Cu | GND |
| B.Cu | decoupling, SuperSpeed RX1 and TX2, GND pour |

Vias are `std` (0.3 mm drill, 0.5 mm pad) and `bga` (0.2 / 0.35 mm, in-pad under the BGAs, filled
and capped). 518 vias sit in pads.

| class | target | geometry | layers |
|---|---|---|---|
| USB_SS | 85 ohm diff | 0.159 mm, 0.15 mm gap | F, B |
| USB_HS | 90 ohm diff | 0.143 mm | F, B |
| LVDS | 100 ohm diff | 0.127 mm, 0.125 mm gap | In2 |
| RF | 50 ohm GCPW | 0.12 mm, 0.25 mm to the pour, In1 below | F |
| Power | 1 A | 0.4 mm | F, B |
| PowerHi | 2 A | 0.9 mm | F, B |

### Layout

- FX5 at the bottom next to J1, FPGA top centre, AD9364 left of it, the three bucks in a column
  on the right, RF on the left edge. The SMAs and the USB-C sit on different edges.
- The BGAs are fanned out with `[[fanouts]]`: a global one puts a `bga` via in every plane pad,
  and each BGA's own entry leaves its escape rows free.
- The single-ended nets were routed with `agentee route`. The RF feeds, the buck cells, the
  SuperSpeed lanes and the LVDS link were drawn by hand.
- Every pair is length matched with `agentee tune`. The SuperSpeed TX pairs are matched end to end
  through their AC caps.
- GND pours on F.Cu and B.Cu, a via fence either side of every RF line, and a 2.5 mm grid of
  stitching vias wherever it clears.
- The SMA centre pads are 1.5 mm wide, so In1-In3 are cut back under them and they reference the
  In4 GND island. The balun signal pads reference In2 with In1 cut out below them.

## Simulations

| sim | what | result |
|---|---|---|
| `sdr-rf` | FDTD, SMA to balun, 70 MHz to 6 GHz | loss under 0.3 dB to 3 GHz and 0.75 dB at 6 GHz; return loss better than 15 dB to 3.6 GHz, 10.9 dB at 6 GHz; RX to TX coupling -92 dB at 70 MHz, -51 dB at 6 GHz |
| `sdr-1v0` | DC, FPGA core at 0.9 A | worst VCCINT ball 0.990 V against a 0.95 V floor |
| `sdr-vbus` | DC, VBUS to the buck inputs at 0.93 A | 4.897 V at the bucks; 74 mV of the 103 mV drop is the load switch; both VBUS pin pairs share the current (659 / 271 mA) |
| `sdr-thermal` | still air, bare board, 4.4 W | FPGA junction 85.9 C, AD9364 102 C, U3 119 C (FX20 at 1.3 W) |
| `sdr-thermal-fan` | 25 W/m2K both faces | FPGA 60 C, AD9364 78 C, U3 93 C (FX20 at 1.3 W) |
| `sdr-usb` | FDTD, SuperSpeed TX1 and RX2 lanes | does not run yet, see below |

The board needs airflow. In still air the commercial grade FPGA sits just over its 85 C limit and
U3 runs hot. The theta_jc figures in the thermal specs are estimates, not datasheet values, and
both runs predate the FX5.

## Still to do

- Fix the FDTD instability that stops `sdr-usb`: a port on U3.M5 diverges at every cell size
  tried (0.1 down to 0.035 mm), a port on J1.A2 diverges at 0.1 mm and runs at 0.075 mm, while
  ports on the 0201 cap pads in the same region are stable. Then run `sdr-usb` and the 5 Gbps
  channel eye on each lane.
- Route FX_PCLK (U3.A5 to U1.N14) and FX_D9 (U3.A8 to U1.R8), unrouted since the P1CLK fix.
- Set U3 in the thermal specs to the FX5 (810 mW max, theta_jc 8.9 C/W), take theta_jc for U1 and
  U2 from their datasheets and rerun `sdr-thermal`; pick a fan, a heatsink on U3, or the
  XC7A50T-1FTG256I (100 C) if the board has to run in still air.
- Reroute the SuperSpeed breakouts so more of each lane runs at the pair gap (check reports
  50-60% coupled against the 80% it wants), `sdr.pcb.toml`, the SS_* and FX_TX* tracks.
- Add a `kind = "pdn"` sim for 1V0 with the FPGA balls as sinks and the 0201 decaps under U1.
- Tune the SMA launch cutout size for better than 15 dB return loss at 6 GHz, `sdr.pcb.toml`
  `[[cutouts]]` under J2.1 and J3.1, then rerun `sdr-rf`.
- Confirm the AD9364 CMOS-mode pin handling (U2.G10, U2.H9 to GND, U2.G7, U2.H11 open) against
  UG-673 before ordering.

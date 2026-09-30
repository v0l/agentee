# sdr

A USB-C software defined radio: one receive and one transmit channel from 70 MHz to 6 GHz,
61.44 MS/s full duplex, streamed over USB 3.2 Gen 1 (5 Gbps). An AD9364 does the RF, an Artix-7
XC7A50T moves the samples, and an EZ-USB FX5 (CYUSB3084) is the USB device. It is bus powered
from the USB-C port, behind an eFuse. A switchable LNA and TX driver sit between the baluns and
the SMAs, and the 40 MHz reference can lock to an external 10 MHz with a PPS input beside it.

61.44 MS/s full duplex as sc12 (12 bit I and Q) is 2.95 Gbps, inside the 3.7 Gbps Infineon quotes
as the FX5's maximum USB throughput. As sc16 it would be 3.93 Gbps, which does not fit, so full
rate full duplex needs sc12 packing in the gateware.

The schematic is `sdr.sch.toml` with six sheets (`power`, `fpga`, `rf`, `frontend`, `clock`,
`usb`), the layout is `sdr.pcb.toml` and the board spec is `sdr.board.toml`. Every part is under
`symbols/` and `footprints/`. The schematic passes `agentee check`; the layout does not yet (see
Still to do).

## Block diagram

```
SMA J3 -> U14 -+- U16 LNA -+- U15 -> T1 balun -> AD9364 RX  \                        / FX5 LVDS P0 (FPGA -> host)
               +-- bypass -+                                  AD9364 CMOS P1 -> FPGA
SMA J2 <- U19 -+- U18 amp -+- U17 <- T2 balun <- AD9364 TX  /  AD9364 CMOS P0 <- FPGA \ FX5 GPIF P1 (host -> FPGA)
               +-- bypass -+                                                           FX5 <-> USB-C J1

SMA J6 10 MHz -> U20 LTC6957-3 -+-> U21 ADF4002 REF      Y1 40 MHz -> ADF4002 RF (N = 4)
                                +-> FPGA REF_10M         ADF4002 CP -> loop filter -> U22 mux -> Y1 VC
SMA J7 PPS -> U23 Schmitt -> FPGA PPS_IN                 AD9364 AUXDAC1 -> U22 mux -> Y1 VC
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

## RF front end

`frontend.sch.toml`. Each path has an SPDT pair, Infineon BGS12WN6 (0.05 to 9 GHz, 0.2 dB loss
below 700 MHz, 0.6 dB at 6 GHz, 26 dBm), around an amplifier and a straight bypass. The switches
need no DC blocks as long as their RF ports sit at 0 V, so each amplifier has a block on both
sides and the bypass runs straight through.

| path | amplifier | supply | control, high is |
|---|---|---|---|
| RX | U16 Qorvo QPL9547, 0.1 to 6 GHz, 19.5 dB, 0.3 dB NF at 1.9 GHz | 5V_LNA from VSYS via FB6, 65 mA set by R37 3.32k | RX_BYPASS: bypass, RX_LNA_SD: LNA off |
| TX | U18 TI TRF37A73, 1 to 6000 MHz, 12 dB, 14.5 dBm P1dB | 3V3_TXA from 3V3 via FB7, 55 mA | TX_BYPASS: bypass, TX_AMP_PD: amplifier off |

- CTRL high selects RF2, which is the bypass on all four switches. R38 to R41 pull the four
  controls high, and PUDC_B is low so the FPGA pulls its IOs up during configuration: until the
  gateware says otherwise the board is in bypass with both amplifiers off.
- The LNA is specified from 100 MHz. Below that use the bypass.
- Both amplifiers take bias through their output pin: L6 and L7 are 100 nH (Murata
  LQW18ANR10G00, TI's value for the TRF37A73). DC blocks are 100 pF C0G on the LNA (Qorvo's EVB
  value) and 1 nF on the TX driver (TI's).
- The TX driver's input limit is 10 dBm absolute. The AD9364 cannot exceed that.

## Clocks

| ref | frequency | feeds |
|---|---|---|
| Y1 | 40 MHz VCTCXO, Taitien TXEADLSANF | AD9364 reference, and the ADF4002 RF input through R47/C118 |
| X1 | 50 MHz oscillator | FPGA SYSCLK on N11 |
| Y2 | 24 MHz crystal, 10 pF | FX5 |

### External reference and PPS

`clock.sch.toml`, the B210 arrangement.

- J6 takes a 10 MHz sine or square. C109 blocks DC, R42 terminates it in 50 ohm, R43 and the
  BAT54S (D7) clip anything large, and U20, an LTC6957-3, squares it: OUT1 to the ADF4002, OUT2 to
  the FPGA as REF_10M on an MRCC pin so the gateware can measure it. LTC6957 takes 0.2 to 2 Vpp
  single ended, which is -10 to +10 dBm into the 50 ohm; stronger signals are clipped by D7.
  FILTA and FILTB are high, the narrowest input filter, for a slow 10 MHz edge.
- The ADF4002 wants 50 V/us on REFIN below 20 MHz, which a 10 MHz sine under +10 dBm does not
  have. That is why the LTC6957 is there.
- U21 ADF4002 locks Y1: R = 1, N = 4, 10 MHz at the phase detector. RSET 5.1k gives 5 mA full
  scale. The loop filter is C120 47n, R48 15k with C121 470n, and R49 1k into C67 (100n, on Y1's
  VC pin) as the third pole: about 100 Hz of loop bandwidth at the 0.625 mA charge pump setting,
  assuming Y1 pulls about 8 ppm over 2 V.
- U22, a TS5A3159, picks Y1's tuning voltage: VC_DAC high is AUXDAC1 through R30 (free running,
  trimmed by the AD9364), low is the loop filter. R50 holds it on the DAC until the gateware
  drives it. PLL_MUXOUT returns lock detect to the FPGA.
- J7 takes PPS. D8 clamps it, R51 limits the current, R52 holds it low when nothing is plugged in,
  and U23, a 74LVC1G17 on 3V3 (5.5 V tolerant input), squares it into PPS_IN.
- J8 is a 2x5 1.27 mm header: 3V3, GND and GPIO0 to GPIO7 from bank 15, each through 33 ohm, with
  TPD4E05U06 ESD arrays (U25, U26) on the header side.

### FPGA pins added on bank 15 (3V3)

| net | pin | | net | pin |
|---|---|---|---|---|
| RX_BYPASS | A8 | | PLL_MUXOUT | A12 |
| RX_LNA_SD | A9 | | VC_DAC | C8 |
| TX_BYPASS | B9 | | REF_10M | E12 (MRCC) |
| TX_AMP_PD | A10 | | PPS_IN | C11 (SRCC) |
| PLL_CLK | B10 | | GPIO0-4 | H11, H12, H13, H14, H16 |
| PLL_DATA | B11 | | GPIO5-7 | G14, G15, G16 |
| PLL_LE | B12 | | | |

VBUS_FAULT_N goes to the FX5 on P4.1 (U3.G6).

## Power

```
VBUS 5 V -> D6 SMF12A -> U5 TPS25200 eFuse -> VSYS            VSYS -> FB6 -> 5V_LNA (U16)
VSYS -> U6 TPS62130 -> 1V0   FPGA VCCINT, VCCBRAM
VSYS -> U7 TPS62130 -> 1V8   FPGA VCCAUX, FX5 core
                              U9 TPS7A91 -> 1V3A   AD9364 analog
                              U24 TPS7A91 -> 1V3S  AD9364 RX/TX synthesizers, VCO LDO inputs
                                            FB1 -> 1V3D   AD9364 digital
                                            FB2 -> 1V3TX  TX bias through L4, L5
                              FB4 -> 1V8_USB (FX5 USB PHY), FB5 -> VCCADC
VSYS -> U8 TPS62130 -> 3V3   FPGA banks 14, 15, FX5 IO, flash, LEDs; FB3 -> 3V3_TCXO (Y1, U22)
                              FB7 -> 3V3_TXA (U18), FB8 -> 3V3_REF (U20, U21)
3V3  -> U10 AP2112K-2.5 -> 2V5   FPGA banks 34, 35, AD9364 VDD_INTERFACE; R36 -> VDD_GPO
```

U6 starts first. U7's EN is PG_1V0 and U8's is PG_1V8, the order the Artix-7 asks for. PG_1V8 also
enables both TPS7A91s. U8's power good is FX_XRES, so the FX5 stays in reset until 3V3 is up; R10
now pulls it up to 3V3, not VSYS.

- U5 is a TPS25200: IN takes 20 V, OUT clamps at 5.4 V, and R61 56k limits it at 1.62 to 1.86 A.
  R1 (300k) ties EN to VBUS through the pin's internal zener. FAULT is open drain, pulled up by
  R62 and read by the FX5 on P4.1, so firmware can report an overcurrent or overvoltage.
- D6, an SMF12A, catches hot plug and ESD on VBUS. Its 12 V standoff leaves the eFuse to handle a
  sustained overvoltage.
- The AD9364's synthesizer and VCO LDO inputs (J3, K3, B10, F2) have their own LDO, U24, as ADI
  recommends, so the synthesizers do not share a rail with the RX and TX analog. C50, C55 and C56
  moved to 1V3S with them.
- Budget: the VBUS DC sim measured 0.93 A before this change. The LNA (65 mA at 5 V), the TX
  driver, the LTC6957 and the ADF4002 add up to about 0.6 W with everything on, so a full load
  needs more than the 900 mA a USB 3 port promises. The three
bucks share one cell layout (U6/U7/U8 at y 9, 19, 29): switch node and input as small F.Cu
pours, VOS and EN on short necks to plane vias, the ground pins run into the exposed pad.

## Parts

| ref | part | notes |
|---|---|---|
| U1 | AMD XC7A50T-1FTG256C | FTG256 |
| U2 | Analog Devices AD9364BBCZ | custom footprint `AD_BC-144-7_CSP_BGA_10x10mm_P0.8mm` |
| U3 | Infineon CYUSB3084-FCAXI (EZ-USB FX5, USB 3.2 Gen 1) | custom footprint `Infineon_PG-TFBGA-169_10x10mm_P0.75mm` |
| U4 | Winbond W25Q128JVSIQ | FPGA config flash |
| U5 | TI TPS25200DRVR | VBUS eFuse, symbol drawn from the datasheet, KiCad's `WSON-6-1EP_2x2mm_P0.65mm_EP1x1.6mm` |
| U6, U7, U8 | TI TPS62130RGTR | 1V0, 1V8, 3V3 bucks, L1-L3 Murata DFE322520F-1R0M |
| U9, U24 | TI TPS7A9101DSKR | 1V3A and 1V3S LDOs |
| U10 | Diodes AP2112K-2.5TRG1 | 2V5 |
| U13 | TI TPD4E05U06DQAR | ESD on USB 2.0, CC and SBU |
| U14, U15, U17, U19 | Infineon BGS12WN6E6327 | RX and TX bypass switches |
| U16 | Qorvo QPL9547 | RX LNA |
| U18 | TI TRF37A73IDSGR | TX driver |
| U20 | Analog Devices LTC6957IDD-3 | 10 MHz reference squarer |
| U21 | Analog Devices ADF4002BCPZ | reference PLL |
| U22 | TI TS5A3159DBVR | VCTCXO tuning voltage mux |
| U23 | TI SN74LVC1G17DBVR | PPS buffer |
| U25, U26 | TI TPD4E05U06DQAR | ESD on the GPIO header |
| D6 | Littelfuse SMF12A | VBUS TVS |
| T1, T2 | Mini-Circuits TCM1-63AX+ | 1:1 baluns, footprint `MiniCircuits_DB1627` |
| J1 | Molex 105450-0101 | USB-C, mid-mount at the bottom edge |
| J2, J3, J6, J7 | Amphenol 132289 | edge SMAs: TX, RX, 10 MHz REF, PPS |
| J8 | 2x5 1.27 mm SMD header | GPIO |
| L6, L7 | Murata LQW18ANR10G00 | 100 nH amplifier bias chokes |
| J4, J5 | Tag-Connect TC2050 / TC2030 | FPGA JTAG and FX5 SWD |
| L4, L5 | Murata LQW18ANR33G00 | 330 nH TX bias chokes to 1V3TX |
| C68-C71 | Murata GJM1555C1H180GB01 | 18 pF RF DC blocks |
| C88-C91 | 220 nF 0201 | SuperSpeed TX AC caps |

The rest are 0201, 0402, 0603 and 0805 passives; `agentee fab` writes the full BOM.

## Board

`sdr.board.toml`: 65 x 45 mm, 6 layers, the `JLC06161H-1080B` preset (1.6 mm), ENIG, black mask.
In1 sits 0.1 mm under In2 and In3 0.1 mm over In4, with 1.1 mm between the two pairs, so In2
references the In1 ground and the In3 power plane gets a tight ground below it.

| layer | use |
|---|---|
| F.Cu | parts, fanout, RF feeds, SuperSpeed TX1 and RX2, FX_D bus, GND pour |
| In1.Cu | GND |
| In2.Cu | AD CMOS buses, FPGA to FX5 LVDS, a GND island under the baluns |
| In3.Cu | power: 1V0, 1V8, 1V8_USB, VSYS, 1V3A, 2V5 and 3V3 split, a GND island at the SMA edge |
| In4.Cu | GND |
| B.Cu | decoupling, SuperSpeed RX1 and TX2, GND pour |

The layout was drawn for the earlier 8 layer JLC08161H-1080 stack and its layers were mapped
across: In6 to In4, and both power layers onto In3, where the planes now overlap.

Vias are `std` (0.3 mm drill, 0.5 mm pad) and `bga` (0.2 / 0.35 mm, in-pad under the BGAs, filled
and capped). 518 vias sit in pads.

| class | target | geometry | layers |
|---|---|---|---|
| USB_SS | 85 ohm diff | 0.159 mm, 0.15 mm gap | F, B |
| USB_HS | 90 ohm diff | 0.143 mm | F, B |
| LVDS | 100 ohm diff | 0.097 mm, 0.25 mm gap | In2 |
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
- The SMA centre pads are 1.5 mm wide, so In1 and In2 are cut back under them and they reference
  the In3 GND island. The balun signal pads reference In2 with In1 cut out below them.

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

- Grow the outline and place the 83 new parts in `sdr.pcb.toml` (front end between the baluns and
  the SMAs, J6 and J7 on an edge, J8 near bank 15); check lists each one as not placed.
- Split In3 into the power islands the old In4 and In5 held: 1V0 under U1's core balls, 2V5 under
  banks 34/35 and U2, 3V3 under banks 14/15 and U3, 1V3A and 1V3S under U2, with 1V8 and 1V8_USB
  moved to B.Cu pours if In3 runs out. The mapped zones overlap today (33 shorts in check).
- Reroute the LVDS pairs for the new 0.097 mm / 0.25 mm geometry and move the 1V3S decaps (C50,
  C55, C56) and U5's new pads, then rerun `sdr-rf`, `sdr-vbus` and `sdr-1v0`.
- Widen the RF class to the 0.171 mm the field solver asks for (check reads 61.9 ohm at 0.12 mm),
  `sdr.board.toml` `[[netclasses]]` RF, and redraw the RF tracks.
- Add a front end FDTD with ports at U14 to U19 and a cascade with the QPL9547 and TRF37A73
  S-parameters; the 100 nH chokes and 100 pF blocks at 100 MHz and 6 GHz are unverified.
- Confirm LTC6957 FILTA/FILTB high is the best setting for 10 MHz against the datasheet's
  filter table (unverified), `clock.sch.toml` U20.1 and U20.6.
- Check Y1's pulling range in the Taitien datasheet and size the loop filter with ADIsimPLL,
  `clock.sch.toml` C120, R48, C121, R49.
- Decide how the board gets more than 900 mA: read the USB-C current advertisement on CC in FX5
  firmware, or add an auxiliary 5 V input; the front end and reference add about 0.6 W.

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

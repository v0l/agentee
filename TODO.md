# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## Spacing

- A footprint `clearance` relaxes its pads but not the tracks that run into them: the
  UCC256404 RVCC (pin 12) and HB (pin 14) pads are 1.94 mm apart, so the two tracks leaving
  them are always under a 2.5 mm class clearance. `[[drc.waive]]` cannot take one such pair
  either, since every clearance finding has `at = "clearance"`. Seen on ubench `mains`.
- `agentee place mains --parts ...` (ubench) put the PFC controller cluster on the secondary
  side of the primary/secondary barrier, and treats THT courtyards as blocking the other side
  so nothing goes on B.Cu under the bulk caps, which check allows.
- `agentee layout mains` (ubench, 2.5 mm HV classes) ran 30 min with no output and was killed.

## Circuit sim

- No THD or spectrum of a transient, so clipping shows only as a waveform and checks read a
  voltage at a fixed time (`examples/preamp` `preamp-clip`). No peak, RMS or frequency over a
  time window either: `examples/ladder-vcf` `vcf-osc` oscillates at 441 Hz but can only check
  an operating point.
- A pot (`RV`) has no model: a 3-pin pot needs a `vcvs` block, which has no wiper resistance
  (`examples/ladder-vcf` RV2, RV3). A `potentiometer` kind with `resistance` and `position`.
- Op-amp block has one `headroom` for both rails; the LM358 swings to within mV of ground but
  1.5 V of V+, so `preamp-clip` clips low at 1.5 V. No slew rate, input bias current or PSRR,
  and `en` is flat with no 1/f corner (`crates/agentee-sim/src/circuit.rs` Buffer).

## Library

- `library find` merges offers by part number across makers (taken: todo-fixes-3): 1N5817G shows as onsemi with
  Mouser's Comchip PDF (`QW_BG016_1N5817_G...`) as its datasheet link.

## Layout

- Engine leaves small THT parts far from their op-amp pin on `examples/preamp`: R4 13.6 mm from
  U1.2, R5 11.5 mm (`placement-cluster-spread` notes after `--search 40 --keep 8`). On
  `examples/ladder-vcf` the unlocked first run scattered the ladder transistors across the
  board (169 crossings) and put D1/D2 70 mm from J1; the ladder, expo pair and diodes are locked
  by hand there. 100n decouplers still land 7-14 mm from their TL072 pins.
- `layout ladder-vcf --search 64 --keep 8` scored worse than `--search 32` and left GND
  unrouted (RV3.1 cut off from the pour).

## Topological router

- Classes whose width plus clearance is over `site_pitch` (0.4 mm) cannot cross the via site
  lattice: a portal between two unused sites has only the pitch as room
  (`crates/agentee-layout/src/topo.rs` `Router::room`). On `sdr` the Power and PowerHi nets fail.
- About 165 nets on `sdr` lose copper when topo's route is held to the rules: wires 10-60 um
  inside clearance of each other or of a pad between portals, and arcs round a single outline
  sample dropping under the pad edge beside it (`round_corners`). Seen with
  `phases = ["global", "topo"]` and `AGENTEE_UNROUTED_DEBUG=1`.
- Topo as a guide (`topo` and `detail` both in `phases`) changes nothing on `sdr`: detail's
  `prefer` reservation from topo's copper gives the same 66-67 unrouted as detail alone.

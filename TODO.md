# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.
- `library verify` fails three footprints in the global library on JLCPCB annular rings:
  USB_C_Receptacle_GCT_USB4960_Vertical (0.175 mm, 4 pads),
  VQFN-24-1EP_4x4mm_P0.5mm_EP2.5x2.5mm_ThermalVias and VQFN-28-1EP_5x5mm_P0.5mm_EP3.7x3.7mm_ThermalVias
  (0.15 mm thermal vias). Seen from `examples/preamp`.

## Spacing

- `agentee layout mains` (ubench) never finishes the detail stage: round 0 of the negotiated
  router runs past 10 min on a 2725x2805x4 grid with 237 connections. Access reports 190 of 351
  pads with no legal exit and global an overflow of 7518 mm, so most searches can only fail, and
  each exhausts its window first. Detail should fail connections to pads with no exit up front
  (`access_report` in `crates/agentee-layout/src/negotiate/mod.rs` already finds them). The
  footprint clearance zones in `negotiate/rules.rs` also raise the track buckets from 52 to 92.
- The footprint clearance zone in the router covers tracks only: via buckets carry no zone
  (`crates/agentee-layout/src/negotiate/soft.rs` `Soft::footprint`), so a via inside the
  courtyard still keeps the class clearance from the part's other nets.
- `agentee place mains --side both` (ubench, tracks stripped) leaves 3 creepage findings
  against 1 on the placer before 580f1f5: U3's pins 5.5 mm from the GND_SEC pour through a slot,
  and U8.1 to U8.6 at 6.02 mm. The placer does not weigh creepage to pours.
- The engine's place stage on `mains` takes 146 s against 49 s for `agentee place`: it runs the
  core placer twice (two sets of 8 starts), the first with legalise at 50-100 s per start.

## Layout

- `examples/ladder-vcf` bias string R9-R13 and C17/C18 land 25-50 mm from the ladder: their
  nets (V1-V4) join two transistor bases, so `build_attachments` in
  `crates/agentee-core/src/place.rs` gives them no pull, and the space beside the locked ladder
  goes to U1, C4 and R3. Seen with `layout ladder-vcf --to place`, seeds 1-8.
- With every part but connectors, holes and pots unlocked, `layout ladder-vcf` still scatters
  the ladder: 108-179 crossings over seeds 1-3, against 118-153 with the ladder locked.
- `examples/preamp`: the worst passive is still 11.6 mm from its U1 pin on average over seeds
  1-8 of `layout preamp --to place` (R4 and C6 lose the space round U1 to R3, R5, R6).
- `layout ladder-vcf` seed 3 leaves 1 connection unrouted since the bridging pull (8a995cb),
  where seeds 1 and 2 route everything.

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

## Circuit sim

- The op-amp output stage is a tanh soft clip about mid-rail, so its slope is under 1 well
  inside the rails: a 1 V/us slew limited follower ramps at 0.97 V/us between 3 and 6 V on 10 V
  rails (`crates/agentee-sim/src/circuit.rs` `Unit::Buffer`).

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

- KiCad 9 footprints name 3D models that kicad-packages3D does not have on any tag
  (`Connector_Audio.3dshapes/Jack_3.5mm_QingPu_WQP-PJ398SM_Vertical.step`,
  `Potentiometer_THT.3dshapes/Potentiometer_Bourns_PTV09A-1_Single_Vertical.step`), so `library fill` finds none.
  Espressif's repo is tried for the ESP32 modules; no source is tried for the rest.
- `library fill --like` refuses an EasyEDA model when the pads sit 1.1-1.3 mm off ours (TL3342F160QG,
  105017-0001 against USB_Micro-B_Molex-105017-0001, a 3x4 mm tactile), and with no shared pad numbers
  (PJ-320D against Jack_3.5mm_KoreanHropartsElec_PJ-320D-4A_Horizontal). Unchecked whether ours or theirs is off.

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
- `agentee place` does not keep a floating domain's parts together: on ubench `main` (900
  parts, PSU and LOAD domains with barriers) it put 180 rear parts in another domain's area.
  Placing a domain at a time with `[place] keepouts` round its region works, but `[place]` has
  no `edit pcb` command, so each pass edits the layout file by hand. A `region` per domain or
  per sheet would do it.
- `[place] keepouts` stop courtyards only. There is no height limit per area, so parts under a
  heatsink or a fan (ubench `main`: under 2 mm and 5 mm) are kept low by hand.
- `agentee layout --from access` on ubench `main` (900 parts, 4 layers) prints nothing for over
  1.5 hours; a progress line per stage or per few hundred nets would help.

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

- No Pst readout for flicker (EN 61000-3-3) from a switched load.

## Performance

- Loading ubench `hardware` takes 8 s in a release build, 3 min in debug (`agentee list .`
  from any item). The schematic wire router is most of it: `main` (937 parts, grid 353x3745
  cells) runs 742 searches. `Layout::resolve` is 1.6 s for the two layouts, mostly zone fills.
  Run `agentee` from `target/release` for anything on ubench.
- `free_spot` (`crates/agentee-core/src/layout.rs`) tries 636 spots for each of 258 labels on
  ubench `main`, and 247 find none: the part is boxed in by `crowds` and `sits on pads`. A
  label with no clear spot near the part could stop early.

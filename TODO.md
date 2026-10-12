# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## Spacing

- `agentee layout mains` (ubench) never finishes the detail stage: round 0 of the negotiated
  router runs past 10 min on a 2725x2805x4 grid with 237 connections. Access reports 190 of 351
  pads with no legal exit and global an overflow of 7518 mm, so most searches can only fail, and
  each exhausts its window first. Detail should fail connections to pads with no exit up front
  (`access_report` in `crates/agentee-layout/src/negotiate/mod.rs` already finds them). The
  footprint clearance zones in `negotiate/rules.rs` also raise the track buckets from 52 to 92.

## Layout

- `examples/ladder-vcf` bias string R9-R13 and C17/C18 land 25-50 mm from the ladder: their
  nets (V1-V4) join two transistor bases, so `build_attachments` in
  `crates/agentee-core/src/place.rs` gives them no pull, and the space beside the locked ladder
  goes to U1, C4 and R3. Seen with `layout ladder-vcf --to place`, seeds 1-8.
- With every part but connectors, holes and pots unlocked, `layout ladder-vcf` still scatters
  the ladder: 108-179 crossings over seeds 1-3, against 118-153 with the ladder locked.

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

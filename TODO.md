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

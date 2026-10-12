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
  voltage at a fixed time (`examples/preamp` `preamp-clip`).
- Op-amp block has one `headroom` for both rails; the LM358 swings to within mV of ground but
  1.5 V of V+, so `preamp-clip` clips low at 1.5 V. No slew rate, input bias current or PSRR,
  and `en` is flat with no 1/f corner (`crates/agentee-sim/src/circuit.rs` Buffer).
- `render sim:NAME` at the default 900 px cuts off the noise panel when there is a transient or
  AC plot above it; `preamp-ac` needed `--height 1800` (`crates/agentee-view/src/circuit.rs`).

## Schematic edit

- `edit sch add U1 ... --unit 2` fails with "`U1` is already a part here", and `net` only
  resolves pins of the first placed unit ("`U1` has no pin `8`; it has 3, 2, 1"). U1 units 2
  and 3 of `examples/preamp` were written by hand.
- `edit sch remove D1` left `pins = [ "C1.1", ...]` with a stray space after the bracket.
- `edit sch` wrote a batch while the schematic had a load error (unknown key `title`).

## Library

- `library add --datasheet URL` pins the URL in `part.toml` but leaves the KiCad symbol's
  `datasheet`: LM358P kept `lm2904-n.pdf`, 1N5817G kept Vishay's for an onsemi part.
- `datasheet fetch LM358P` without `--url` cached ST's LM358 (DocID2163) for TI's LM358P, and
  1N5817G (onsemi 1N5817/D) was cached under `comchip-technology/`.
- `library find` run 4 at a time gets Farnell `403 Account Over Queries Per Second Limit` and
  drops Farnell from the result instead of retrying.
- `library verify` passes a footprint whose slots break the fab minimum (KiCad's SJ1-3523N has
  0.4 mm slots); only `check` on a placed layout catches it.

## Layout

- `agentee layout NAME --search` places nothing while parts are unplaced (only placed parts
  reach `model.layout.parts`), so `agentee place NAME` has to run first. SKILL.md's layout loop
  says the engine places them.
- After `edit pcb unplace ...` then `agentee place`, `examples/preamp/preamp.pcb.toml` had two
  `# plan route` headers and one `# end plan route`; the next `layout --search` kept the old
  plan's tracks and check found 30 shorts. A second `layout --search` cleaned it. Not reproduced.
- `render pcb:NAME --hide F.Cu` draws no B.Cu copper unless `--show B.Cu` is also given.
- Engine leaves small THT parts far from their op-amp pin on `examples/preamp`: R4 13.6 mm from
  U1.2, R5 11.5 mm (`placement-cluster-spread` notes after `--search 40 --keep 8`).

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
- `examples/sdr/sdr.pcb.toml` has `# plan route` with no `# end plan route`, so `agentee layout`
  cannot strip it: the old route stays as fixed copper and only about 53 leftovers reroute.

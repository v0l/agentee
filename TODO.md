# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## Spacing (taken: hv-spacing)

- `agentee route` inflates pads by the routed net's class clearance and ignores a footprint
  `clearance`, so it cannot reach the middle pin of a TO-220 on a 2.5 mm HV class that check
  accepts (`crates/agentee-layout/src/negotiate/grid.rs`). Seen on ubench `mains`.
- No way to say a class's nets ride together (a half-bridge high side: HB, HO and the gate
  within 15 V of the switch node), so a 430VDC class needs 2.5 mm between its own nets and
  the UCC256404 HB/HO/HS pins on 1.27 mm cannot be routed.

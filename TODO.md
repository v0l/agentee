# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## DRC (from PR #15) (taken: drc-waivers)

- `import board` sets `edge = false` on every footprint pad
  (`crates/agentee-kicad/src/footprint.rs`), so castellated pads and edge-card lands that cross
  the outline report `pad-off-board` on every imported board.
- No per-finding waiver: `[drc] disable` turns a rule off for the whole board, nothing records
  one reviewed finding (a milled edge-card socket's `pad-off-board` or `unrouted`) as accepted.

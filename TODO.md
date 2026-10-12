# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## Fab

- `via-in-pad` (`crates/agentee-core/src/drc/via.rs`) and the fab notes miss plated holes a
  footprint puts in its own SMD pad (KiCad `*_ThermalVias` exposed pads): the rotator `ctl`
  board has 41 such 0.2 mm holes under U12, U13 and U14, and check and `fab-notes.txt` say nothing.

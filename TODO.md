# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Smaller gaps

Found while laying out the rotator control board (`~/git/rotator/board`).

- There is no `uncutout`, so a duplicate `[[cutouts]]` has to be removed by hand.
- `agentee tie` drops far more stitching vias than the island needs.
- In `agentee show pcb:NAME` JSON, `net` is an index on tracks and vias but a name on
  pads. Use one form everywhere.

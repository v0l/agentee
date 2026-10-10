# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Port the rest of `place-current-nets` (taken: master)

Branch `place-current-nets`, worktree `~/git/agentee-place-current-nets`. Master already
has its mounts and current-net placement. These three are still missing:

- `9eada12`: `edit pcb cutout --point X,Y ... --layers ...`, a copper cutout on chosen
  layers, plus point lists for `zone --outline` (`POINT_LISTS` in `edit/mod.rs`).
  `~/git/rotator/board/tools/zones.txt` uses it for the Ag5424 isolation keep-out.
- `54a4a62`: `zone` replaces only a pour with the same net and the same outline, so a
  second outline on one net becomes a second pour.
- `f73e7b1`: `dangling-via` DRC warning for a via that joins fewer than two layers.

When they are on master, delete the branch and the worktree.

## Smaller gaps

Found while laying out the rotator control board (`~/git/rotator/board`).

- There is no `uncutout`, so a duplicate `[[cutouts]]` has to be removed by hand.
- `agentee tie` drops far more stitching vias than the island needs.
- In `agentee show pcb:NAME` JSON, `net` is an index on tracks and vias but a name on
  pads. Use one form everywhere.

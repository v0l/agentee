# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Edit

- `edit sch NAME net NET PIN` without `--sheet` writes into the first schematic in the project
  that has a net of that name, even one of another board (`crates/agentee/src/edit/sch.rs`,
  `here = sheets[0]`). In buggy-guard `edit sch power net +5V R67.1` failed with "no net `+5V` in
  pendant.sch.toml" and `net +5V U2.1` with "no part `U2`".

## View

- `the_engine_runs_into_the_editor_and_waits_for_save` in `crates/agentee-view/tests/edit.rs`
  fails on master (ef65ccf): "the editor never settled" after 400 frames at line 120.

## Check

- `testpoint::is_rail` takes any name starting `VBAT`/`VBUS` as a rail, so the level check calls
  `VBAT_ADC` and `VBUS_SENSE` ADC pins overdriven at the class voltage
  (`crates/agentee-core/src/testpoint.rs:221`). buggy-guard works round it with `rails` entries.

## Layout

- No routing keepout: `route` and `layout` cannot be kept off an area of a layer, such as a buck
  switch node. buggy-guard v0.4.0 dropped temporary 4 mm GND tracks there and removed them after.
- `[[stitching]]` grid points sit on multiples of `pitch` from the origin, so a thin edge strip
  `outline` that straddles no multiple places nothing (`crates/agentee-core/src/layout.rs`, the
  `(b.min / pitch).ceil() * pitch` start).
- `testpoints` put TP29's via inside F1.2 on buggy-guard, which made the fab notes ask to fill
  and cap it, and put TP30 at the same schematic spot as TP24 on the mcu sheet.

## Fab

- `fab-notes.txt` has no IPC class, finished hole tolerance, removal of non-functional pads or
  fab logo keepout, and the board file has no key to set them; practice.md asks for all four.

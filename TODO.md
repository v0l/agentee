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

## Library

- 3D model resolver: one chain behind `library add`, `models` and the 3D view that tries KiCad
  (project, local libs, kicad-packages3D) then vendor sources, and fits a vendor model to our
  footprint by matching pad numbers against the vendor's footprint (model offset and Z rotation).
  Sources tested on rotator's board (2026-10-12):
  - EasyEDA/LCSC, no account: JLC's `selectSmtComponentList` POST gives the LCSC code from an
    MPN, `easyeda.com/api/products/C258169/components` (User-Agent `curl/8.0`; the
    easyeda2kicad one gets 403) has the model uuid in the footprint's `SVGNODE`, and
    `modules.easyeda.com/qAxj6KHrDKw4blvCG8QJPs7Y/<uuid>` returns the STEP. Found MDB10S, none
    for USB4960-00-C. The same lookup could fill LCSC numbers in `bom-jlcpcb.csv`.
  - Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
    and Ultra Librarian need an account; their search endpoints return 403 without one. Until
    then, take their KiCad ZIP (`.kicad_mod` + STEP) as `--model`, fitted the same way.
- KiCad has no model for the footprint name on some parts, so `library add` stores none:
  `VQFN-24-1EP_4x4mm_P0.5mm_EP2.5x2.5mm` and `VQFN-28-1EP_5x5mm_P0.5mm_EP3.7x3.7mm` (rotator
  used the EP2.6 and EP3.35 bodies by hand). The resolver should fall back to the nearest body.
- An MPN with a space (`FC-135 32.7680KA-AG0`) is not matched to its library part by the
  `library` check; rotator's Y1 needed a `library` field.
- `library verify` panics in truck-geometry `bspcurve.rs:301` (`ZeroRange`) on some STEP models
  while checking rotator's library, and keeps going.
- The `library` check suggested a 330 uF capacitor for 100 nF parts before the passives were in
  the library (rotator, 2026-10-11).

## Edit (more)

- `edit pcb track ... --width 0.2621mm` on a net whose class width is 0.2621 mm fails
  `class-width` ("0.2621mm is narrower than the USB class width 0.2621mm"); leaving `--width` off
  works. The class width is `calc impedance`'s 0.26214.
- A batch that adds a part with `add` and then nets it in the same `edit sch` script fails with
  "no part R3" (rotator, 2026-10-11).

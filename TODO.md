# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library (taken: master)

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
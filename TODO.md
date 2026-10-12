# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Vendor model sources, behind `agentee_3d::resolve` (KiCad local, mirrors and nearest body are
  in; `library add --model vendor.zip` fits a vendor model by its `.kicad_mod` pads, ed90975):
  - EasyEDA/LCSC, no account: JLC's `selectSmtComponentList` POST gives the LCSC code from an
    MPN, `easyeda.com/api/products/C258169/components` (User-Agent `curl/8.0`) has the model uuid
    in the footprint's `SVGNODE`, and `modules.easyeda.com/qAxj6KHrDKw4blvCG8QJPs7Y/<uuid>`
    returns the STEP. Not built: the library's terms grant commercial use only inside
    JLCEDA/EasyEDA, so fetching it needs a decision first. The same lookup could fill LCSC
    numbers in `bom-jlcpcb.csv`.
  - Component Search Engine (SamacSys), SnapMagic (API key on request from
    snapeda.com/get-api) and Ultra Librarian need an account; their search endpoints return 403
    without one.

# Changelog

All notable changes to agentee are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Circuit sims check transient peak, RMS, frequency and THD, plot a spectrum, and check harmonic
  and conducted emissions. Varistor and potentiometer blocks, surge and burst drives, and a logic
  simulator.
- Op-amp models with per-rail headroom, slew rate, bias current, PSRR and 1/f noise, settling,
  and no windup on unnamed outputs.
- Through-hole op-amp preamp and Eurorack transistor ladder VCF examples.
- `edit sch` places and wires every unit of a multi-unit part.
- `own_voltage` on a net class, for nets that ride together.
- Slot widths are checked in footprints and by `library verify`.
- An optional topology stage in `layout`.
- `layout` reports progress on stderr, and sims show their solver stages.
- The icon is embedded in the Windows binary.
- `library fill` pins a missing datasheet and 3D model from the symbol's URL, the distributors,
  KiCad and EasyEDA. `--like` takes the EasyEDA model of a part in the same package.
- `library set --symbol` and `--footprint` swap a part's symbol or footprint.
- `library intake` adds the parts in a distributor bag to your own stock from its 2D barcode
  (DigiKey, Mouser, LCSC). Own stock and a shelf bin are recorded per part and subtracted from
  what needs ordering.
- `library assets` searches every source for a symbol, footprint, 3D model or datasheet:
  the KiCad libraries, EasyEDA, and the distributors. A source is one `AssetSource` impl.
- `library add --model easyeda:CODE` takes a model found by that search.
- A library tab in the viewer, with an on-shelf filter, footprint, 3D and symbol previews,
  and fix match and add part dialogs. Add part searches each of the four assets and previews
  the part beside the form.
- `assemble` and an assembly tab in the viewer step through the hand assembly groups of a
  layout.
- `edit board domain` and `barrier`, `edit sch remove net`, and globs in keepout nets.
- A `ground` key on circuit sims.
- `place` reports the parts it leaves off the board.

### Changed
- Tracks route to pads by the footprint's clearance, which holds inside the courtyard, and
  waivers cover pairs.
- The placer pulls bridging passives from both pins, keeps isolation domains on their side, and
  uses the back of the board under through-hole parts.
- The 3D view renders with wgpu, and FDTD runs on the sims field engine.
- Hiding a copper layer shows the others.
- Farnell queries back off on its rate limit, datasheet links stay with their maker, and
  `library find` keeps makers apart.

### Fixed
- Commas in `edit board` descriptions are kept.
- Holes and fiducials no longer warn about a missing pin, and KiCad MP pads need no symbol pin.
- Refilling zones keeps the plan's end marker, and a plan section with no end marker is refused.
- `edit sch` refuses a file that does not load, and tidies pin lists.
- `layout` places parts that have no footprint entry.
- A part a distributor lists as `NO_LONGER_MANUFACTURED` or not recommended is no longer treated
  as active, so `library find` and `library add` agree on stock.
- A BJT or MOSFET block reads its polarity from the symbol's description and keywords too, so an
  `MMBT3906` with no "PNP" in its value simulates as a PNP.
- Loading a project routes schematic wires 2.4 times faster (ubench `hardware` from 25 s to 10 s)
  by steering the wire search toward its targets, with the same wires.
- `library use` writes all of a part's files or none.
- `library add` points the symbol's datasheet at the pinned one.
- A footprint's 3D model that KiCad lacks is also looked for in Espressif's kicad-libraries, and
  a `--model` that cannot be found says which model replaced it.

## [0.3.0] - 2026-10-12

### Added
- `route` runs a negotiated detail router, with coupled pairs and via budgets. A pair's
  `max_uncoupled` defaults to 20% of its length.
- ERC by pin type.
- Analog circuit simulation.
- `datasheet` with a shared cache, a parts library with stock snapshots, and DigiKey beside
  Farnell (#12, #13, #14). `library find` looks in the library first, then at stocked parts.
  EasyEDA models and LCSC numbers come into the library.
- Placement rules for power, protection, feedback loops, sheets and board edges, thermal mounts,
  and model heights for the placer.
- Routing keepouts, per-finding DRC waivers, and a dangling via check.
- Fab notes for IPC class, hole tolerance, pads and marking.
- Zone and cutout outlines from point lists, `uncutout`, and one tie via per pour island.
- An open button in the viewer, and net names throughout `show pcb`.
- When a 3D model is missing the nearest KiCad body is used, and vendor models are fitted to the
  footprint by its pads.
- Board design practice for the agent skill, in `practice.md`.

### Changed
- The 3D view uses the egui_bench viewer.
- Routing steers away from filled and plugged vias.

### Fixed
- `tie` skips pads already joined to the plane (#9), stacked pads of one contact are not
  overlaps (#10), and locked parts stay obstacles (#11).
- Silk is checked again after the last label pass.
- Footprint holes inside their own pad must be filled.
- Imported pads that reach the board edge are marked.

## [0.2.0] - 2026-10-08

### Added
- One set of copper rules for DRC and for every tool that adds copper: route, tie, test points,
  fanout, stitching, tune, neck and the placer all keep to it, through keepouts and per-layer
  green zones.
- Isolation domains, barriers, creepage and spark gap footprints, with routing kept off the
  barriers.
- Net class voltages set spacing, with supplies, rails and ratings. Every net class needs one.
- Logic levels, STEP export, and NPTH clearance when routing.
- Parts turn to face their connections and are placed on the back, BGA decoupling included.
- Pin pours for runs of same-net pins, signal segments on plane layers, and plane cuts that give
  way to critical tracks.
- `layout --search` over engine settings, a beam search over engine stages, and a smallest-board
  search.
- A back view in the 2D layout, focused renders with rulers and crop, and the board title in the
  silk. The 3D view zooms toward the pointer and reloads changed models.
- Named batch edits and an MCP edit tool.
- Order sheets and a BOM cost table.
- The build hash in `--version`.

### Changed
- Routing uses fewer vias and retries nets that failed.
- The place stage is faster.

### Fixed
- Pour islands cut off from their net are dropped, and every open net is repaired after finish.
- Fanout vias fit inside their pad and skip pins beside a bigger same-net pad.
- Routes end on pad centres (#5) and price inner pours of other nets (#1). Test pads stub
  straight to copper (#2). `show` carries map and channel readings (#3). `untrack` takes only its
  net or span (#4). Touching pads of one net are one terminal (#6). Bottom parts are placed on
  the bottom (#7). Hole gaps count slots (#8).
- Farnell parsing, and a repeated `--field` keeps every value.

## [0.1.0] - 2026-10-06

First release.

### Added
- Designs as plain TOML: board specs with stackup, fab rules, vias and net classes, schematic
  symbols, footprints, schematics, layouts and FDTD simulations.
- `check` for the whole project: stackup, impedance and current per net class and layer,
  connectivity and clearance, exiting 1 on errors.
- `agentee edit` for schematics, layouts and board specs, keeping the file's comments and layout,
  one command at a time or a list at once.
- KiCad symbol and footprint import and search, and KiCad 3D model download.
- Impedance and trace width calculators, a GPU field solver for trace cross-sections, and a GPU
  3D FDTD that writes S-parameters as JSON and Touchstone.
- Zone fill, ground and supply ties, autorouting, and `agentee layout`, an engine that places
  and routes a board.
- A live viewer that reloads on save, with hand placement, routing and vias saved back into the
  TOML, and a 3D view. `render` writes the same picture as a PNG.
- An MCP server and an agent skill.
- Demo, LNA, HDI and HackRF Pro examples.

[Unreleased]: https://github.com/v0l/agentee/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/v0l/agentee/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/v0l/agentee/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/v0l/agentee/releases/tag/v0.1.0

---
name: agentee
description: Design and iterate on electronics with agentee, where boards, symbols, footprints, schematics, layouts and simulations are plain TOML files checked by the `agentee` CLI or MCP server. Use when creating or editing `*.board.toml`, `*.sym.toml`, `*.fp.toml`, `*.sch.toml`, `*.pcb.toml` or `*.sim.toml` files, importing KiCad parts, fixing `agentee check` errors, routing a layout, sizing traces for impedance or current, running FDTD, cascade, channel, PDN, DC drop or thermal sims, or writing a fab package.
---

# agentee

A project is a directory of TOML files. You write the files; `agentee` loads everything under
the directory, checks it, computes what the stackup gives, and renders it. There is no editor
state to sync: the files are the design.

The full key-by-key reference is `agentee docs` (MCP `format_reference`). Read the section for
the file kind you are about to write before writing it. This skill covers how to work, not every
key.

## The loop

Every edit goes through the same four steps. Do not batch several unchecked edits.

1. Edit one file.
2. `agentee check` (exit 0 clean, 1 errors, 2 load failure). Add `--item pcb:NAME` to scope it,
   `--info` for notes, `--json` for machine output.
3. `agentee render KIND:NAME -o /tmp/x.png` and look at the PNG. Check passing does not mean it
   looks right: crowded silk, a part on the wrong side of the line, or a detour in a track only
   show up in the picture.
4. `agentee show KIND:NAME` when you need numbers: pin positions, net lengths and delays,
   solved trace widths, sim readings.

Every diagnostic names the file, the item and the place (`tracks[1] RF_IN`, `U1.1`, a coordinate).
Fix the named thing, rerun check. When a silk label fails, check prints a `label = { at = [...] }`
line that passes every rule; paste it.

Name items with their kind whenever names collide, which they do by design (`lna` is a board, a
schematic and a layout): `board:lna`, `sch:lna`, `pcb:lna`, `sim:lna-rf`, `sym:R`, `fp:R_0402_1005Metric`.

`agentee view` opens a live window that reloads on save. Start it for the human if they are
watching; you work from `render` and `show`.

## Starting a project

```sh
agentee new board NAME          # fab, stackup, outline, vias, net classes
agentee new schematic NAME
agentee new layout NAME
agentee check
```

The schematic and layout starters are only a `name`. Add `board = "NAME"` to the schematic and
`board` plus `schematic` to the layout before anything else.

Past a few dozen parts, give each section its own schematic (power, MCU, RF, ...) and list them
in a top schematic with `sheets = [...]`; the layout places the top one. Nets join across sheets
by name.

Order of work, each stage passing check before the next:

1. **Board spec.** Pick `fab` (`jlcpcb` or `generic`) and a stackup preset. Define `Default` and
   one net class per kind of net (RF, power, pairs). Put impedance and current targets on the
   class and let check solve the widths; `agentee show board:NAME` prints them per layer.
2. **Parts.** Import rather than draw (see below). Every symbol pin number needs a pad of the same
   number in its footprint.
3. **Schematic.** Place parts, list nets as `REF.PIN`. Leave `wires` out and agentee routes them.
   Put deliberately open pins in `no_connect`.
4. **Layout.** Place footprints, add zones, then tracks and vias net by net. Check reports the
   ratsnest for every unrouted connection, so route until `unrouted` is 0 on every net in
   `show pcb:NAME`.
5. **Simulate** what the design depends on (see below).
6. **Fab.** `agentee fab pcb:NAME -o fab/` once check has no errors.

Write a `DESIGN.md` next to the files as you go: the circuit, why each part was chosen, the layout
rules the circuit needs, and what is still unverified. `examples/lna/DESIGN.md` is the model.

## Parts from KiCad

```sh
agentee search symbol lm358                           # every word must match Library:Name
agentee search footprint soic 3.9x4.9
agentee import symbol Amplifier_Operational:LM358 --with-footprint
agentee import symbol Device:R --footprint Resistor_SMD:R_0402_1005Metric
agentee import footprint Package_TO_SOT_SMD:SOT-89-3
```

Imports land in `symbols/` and `footprints/`. `--force` overwrites. Run check straight after:
KiCad silk is often 0.12 mm and JLCPCB wants 0.15 mm, so widen it in the imported `.fp.toml`.
`agentee models` fetches the 3D models the footprints name.

When KiCad lacks the part, `agentee new symbol NAME` / `new footprint NAME` and build it from the
datasheet. Use `[[bodies]]` with per-side pin lists for box symbols and pad rows (`count`,
`pitch`) for footprints instead of listing every pin or pad.

## Traps

- **Y grows down** in symbols, footprints, schematics and layouts. Rotation is counter-clockwise
  on screen.
- Bare numbers are millimetres. Anything else needs a unit string: `"8mil"`, `"35um"`, `"1oz"`.
- Unknown keys are errors. If check says `unknown field`, it lists the valid ones.
- Schematic parts and pins sit on the 1.27 mm grid. Off-grid points are flagged.
- A track has to run into a pad. One that only grazes the pad edge is flagged even though the
  copper touches.
- Pad nets come from the schematic by pin number. Change a net in the schematic, not by drawing
  copper.
- `side = "bottom"` on a footprint mirrors it and swaps F./B. layers; place its coordinates as
  seen from the top.

## Placing for assembly, handling and test

Check reports these as notices, not errors: they are practice, not fab limits. Follow them
unless the design gives a reason not to, and say why in DESIGN.md when you don't.

- **Keep bodies off the edge.** Every part body at least 1 mm from the outline, so handling,
  depaneling and enclosures don't knock parts off. Edge-launch connectors, castellations and
  mounting holes are the exceptions (`edge = true` pads, `overhang = true` footprints).
- **Ceramic caps crack where the board bends.** Within about 5 mm of an edge, a corner, a
  mounting hole or a V-cut the board flexes when it is broken out of the panel, screwed down or
  handled (Knowles, Murata). Keep MLCCs out of that zone; one that has to be there lies with its
  long axis parallel to the nearest edge. Never put 0805 and larger MLCCs in it.
- **Tombstoning.** A small passive (0603 and down) lifts one end in reflow when its ends heat
  unevenly. Give both pads the same size, the same copper (both on thin tracks, or both into
  the pour through the same relief), and via-in-pad on both ends or neither. Keep small parts
  at least a tall neighbour's height away from it, so it doesn't shade them in the oven.
- **Test pads, always.** Every board needs probe access for bring-up and a pogo-pin fixture for
  production test. Put a test pad on every power rail and ground, every reset, enable, power
  good and boot or strap pin, every clock, and each low-speed bus line (UART, SPI, I2C, SWD,
  JTAG where there is no connector). Keep them on one side (the bottom by default) so one
  fixture reaches all of them: round pads of 1 mm or more, centres at least 1.27 mm apart (2.54 mm
  for 100 mil pogo pins), 1 mm from parts and 3 mm from the edge and tooling holes, not under
  parts, never on high-speed pairs or RF lines (the stub hurts them). Use `TestPoint_Pad_D1.0mm`
  style footprints and name them `TP1...`.
- **Fiducials and tooling holes.** Two or three fiducials per side that has fine-pitch parts,
  and non-plated tooling holes if the board is tested in a fixture.
- Every fab package carries `agentee vX.Y.Z-hash` in silk; leave room for it.

## Sizing traces

```sh
agentee calc impedance --layer F.Cu --target 50ohm
agentee calc impedance --layer F.Cu --target 90ohm --gap 0.15mm
agentee calc field --netclass RF                     # GPU 2D solve, mask and copper thickness
agentee calc field --netclass RF --sweep 10MHz,6GHz,21   # loss per frequency
agentee calc trace-width --current 2A
agentee calc serpentine --from 10,5 --to 20,5 --add 2.5mm
```

The closed forms ignore mask and copper thickness and can be several ohms off on thin prepreg.
For any class that matters, set `solver = "field"` on it; check then uses the field solver and
suggests the width that meets the target.

## Simulation

Each `*.sim.toml` has a `kind`: FDTD (default), `cascade`, `channel`, `pdn`, `dc`, `thermal`.
`agentee sim NAME` writes `NAME.result.json` (and `NAME.sNp` for S-parameters) next to the spec.
Read the result with `agentee show sim:NAME`: the `readings` list is the summary (gain, match,
NF, stability, eye height, peak temperature, drop). Look at the plots with `agentee render sim:NAME`.

FDTD is expensive. The LNA example's six-port run took 27 minutes on a workstation GPU. So:

- `agentee sim NAME --dry-run` first. It prints the grid, time step and run count in a second.
- Iterate at `cell = 0.1` with a narrow `region` and only the ports you need in `excite`, then
  tighten once the design is settled.
- Run long sims in the background and keep editing other things.
- DC and thermal take seconds. Run them freely.

A result goes stale when its spec, the layout, or an input result changes, and check warns. A
cascade, PDN or channel sim reads another sim's result, so rerun the FDTD before it. Cascade and
PDN need every port of the board sim driven: leave `excite` out there.

Post-process any S-parameter result without rerunning it:

```sh
agentee sparam NAME --tdr IN --rise 50ps
agentee sparam NAME --pair 1,2,3,4
agentee sparam NAME --xtalk 1,3
```

The usual split for RF: an FDTD of the passive board with a port where each active part or
measured component sits, then a `cascade` that drops in the vendor `.s2p` files and datasheet NF
and OIP3. Change the board, rerun the FDTD; change a part, rerun only the cascade.

## MCP

`agentee mcp <project>` serves the same operations on stdio: `format_reference`, `check`,
`list_items`, `show_item`, `render_item` (returns the PNG inline), `run_sim`, `sparam`,
`field_solve`, `impedance`, `trace_width`, `serpentine`, `kicad_search`,
`import_kicad_symbol`, `import_kicad_footprint`, `new_item`, `models`, `fab`. You still write the
TOML files yourself with your normal file tools.

## Worked examples

- `examples/demo`: a small board and part library that passes check.
- `examples/lna`: a full design. Board spec with a field-solved coplanar RF class, schematic,
  routed four layer layout, FDTD, cascade with vendor data, DC drop and thermal sims, and
  `DESIGN.md`. Copy its patterns before inventing new ones.

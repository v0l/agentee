---
name: agentee
description: Design and iterate on electronics with agentee, where boards, symbols, footprints, schematics, layouts and simulations are plain TOML files checked by the `agentee` CLI or MCP server. Use when creating or editing a design with `agentee edit` (sch, pcb, board commands), or when hand-writing `*.board.toml`, `*.sym.toml`, `*.fp.toml`, `*.sch.toml`, `*.pcb.toml` or `*.sim.toml` files, importing KiCad parts, fixing `agentee check` errors, routing a layout, sizing traces for impedance or current, running FDTD, cascade, channel, PDN, DC drop or thermal sims, or writing a fab package.
---

# agentee

A project is a directory of TOML files. You write the files; `agentee` loads everything under
the directory, checks it, computes what the stackup gives, and renders it. There is no editor
state to sync: the files are the design. `agentee edit` writes those files for you.

Change schematics, layouts and board specs with `agentee edit`, and send a change of more than
one command as one batch (`agentee edit sch NAME -` with the commands on stdin). It keeps the
comments and layout of the file, refuses a typo before writing, and checks what it changed.
Reach for a text editor only for a key it has no command for, and for symbols and footprints,
which it does not cover. See "Editing with commands" below.

Never parse and rewrite the TOML from a Python or shell script. It drops comments, skips the
checks `edit` does, and turns a typo into a new net or part. When a change is too repetitive to
type, have the script print `agentee edit` commands and pipe them in (see below).

## Installing

Run `agentee --version` first. If it is missing, install the release build:

```sh
curl -fsSL https://agentee.sh/install.sh | sh          # Linux and macOS, into ~/.local/bin
irm https://agentee.sh/install.ps1 | iex               # Windows PowerShell
cargo install --git https://github.com/v0l/agentee agentee   # anything else, Rust 1.92+
```

Rerun the same command to update. `search` and `import` read the KiCad libraries from
`/usr/share/kicad` (Debian and Ubuntu package them as `kicad-symbols` and `kicad-footprints`) or
from KiCad.app on macOS; set `KICAD9_SYMBOL_DIR` and `KICAD9_FOOTPRINT_DIR` for anywhere else.
FDTD and thermal sims need a GPU through Vulkan, Metal or DX12; the 2D field solver falls back to
the CPU. Rendering to PNG runs headless. Full per-platform notes are at https://agentee.sh.

To use it over MCP instead of the shell:

```json
{ "mcpServers": { "agentee": { "command": "agentee", "args": ["mcp", "/path/to/project"] } } }
```

## Reference

The full key-by-key reference is `agentee docs` (MCP `format_reference`). Read the section for
the file kind you are about to write before writing it. This skill covers how to work, not every
key.

## The loop

Every change goes through the same four steps. A change is one batch of `agentee edit` commands
(a whole sheet, a bus, a row of parts) or one hand edit of a file. Do not stack several hand
edits before checking them.

1. Make the change, with `agentee edit` where there is a command for it.
2. `agentee check` (exit 0 clean, 1 errors, 2 load failure). Add `--item pcb:NAME` to scope it,
   `--info` for notes, `--json` for machine output. An `agentee edit` already ran this for the
   files it touched and printed the result, so `check` is for anything else you changed.
3. `agentee render KIND:NAME -o /tmp/x.png` and look at the PNG. Check passing does not mean it
   looks right: crowded silk, a part on the wrong side of the line, or a detour in a track only
   show up in the picture.
4. `agentee show KIND:NAME` when you need numbers: pin positions, net lengths and delays,
   solved trace widths, sim readings.

`agentee list` names every item with its error count. `agentee drc NAME` prints a layout's rule
messages alone, and `--list` every rule id with whether it applies to this board; turn one off
with `[drc] disable = ["id"]` in the board file.

Every diagnostic names the file, the item and the place (`tracks[1] RF_IN`, `U1.1`, a coordinate).
Fix the named thing, rerun check. When a silk label fails, check prints a `label = { at = [...] }`
line that passes every rule; `agentee silk NAME` pastes them all for you, and `--hide` hides the
ones with nowhere to go.

Name items with their kind whenever names collide, which they do by design (`lna` is a board, a
schematic and a layout): `board:lna`, `sch:lna`, `pcb:lna`, `sim:lna-rf`, `sym:R`, `fp:R_0402_1005Metric`.

A whole schematic or board at 1400 px is too small to read. Render the part you are asking about:

```sh
agentee render pcb:NAME -o /tmp/x.png --canvas-only --rulers          # mm labels on the edges
agentee render pcb:NAME -o /tmp/x.png --canvas-only --region 10,5,30,20
agentee render sch:NAME -o /tmp/x.png --canvas-only --focus U3,SPI_*   # zoom to them, dim the rest
agentee render pcb:NAME -o /tmp/x.png --canvas-only --focus U3.4 --context hide
```

`--focus` takes references, net names and `REF.PIN`, with `*` as a wildcard, on schematics and
layouts. A net brings in the parts it touches. `--context` is `dim` (default), `hide` to leave
only the focus, or `show` to just zoom. Start from an overview with `--rulers`, then pass the
coordinates you read off it to `--region`. With `--canvas-only` the PNG is cropped to the drawing,
so `--width` and `--height` are the largest it gets. Combine with `--hide`/`--show` to look at one
copper layer.

`agentee view` opens a live window that reloads on save. Start it for the human if they are
watching; you work from `render` and `show`.

## Starting a project

```sh
agentee new board NAME          # fab, stackup, outline, vias, net classes
agentee new schematic NAME
agentee new layout NAME
agentee new sim NAME --kind logic   # a logic sim of the schematic; default --kind fdtd
agentee check
```

The schematic and layout starters are only a `name`. Add `board = "NAME"` to the schematic and
`board` plus `schematic` to the layout before anything else.

Past a few dozen parts, give each section its own schematic (power, MCU, RF, ...) and list them
in a top schematic with `sheets = [...]`; the layout places the top one. Nets join across sheets
by name.

Order of work, each stage passing check before the next:

1. **Board spec.** Pick `fab` (`jlcpcb`, `generic`, or `hdi` for microvia boards) and a stackup
   preset from `agentee stackups --fab jlcpcb --layers 4` (or `pcbway`, `generic`) rather than
   typing layers in. Define `Default` and one net class per kind of net (RF, power, pairs). Put
   impedance and current targets on the class and let check solve the widths; `agentee show
   board:NAME` prints them per layer. Give every class a `voltage`, the highest its nets reach
   (`"0VDC"` for ground, `"3.3VDC"` for 3.3 V logic, `"48VDC"`, `"230VAC"`); a class without one
   is a warning. It sets the clearance and creepage to every other class (reinforced from SELV),
   defaults DC sim supplies, makes the power nets of the class rails for the level check (a rail
   named for its voltage, `1V8`, `3V3_PLL`, takes that voltage instead) and checks capacitor
   ratings. Above SELV, one class per conductor that can differ: `DC+`/`DC-` with signed
   voltages, `L1`, `L2`, `L3` each `"230VAC"`, never two phases in one class. Do not write `[[domains]]` or `[[barriers]]`
   for what a voltage already covers.
   `agentee edit board` does all of this, and check names the width a class needs.
2. **Parts.** Import rather than draw (see below). Every symbol pin number needs a pad of the same
   number in its footprint.
3. **Schematic.** Place parts, list nets as `REF.PIN`, and give every net a `class`; a net left in
   `Default` is a warning. Leave `wires` out and agentee routes them. `agentee edit sch` does all
   of this, one batch of commands for the whole sheet. Put deliberately open pins in `no_connect`.
   Give the symbols of logic and MCU parts a `[levels]` table from the datasheet (VIH, VIL,
   limits, leakage) and the schematic a `rails` table, and check flags dividers that land between
   VIL and VIH, floating inputs, pulls too weak for the leakage, and overdriven pins. List ADC
   inputs in `analog`.
4. **Layout.** Place footprints, add zones, then tracks and vias net by net. `agentee edit pcb`
   writes a placement, a track, a via or a zone by hand. The automatic tools are usually better
   for anything with many connections, and each takes `--dry-run`:
   - `agentee place NAME` places every part (`--parts 'U*'`, `--keep-placed`, `--seed N`). It keeps
     parts in different domains a barrier apart and clear of copper it is not moving.
   - `agentee fit NAME` finds the smallest board the placed layout fits on, per aspect ratio,
     checked with the global router; `--strategy tight|balanced|spread` trades size for routing
     room, `--width` or `--height` holds a side, `--write` resizes the board and writes the
     placement. Use it instead of shrinking the outline by hand and replacing.
   - `agentee pinswap NAME --part U1 --write` swaps a chip's interchangeable I/O to untangle it.
   - `agentee tie NAME` stubs every SMD pad of a plane net to its plane with a via. A pad with a
     via close by, or whose pour already joins a plated hole of its net beside it, is left alone.
   - `agentee route NAME --nets 'SPI_*'` routes those nets (`--pairs`, `--reroute`). Each track
     ends on its pad centre, and a run across an inner-layer pour of another net costs five times
     as much, so planes are slotted only where no other layer fits.
   - `agentee tune NAME` meanders pairs over their skew and match groups short of their length.
   - `agentee neck NAME` necks tracks down where they enter a narrower pad.
   - `agentee fill NAME` fills the zones and stores the copper in the file.
   - `agentee layout NAME` runs the whole engine (place, access, route, finish) from the
     layout's `[engine]` settings; `--from`, `--to` and `--only` pick stages.
   - `agentee layout NAME --search` is how to look for a better result. It is a beam search over
     the stages in memory: 16 placement seeds by default, the best 4 carried through routing from
     the global route on, the winner written with its seed pinned in `[engine.place]`. Knobs of
     later stages in `[engine.search]` (`detail.via_cost`) branch every kept placement at that
     stage (see `docs/layout-engine-2.md`). `--search 40 --keep 8` widens it. Do not loop
     `place --seed N` and `route` by hand: one search call covers what a dozen manual rounds
     would.

   Check reports the ratsnest for every unrouted connection, so route until `unrouted` is 0 on
   every net in `show pcb:NAME`.
5. **Simulate** what the design depends on (see below).
6. **Fab.** Put `title = "NAME v1.0"` at the top of the layout so the silk names the board and
   its version, then `agentee fab pcb:NAME -o fab/` once check has no errors.
7. **Enclosure.** `agentee export pcb:NAME -o NAME.step` writes the board solid and every part
   model as one STEP assembly to design a case around.

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
KiCad silk is often 0.12 mm and JLCPCB wants 0.15 mm, so check names the footprints to widen.
Change the `width` of their silk `[[graphics]]` in a text editor, one file at a time.
`agentee models` fetches the 3D models the footprints name.

`agentee import board path/NAME.kicad_pcb --dir DIR` turns a whole KiCad board into a project:
board spec, layout, a netlist schematic, footprints and box symbols.

When KiCad lacks the part, `agentee new symbol NAME` / `new footprint NAME` and build it from the
datasheet. Use `[[bodies]]` with per-side pin lists for box symbols and pad rows (`count`,
`pitch`) for footprints instead of listing every pin or pad.

## Editing with commands

`agentee edit TARGET ITEM COMMAND [args]`, where TARGET is `sch` (schematic), `pcb` (layout) or
`board` (board spec). `agentee edit sch help` (or `pcb`, `board`) lists every command with its
arguments. Every command below is on one of those three.

```sh
agentee edit sch NAME add R1 R 10k --footprint R_0402_1005Metric --at 25.4,25.4
agentee edit sch NAME add C1 C 100n --footprint C_0402_1005Metric
agentee edit sch NAME net VBUS C1.1 U1.7 --class Power
agentee edit sch NAME nc U2.3
agentee edit sch NAME --list
agentee edit sch NAME --json add R1 R 10k     # the resolved pins and coordinates as JSON
```

**Schematic** (`add`, `remove`, `move`, `set`, `net`, `connect`, `disconnect`, `nc`, `unnc`,
`note`). A pin is `REF.PIN`, by number (`U1.3`) or by a unique pin name (`U1.VCC`); a name several
pins share is an error listing the numbers. A pin given a net it is already on is moved to the
new net, not duplicated. `add` puts the part to the right of everything there on the 1.27 mm
grid; `--at X,Y` overrides that and snaps to the grid. `--footprint` names the footprint, and if
the symbol is missing but a symbol in the project uses that footprint, that one is used.

**Layout** (`place`, `unplace`, `track NET LAYER X,Y X,Y ...`, `untrack`, `via NET X,Y`, `unvia`,
`zone NET --layers ...`, `unzone`, `pair`, `text`, `fanout`, `stitch`, `watermark`, `test`,
`board`, `schematic`). `place` takes a part of the layout's schematic or of any sheet that
schematic lists, at any depth, and says which sheet it found it on. A net, layer or via name
that is not in the schematic or its sheets is an error naming what is, so a typo never becomes a
net or a placement of its own. `untrack NET` drops every track of that net; `untrack X,Y X,Y`
(optionally after a net) cuts the span between two points of a track and keeps the rest.

**Board** (`class NAME --track-width ... --impedance ... --via ...`, `unclass`, `via NAME --drill
... --diameter ...`, `unvia`, `outline`, `cutout`, `stackup`). `class` edits the netclass of that
name in place or adds it. `stackup --preset NAME` takes a name from `agentee stackups`.

Pipe the commands in, or write them to a file, whenever a change is more than one command. It
is one load and one check at the end, so it is much faster, and the half-built states that check
would flag between single commands (a part with no net yet) never happen:

```sh
agentee edit sch power - <<'EOF'
add R1 R 10k --footprint R_0402_1005Metric
add R2 R 4k7
add C1 C 100n --footprint C_0402_1005Metric
net MID R1.2 R2.1 C1.1 --class Signal
nc R2.2
note "input divider"
EOF
```

`agentee edit sch power build.txt` does the same from a file. The same works for `pcb` and
`board`. The item name can be left out when the project has only one item of that kind; with
more, the error names them. Lines starting with `#` are skipped.

A batch has no loops. For anything repetitive (16 decoupling caps, a `D0..D15` bus, a row of
placements at a pitch) generate the lines and pipe them in. Doing the coordinate arithmetic in a
script is right; editing the TOML from it is not:

```sh
for i in $(seq 0 15); do
  echo "add C$((10+i)) C 100n --footprint C_0402_1005Metric"
  echo "net 3V3 C$((10+i)).1"
  echo "net GND C$((10+i)).2"
done | agentee edit sch power -

python3 -c 'for i in range(16): print(f"place C{10+i} {20+i*1.5:.2f},12 --rotation 90")' \
  | agentee edit pcb main -
```

Each edit writes the file, refills a layout's stored zone fills if the layout stored them, and
prints the check diagnostics of the files it touched. It exits 1 if any of those is an error, so a
batch that leaves the design broken fails the same way `check` does. A pin or symbol that does
not exist, a flag it has no meaning for, or a value it cannot read is an error before anything is
written, so the file is untouched when a command is wrong.

`--class` is only checked against the board once there is one. Build a schematic before its board
and the class is written as given; add the netclass to the board before you route.

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
  parts, never on high-speed pairs or RF lines (the stub hurts them). `agentee testpoints NAME`
  adds a `TP` part to the schematic and a routed `TestPoint_Pad_D1.0mm` pad to the layout for
  each net that has no probe access (`--nets`, `--side B`, `--pitch 2.54`).
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

Each `*.sim.toml` has a `kind`: FDTD (default), `cascade`, `channel`, `pdn`, `dc`, `thermal`,
`logic`. A logic sim runs a schematic's digital parts and writes a VCD as well as the result.
`agentee sim NAME` writes `NAME.result.json` (and `NAME.sNp` for S-parameters) next to the spec.
Read the result with `agentee show sim:NAME`: `result.readings` is the summary (gain, match,
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

## Parts and prices

```sh
agentee bom NAME                       # cost table at 1, 10 and 100 boards, total and per board
agentee bom NAME --boards 5,50 --json
agentee parts NAME --boards 5          # stock, price and cheaper drop-ins per BOM line
agentee parts NAME --refs C11 --json
agentee parts NAME --boards 2 --spares --order docs/   # order sheets per distributor
```

Give every part `mfr` and `mpn` fields first; lines without an `mpn` are not looked up. Keys
go in `~/.config/agentee/distributors.toml` (`[mouser] api_key`, `[farnell] api_key` and
`store`), never in the project. Alternatives keep value, package and ratings for resistors,
ceramics, generic discretes and LEDs; ICs and connectors only get the same part elsewhere or the
distributor's suggested replacement. Write the swap into the part's `mpn`, not the BOM.
`--order` writes `NAME-order.csv` and a sheet per distributor with the lines to buy there on top
and what it lacks at the bottom. Put off-board parts (housings, crimps, antennas) in a
`buy_with` field on the part that needs them, never in a separate list.

## MCP

`agentee mcp <project>` serves the same operations on stdio: `format_reference`, `check`, `drc`,
`list_items`, `show_item`, `render_item` (returns the PNG inline, takes `focus`, `context`,
`region` and `rulers`), `stackups`, `run_sim`, `sparam`, `field_solve`, `impedance`,
`trace_width`, `serpentine`, `place`, `route`, `tie`, `fill`, `tune`, `neck`, `silk`,
`testpoints`, `layout`, `fit`, `parts`, `kicad_search`, `import_kicad_symbol`,
`import_kicad_footprint`, `new_item`, `models`, `fab`, `export`, `edit`. `edit` takes `target`
(`sch`, `pcb` or `board`), `item`, and `commands`: the same batch as `agentee edit`, one command
per line. Send `commands = "help"` for the list.

## Worked examples

- `examples/demo`: a small board and part library that passes check.
- `examples/lna`: a full design. Board spec with a field-solved coplanar RF class, schematic,
  routed four layer layout, FDTD, cascade with vendor data, DC drop and thermal sims, and
  `DESIGN.md`. Copy its patterns before inventing new ones.
- `examples/sdr`: a large design. Hierarchical schematic (power, clock, FPGA, RF, USB sheets),
  BGAs, USB 3 pairs with an FDTD and eye, DC drop on the core rail, thermal, and `DESIGN.md`.
- `examples/hdi`: a 1+4+1 HDI coupon with microvias in the pads of a 0.5 mm BGA.
- `examples/logic`: logic sims of a counter and an I2C write, with their VCDs.
- `examples/hackrf-pro`: the HackRF Pro main board, imported from KiCad with `import board`.

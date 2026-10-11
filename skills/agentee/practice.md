# Board design practice

Judgement that `agentee check` cannot make for you: which stackup, which part, where it goes and
what the rules should be. Distilled from Phil's Lab (#60, #75, #88, #93, #95, #114, #131, #137,
#156, #170, #171, #178) and Robert Feranec's FEDEVEL talks with Rick Hartley, Eric Bogatin, Keith
Armstrong, Hans Rosenberg, Min Zhang and Art Kay. Check already enforces mask webs, tombstone
risk, starved thermals, fiducials, return vias on interfaces, flex zones and edge clearance, so
those are not repeated here. Where a rule below is broken on purpose, say why in DESIGN.md.

Treat app notes and evaluation board layouts as unverified. Hartley puts about 95% of switcher
layout app notes in the poor category, and the three-value decoupling ladder and split grounds
both came from them. When one disagrees with this file, follow the principle and note it.

## Stackup and rules

- Pick the stackup before placing anything, from `agentee stackups`: the fab's standard build and
  its impedance widths. Rules and impedance widths follow from it.
- Four layers by default, signals outside and two ground planes inside on thin prepreg. Two
  layers only for simple, cost-driven boards made in large volume; at 10k boards 2L saves about
  10 cents, which a few hours of layout or a failed EMC test outweighs. If 2L, use 0.8 mm (a
  standard core, no cost adder): 1.6 mm puts the plane too far away for any impedance control.
- Every signal layer gets a solid reference plane next to it. Never two signal layers sharing one
  plane on the outside (S-S-G-S): the coupling is too small to break signals but plenty to drive
  common-mode current off the board. Inside, between two grounds, a signal pair is fine up to a
  few hundred MHz if routed exactly orthogonal.
- Reference signals to ground. Under about 10 A a digital board needs no power plane: route power
  as wide copper (a 6 mil trace carries 1 A, 100 mil carries 10 A) and make the layer another
  ground. Plane capacitance is not a reason for a power plane; it is tiny next to discrete caps.
- When a power plane is needed: put it within 0.2 to 0.25 mm of a ground plane so the pair acts as
  an HF capacitor, never put a signal layer between power and ground unless another ground sits
  right beside the power plane, never put two power planes next to each other, and keep critical
  power-ground pairs near the surface on the side of the chip that draws it (vias are the highest
  inductance in a stack). A signal referenced to power should reference the rail that drives it.
- Keep the stackup symmetrical with similar connected copper on each layer so it does not warp
  in reflow.
- Stay well above the fab minimum. Global track and space 0.127 mm (5 mil) when fine-pitch parts
  need it, 0.2 to 0.3 mm when the board is 0603 and SOIC. Relax only around the part that needs
  it. Drill 0.3 mm (0.25 mm minimum), board edge 0.5 mm, mask expansion 0.05 to 0.1 mm set once.
- A cheap 2L build wants 0.15 mm track and space and 0.3 mm holes; anything finer is a cost adder.
  Thick boards (over 2.4 mm, 16+ layers) need larger minimum holes; heavy copper cannot do fine
  tracks, so fine-pitch BGAs and heavy copper usually mean two boards.
- Every process option costs: ENIG over HASL, non-green mask, 2 oz copper, filled vias,
  castellations, gold fingers, HDI. Use one only when a part or interface needs it. Gold fingers
  must run to the board edge with no tooling strip there.
- Edges set the frequency content, not the clock: about 0.5 / rise time, and with no rise time in
  the datasheet or IBIS model assume 1 / (10 x clock). A 10 MHz bus with 0.5 ns edges has 1 GHz in
  it. Controlled impedance (with `solver = "field"`) on nets that need it whatever their length;
  only those few, the rest get slowed edges instead.

## Ground and return paths

- One ground net and one unbroken plane. Do not split ground for mixed signal, for a switcher, or
  because an app note says so; separate by placement. Split planes joined at one point or by a
  ferrite make a dipole antenna and push ESD through the chip. The only real reasons to separate
  are galvanic isolation and antennas.
- Above a few kHz the return flows on the nearest plane directly under the trace. A trace over a
  slot lost 20 dB at 4 GHz in Rosenberg's test; bridging the slot only under the trace fixed it.
  Below about 100 kHz (audio) the return spreads through the whole board.
- Every ground pin, IC, connector or capacitor, gets its own via. A shared via between RF, PLL and
  digital ground pins took an FM chip from 8 uV to 19 uV sensitivity at clock harmonics.
- A ground via beside every signal via that changes reference plane. None is needed passing from
  one side of a single plane to the other. In a dense via field, one ground via in the middle and
  a few around the edge. Between two different-voltage planes no via is possible, so keep those
  planes close and add DC-blocking caps near the transitions.
- Star grounding is a tube-era workaround; on a board it is long inductive traces and nested
  loops. The exception is a high-current, high-dynamic-range circuit (power amp output stage, a
  precision meter): keep one plane but place a virtual star point so the big currents and the
  input stage sit on opposite sides of it.
- Don't pour copper by default. Over a solid plane it adds nothing, and an unstitched pour
  resonates: Bogatin measured about 20 times the near-field emission from a flooded 2L board versus
  the same circuit with a solid plane. Pour on purpose: coplanar ground between signals when two
  layers share one plane, or alternating ground and power pours on signal layers to repair a weak
  stackup (8 to 10 dB in Lee Ritchey's case). Stitch any pour and any pair of ground planes at
  most lambda / 6 apart at the highest frequency that matters (about 25 mm at 1 GHz in FR4),
  empty regions included. Keep pours 3H from impedance traces unless the class is coplanar.

## Parts

- Decoupling is about loop inductance, not value. Size the minimum with C = I x t / dV; above it
  the value does not matter. Use the highest value the smallest body holds (10 uF 0402 has the
  same inductance and about the same price as 100 nF 0402), plus a few bulk caps elsewhere on the
  rail for slow droop. The 10 nF, 100 nF, 1 uF ladder is a through-hole leftover.
- A ferrite bead does not fix rail noise from switching IO; decoupling does. Beads are for
  low-current sensitive rails (PLL, ADC reference, analog). A bead alone rings with the pin's
  capacitance; always bead plus a cap on the device side, with the pole at least 10 times below
  the switcher and clock frequencies. Skip it when the part tolerates 5 to 10% ripple, and use a
  local LDO when it needs under 1%.
- Class 2 MLCCs (X5R, X7R) lose capacitance with DC bias: 100 nF 10 V is about 60 nF at 3.3 V.
  Rate at least twice the applied voltage and read the bias curve before sizing a value. An MLCC
  can resonate with a few nH of trace; damp it or change the value.
- C0G for crystal load caps, filter and timing caps and the audio signal path.
- Connectors: a return pin for every signal pin where possible; budget the grounds before
  choosing the connector size (18 signals wants 36 pins, not 20). Sharing, one ground every 3 or
  4 signals is far better than one per row. Ribbon cables alternate signal and ground.
- Choose quiet chips. Equivalent MCUs can differ by 40 dB in emissions; sweep a loop probe over
  each eval board on a spectrum analyzer and compare. An external-FET switcher controller lets
  you lay out the hot loop; prototype and EMI-test integrated-FET parts before committing.
- Consolidate the BOM: one 100 nF part for every rail, one pull-up value (10 k on I2C runs the
  bus slower, which is usually fine), two of an existing value in parallel instead of a new line.
  Each line is quoted, bought as a full reel, shipped and loaded. Prefer JLC basic parts there.
- 0603 passives by default, 0402 when space needs it, 0201 only under BGAs.
- Leaded packages over QFN, QFN over BGA, when the part comes in more than one. A BGA needs a fab
  and assembler that can do its pitch, 0201 and X-ray: confirm before layout.
- Use the reel part number (with pick-and-place cap where offered), not bulk or tube.
- SMD parts taller than the pick-and-place limit (an 18 mm electrolytic) get hand placed; two
  smaller ones in parallel on the same line do not.
- Footprints that accept every variant of a connector (pegs or not, clip, screw) ride out
  shortages.
- Power parts with an exposed pad, chosen by theta-JA; LDO heat is (Vin - Vout) x I.
- A TVS on every line that leaves the board and on every supply rail (an input fault reaches the
  rail through the chip's ESD cells faster than an LDO reacts): working voltage at or above the
  line's maximum, lowest clamping voltage, under 0.5 pF on USB 3 and HDMI. On analog inputs put a
  series resistor ahead of it, sized for continuous faults, and remember TVS capacitance varies
  with voltage and distorts AC signals.
- Shielded, low-profile inductors for switchers.

## Placement

- Plan the floor first: analog, digital and power regions, then route the most sensitive nets
  first. Components on one side when the board allows.
- Mixed signal: low-frequency analog, high-frequency analog and digital each in their own region,
  at least 20H apart (H the height to the plane) and further if room. Analog in a corner where no
  other block's return current passes, not the middle. Feed each block's supply so its current
  never crosses another block. Audio stays on the same side as digital, separated by distance;
  HF analog and digital can go on opposite sides of a multi-plane board.
- Pick ICs whose pinout splits digital and analog cleanly. A digital line into the analog region
  gets a 1 to 2 k series resistor or an RC-C filter right at the boundary.
- Analog front ends: the gain stage right at the connector so millivolt traces are short, the
  anti-alias filter right at the ADC input, never fast digital beside a millivolt net.
- Connector at the edge, then its TVS right beside it, then reverse polarity protection, then the
  filter, then the IC.
- Buck converter: Cin first, tight to VIN and GND, then the switch node and inductor, then Cout
  with its ground beside the IC ground. FET, diode and inductor (or an integrated-FET IC and its
  inductor) on the same side, no vias in the hot loop. Connect the inductor's winding start to the
  switch node so the outer turns shield it. Feedback from the output cap, away from the inductor.
- Decoupling placement beats connection style: the distance from cap to power pin mattered far
  more than via position or trace width in Feranec's PDN sims.
- Keep traces and parts off the board edge: return current crowds the plane edge and radiates. A
  trace 0.2 mm above the plane and 6 mm from the edge puts 1% of its current on the edge.
- Rotate parts and swap pins (`agentee pinswap`) to cut ratsnest crossings. Rotate neighbouring
  shunt caps and air-core inductors in a high-rejection filter 90 degrees so their loops do not
  couple.

## Copper

- Keep the switch node copper as small as the current allows; input and output copper can be
  large. A solid plane directly under the hot loop cancels its field; never cut it there.
- Size every current path with `current` on its class. Inner layers need about 2.5 times the
  width of outer and connect to via barrels more weakly. Widen or parallel layers before 2 oz.
  A 1 V, 10 A rail through one via can drop enough to break an FPGA's timing.
- Neck down only where a pad forces it, and widen again straight after (`agentee neck`).
- Decoupling caps: short wide connections (under 1 nH; a long thin trace is 5 to 10 nH), route
  into the cap then the pin, and add vias at the IC pin as well as at the cap or the extra ones do
  little. Via inductance scales with the dielectric it crosses: about 500 pH through 1.6 mm, 12 pH
  through 0.2 mm, quartered by four in parallel. A regulator's output cap joins its pins on the
  same layer.
- Clear the plane under the inverting input and feedback of a high-speed op-amp, and keep that
  node short; differential inputs symmetric and close.
- Spread tracks out wherever there is room; crosstalk shows up as random resets and interrupts.
  Spacing is the fix (2H microstrip, 1H stripline as a floor, much more for an ADC input or
  reset). Slowing the aggressor's edge does more than shortening the run. Never a grounded guard
  trace between signals unless it is stitched densely.
- Slow every edge you don't need: lower drive strength or slew control, or a series resistor or
  ferrite on each non-critical IO pin, with an RC/LC filter where it leaves the board. Give every
  IC pin that is not a fast interface provision for one.
- Space vias so plane copper flows between their antipads, especially under BGAs.
- Thermal vias in an exposed pad, as many as fit, 0.2 to 0.25 mm drill, straight to inner planes.
- BGA: dog-bone fanout pointing out by quadrant, a power via beside each ground via, every via
  placed before routing. Decouple with 0201s between paired vias on the back, then 0402, bulk
  nearby. At 0.5 mm pitch route through NC balls the datasheet says are unconnected.
- 2L: parts and routing on top, solid ground below. Jumps to the bottom short, and a longer cut
  under DC pads, never under a signal. RF traces well under lambda / 10.
- Teardrops on thin traces into big connector pads; they survive rework.

## EMC, enclosure and cables

- SI, PI and EMC are the same problem: overshoot and rail noise are emissions. Designing to 5 to
  10 times tighter limits than the logic needs (2% overshoot, 10 mV rail noise) buys EMC margin.
  About 2.5 uA of common-mode current on a cable fails Class B.
- Cables radiate most below 200 to 300 MHz; above that look at slots, internal cables and board
  edges. A cable resonates where its length is a half wavelength of a harmonic.
- Plane edges radiate. Put a ground strip on every layer at the board edge stitched by a via
  fence, or use edge plating (plated with the via barrels, no extra cost where the fab offers
  it). Pulling the power plane back 20H does not help.
- A cable shield bonds 360 degrees to the chassis at the entry, never by pigtail (zero ohms at DC
  was 18 ohms at 300 MHz) to signal ground or earth. Two-pin RJ45 shields leak.
- Single-ended IO that leaves the box needs a shielded cable. In a plastic enclosure make all IO
  truly differential (Ethernet runs unshielded for that reason).
- Bond board ground to a metal chassis at many points, under lambda / 10 apart at the highest
  frequency of concern, most importantly where cables enter. Single-point bonding is a resonant
  structure. Give each mounting hole pads for a 0 ohm, a capacitor (RF-only, for automotive) or a
  resistor (damping), and keep parts clear of screw heads.
- Heatsinks radiate and resonate; bond them to the plane at enough points to push the cavity
  resonance above the noise, or damp with resistive bonds or absorber.
- Shield cans: a perimeter guard trace with fence vias well under lambda / 10 apart (5 mm at
  3 GHz inside the board, where the wavelength halves). Mark shielded zones in the schematic so the
  layout keeps their parts inside. Every net that leaves a shield is filtered at the wall or runs
  as stripline between ground planes. Filters and shields only work together; a leaky shield can
  be 30 dB worse than none.
- Switcher conducted emissions: two input caps in parallel beat one of double the value, moving
  one closer to the FETs gains 10 dB at HF, a small LC input filter beats more bulk, a 10 uF MLCC
  at the input cuts both differential and common-mode noise, and a common-mode choke handles what
  is left on the cable.
- Two mains-powered devices joined by a cable (USB) loop their grounds; isolated supplies fix it.

## Test and DFM

- Tooling holes on every board that goes in a fixture.
- Programming pins on test pads. Several ground test pads, one beside each power and signal
  group, so one probe is not the return for everything; several pads per power input, about
  0.5 A each. Spare pads let a fixture add nets later.
- Ask the fixture maker before setting pad size and pitch: good probes reach 0.63 mm pads, and
  pitch need not be 2.54 mm. Lower `min_test_pad` to what they agree.
- Never tie a regulator's EN straight to VIN; a series resistor with a test pad lets the fixture
  sequence power and spares a logic-level EN from a 12 V input. Bring each device's reset out the
  same way.
- Lock test pad positions once a product is in production: moving one obsoletes the bed-of-nails
  fixture. Reuse one test pad layout across similar boards to share a fixture.
- Build in self-test (rails read back on ADC inputs, firmware checks) for what a fixture cannot
  reach, such as DDR.
- Parts 3 to 5 mm from a V-score, and no connector overhanging a break line. Design the assembly
  panel yourself with the assembler's input, so there is a record of what was built and the edge
  finish suits the enclosure.
- On a double-sided board keep each part number on one side so each side loads its own feeders.
- Conformal coating means hand-masking every connector, button and programming port; minimise
  them.
- Leave stencil and paste design to the assembler and the production panel to the fab.
- Send fab notes and the stackup with the Gerbers, and tick controlled impedance when it matters
  so it is measured. Answer a fab's engineering questions by fixing the design and sending new
  Gerbers, so the next order has none.

## Before fab

- Render and read every copper layer alone: slivers, plane voids, starved reliefs.
- `agentee library verify` for footprints against the datasheet, then look at pin 1 and rotation
  of each fine-pitch part in a render.
- Silk: pin 1, polarity, designators, board name, revision and date.

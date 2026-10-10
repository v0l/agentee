# Board design practice

Judgement that `agentee check` cannot make for you: which stackup, which part, where it goes and
what the rules should be. Distilled from Phil's Lab (#60, #75, #88, #93, #95, #114, #131, #137,
#156, #170, #171, #178) and Robert Feranec's FEDEVEL talks with Rick Hartley, Eric Bogatin,
Keith Armstrong and Hans Rosenberg. Check already enforces mask webs, tombstone risk, starved thermals,
fiducials, return vias, flex zones and edge clearance, so those are not repeated here. Where a
rule below is broken on purpose, say why in DESIGN.md.

## Stackup and rules

- Pick the stackup before placing anything, from `agentee stackups`: the fab's standard build
  and its impedance widths, not a custom one. Rules and impedance widths follow from it.
- Four layers by default: signal, ground, ground (or power), signal with thin prepreg. Two layers
  only for simple, cost-driven boards; at 10k boards 2L saves about 10 cents, which a few hours of
  layout or a failed EMC test outweighs.
- Every signal layer gets a solid reference plane next to it. Never two signal layers adjacent;
  if forced, route them exactly orthogonal so no run sits on top of another.
- Reference signals to ground. A signal over another rail's plane (3.3 V logic over a 5 V plane)
  has no clean return.
- Under about 10 A a digital board needs no power plane: route power as wide copper and make the
  layer another ground. Two planes that form a cavity should both be ground so stitching vias can
  short them; a power and ground pair resonates (about 2.5 GHz across a 25 to 35 mm region).
- One ground net. Do not split ground for mixed signal, for a switcher, or because an app note
  says so; separate by placement instead. Never put a ferrite bead in a ground path.
- A solid plane beats a pour. Pour copper on signal layers only where it is stitched densely to
  the plane below; an unstitched pour adds emissions.
- Keep the stackup symmetrical so the board does not warp.
- Stay well above the fab minimum. Global track and space 0.127 mm (5 mil) when fine-pitch parts
  need it, 0.2 to 0.3 mm when the board is 0603 and SOIC. Relax only around the part that needs
  it. Drill 0.3 mm (0.25 mm minimum), board edge 0.5 mm, mask expansion 0.05 to 0.1 mm.
- A cheap 2L build wants 0.15 mm track and space and 0.3 mm holes; anything finer is a cost adder.
- Every process option costs: ENIG over HASL, non-green mask, 2 oz copper, filled vias,
  castellations, gold fingers, HDI. Use one only when a part or interface needs it.
- Controlled impedance only on nets that need it, with the field solver (`solver = "field"`).
  Fast edges need it whatever the length: frequency content is about 0.5 / rise time, and with no
  rise time in the datasheet assume 1 / (10 x clock).

## Parts

- Treat app notes and evaluation board layouts as unverified; the layouts are often wrong. Follow
  the principles here when they disagree, and say so in DESIGN.md.
- Decoupling is about loop inductance, not value. Above a small threshold the value does not
  matter; the 10 nF, 100 nF, 1 uF ladder is folklore. Use one small-package value for HF and
  bulk caps for the rest.
- A ferrite bead does not fix rail noise from switching IO; decoupling does. Use beads only on
  low-current sensitive rails (PLL, ADC reference, analog), and skip the filter when the part
  tolerates 5 to 10% ripple.
- Connectors: budget a return pin for every signal pin where possible, and pick the connector
  size after adding grounds, not before. Fast signals sharing one return need a crosstalk
  estimate. Ribbon cables alternate signal and ground.
- A switcher controller with an external FET lets you control the hot loop; an integrated one
  fixes it for you.

- Consolidate the BOM: reuse one value where the exact value does not matter (a 12k 1% bias
  resistor also serves as a 10k pull-up). Prefer JLC basic parts when assembling there.
- 0603 passives by default, 0402 when space needs it, 0201 only under BGAs.
- Leaded packages (QFP, SOIC) over QFN, QFN over BGA, when the part comes in more than one.
  A BGA needs a fab and assembler that can do its pitch, 0201 and X-ray: confirm before layout.
- Class 2 MLCCs (X5R, X7R) lose capacitance with DC bias: 100 nF 10 V is about 60 nF at 3.3 V.
  Rate at least twice the applied voltage and read the bias curve before sizing a value.
- C0G for crystal load caps, filter and timing caps and the audio signal path.
- For high-frequency decoupling a smaller package beats a bigger value (0402 about 650 pH,
  1206 about 1250 pH).
- Power parts with an exposed pad. LDO dissipation is (Vin - Vout) x I; pick the package by its
  theta-JA and keep the input close to the output. A large drop wants a switcher.
- Mixed signal: pick ICs whose pinout splits digital and analog cleanly, and compare LDO noise
  and PSRR, not only current.
- A TVS on every line that leaves the board and on every supply rail: working voltage at or above
  the line's maximum, lowest clamping voltage, under 0.5 pF on USB 3 and HDMI.

## Placement

- Components on one side when the board allows; double-sided assembly costs more.
- Rotate parts and swap pins (`agentee pinswap`) to cut ratsnest crossings before routing.
- Connector at the edge, then its TVS right beside it, then reverse polarity protection, then the
  filter, then the IC.
- Mixed signal: digital and analog in their own regions, at least 20H apart and as far as the
  board allows. Put analog in a corner where no other block's return current passes, and feed
  each block's supply so its current never crosses the other block. Draw the boundary in silk and a via fence along it (`[[stitching]] fence`, pitch at most
  lambda / 10). A signal crossing it goes through an RC-C or ferrite pi filter at the crossing.
  Separate analog and digital regulators, a pi filter at the power input and another before the
  analog rail.
- Buck converter: Cin first, tight to VIN and GND, then the switch node and inductor, then Cout
  with its ground beside the IC ground. Whole power stage on one side, no vias in the hot loop.
- Feedback divider at the FB pin, fed from the output cap, routed away from the inductor and
  switch node.
- Route the most sensitive nets first: decoupling, then analog input filters, then clocks. Keep
  differential and op-amp input traces symmetric and close so noise lands on both equally.
- Keep clocks and fast nets away from other signals, the board edge and metal-cased parts.

## Copper

- Keep switch node copper small; output and input copper can be large.
- Size every current path with `current` on its class. Inner layers need about 2.5 times the
  width of outer. Widen or parallel layers before going to 2 oz.
- Neck down only where a pad forces it, and widen again straight after (`agentee neck`).
- Decoupling caps: short wide connections (under 1 nH; a long thin trace is 5 to 10 nH), at least
  one via per power and ground pin, never shared. Extra vias at the cap help only with matching
  vias at the IC pin. A regulator's output cap joins its pins on the same layer, then several
  vias to the plane.
- Clear the plane under the input and feedback nodes of a high-speed op-amp.
- Spread tracks out wherever there is room.
- Ground pours on signal layers stay at least 3H from impedance traces (H the dielectric height)
  unless the class is coplanar. Stitch pours to the plane regularly and along the board edge, at
  most lambda / 10 apart at the highest harmonic that matters (the 9th, or 0.5 / rise time).
- Space vias so plane copper flows between their antipads, especially under BGAs and on power
  via arrays.
- Crosstalk: spacing is the fix (2H microstrip, 1H stripline as a floor, more for an ADC input
  or reset line). Shortening a parallel run rarely helps; slowing the aggressor's edge does.
  Never a grounded guard trace between signals unless it is stitched densely.
- Thermal vias in an exposed pad, as many as fit, 0.2 to 0.25 mm drill, straight to inner planes.
  Vias beside the pad do little.
- BGA: dog-bone fanout pointing out by quadrant, a power via beside each ground via, every via
  placed before routing. Decouple with 0201s between paired vias on the back, then 0402, bulk
  nearby. At 0.5 mm pitch route through NC balls the datasheet says are unconnected.
- 2L: parts and routing on top, solid ground below. Jumps to the bottom short, and a longer cut
  under DC pads, never under a signal. RF traces well under lambda / 10 so their width does not
  matter (50 ohm microstrip on 1.6 mm 2L is about 2.8 mm wide).

## Enclosure and cables

- A cable shield bonds 360 degrees to the chassis at the entry, never by pigtail to signal ground
  or earth.
- Single-ended IO that leaves the box needs a shielded cable. In a plastic enclosure make all IO
  truly differential (Ethernet runs unshielded for that reason).
- Bond board ground to a metal chassis at many points, under lambda / 10 apart at the highest
  frequency of concern; resistive bonds damp the resonance a solid bond can excite.
- Shield cans: fence vias and solder points well under lambda / 10 apart (5 mm at 3 GHz). A
  leaky shield is worse than none.

## Test and DFM

- Programming and debug pins on test pads, and a ground test pad beside every power and UART
  test pad so the fixture's return is short.
- Keep parts 3 to 5 mm from a V-score. Define the panel yourself so there is a record of what
  was built.
- Send fab notes and the stackup with the Gerbers, never Gerbers alone.

## Before fab

- Render and read every copper layer alone: slivers, plane voids, starved reliefs.
- `agentee library verify` for footprints against the datasheet, then look at pin 1 and rotation
  of each fine-pitch part in a render.
- Silk: pin 1, polarity, designators, board name, revision and date.

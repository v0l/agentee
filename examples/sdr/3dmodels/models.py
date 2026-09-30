import cadquery as cq
from pathlib import Path

HERE = Path(__file__).parent
METAL = cq.Color(0.82, 0.82, 0.84)
PIN = cq.Color(0.9, 0.78, 0.45)
BLACK = cq.Color(0.12, 0.12, 0.13)
BODY = cq.Color(0.2, 0.2, 0.22)
CERAMIC = cq.Color(0.92, 0.9, 0.84)
SOLDER = cq.Color(0.75, 0.75, 0.77)


def save(assy, name):
    assy.save(str(HERE / f"{name}.step"), "STEP")


def box(w, l, h, x=0.0, y=0.0, z=0.0):
    return cq.Workplane("XY").box(w, l, h).translate((x, y, z + h / 2))


def from_y(solid, y0):
    return solid.translate((0, y0 - solid.val().BoundingBox().ymin, 0))


def usb_c_molex_105450():
    width, height = 8.94, 3.26
    front, back = -4.505, 3.965
    wall = 0.25
    length = back - front
    outer = from_y(cq.Workplane("XZ").center(0, height / 2).slot2D(width, height).extrude(length), front)
    opening = from_y(
        cq.Workplane("XZ").center(0, height / 2).slot2D(width - 2 * wall, height - 2 * wall).extrude(length - 1.2),
        front - 0.01,
    )
    shell = outer.cut(opening)
    tongue = box(6.69, length - 1.6, 0.7, 0, (front + back) / 2 + 0.4, height / 2 - 0.35)
    insert = from_y(
        cq.Workplane("XZ").center(0, height / 2).slot2D(width - 2 * wall - 0.02, height - 2 * wall - 0.02).extrude(1.0),
        back - 1.25,
    )
    legs = cq.Workplane("XY")
    for x in (-4.32, 4.32):
        for y, l in ((-2.805, 1.4), (2.555, 1.9)):
            legs = legs.union(box(0.3, l - 0.2, 1.6, x, -y, -1.0))
    pins = cq.Workplane("XY")
    for k in range(12):
        x = -3.0 + 0.5 * k if k < 6 else 0.5 + 0.5 * (k - 6)
        pins = pins.union(box(0.25, 1.0, 0.15, x, 3.415))
    for k in range(12):
        pins = pins.union(box(0.25, 0.6, 0.15, -2.75 + 0.5 * k, 1.915))
    assy = cq.Assembly(name="USB_C_Receptacle_Molex_105450-0101")
    assy.add(shell, name="shell", color=METAL)
    assy.add(tongue, name="tongue", color=BLACK)
    assy.add(insert, name="insert", color=BLACK)
    assy.add(legs, name="legs", color=METAL)
    assy.add(pins, name="pins", color=PIN)
    save(assy, "USB_C_Receptacle_Molex_105450-0101")


def bga(name, body, rows, pitch, ball, seat, height):
    lid = cq.Workplane("XY").box(body, body, height - seat).edges("|Z").chamfer(0.1)
    lid = lid.translate((0, 0, seat + (height - seat) / 2))
    mark = cq.Workplane("XY").circle(0.35).extrude(0.02).translate((-body / 2 + 1.0, body / 2 - 1.0, height))
    balls = cq.Workplane("XY")
    first = -(rows - 1) * pitch / 2
    for i in range(rows):
        for j in range(rows):
            balls = balls.union(
                cq.Workplane("XY").circle(ball / 2).extrude(seat).translate((first + i * pitch, first + j * pitch, 0))
            )
    assy = cq.Assembly(name=name)
    assy.add(lid, name="body", color=BODY)
    assy.add(mark, name="pin1", color=CERAMIC)
    assy.add(balls, name="balls", color=SOLDER)
    save(assy, name)


def balun_db1627():
    base = box(3.81, 4.06, 0.6)
    cap = cq.Workplane("XY").box(3.3, 3.6, 2.5).edges("|Z").chamfer(0.3).translate((0, 0, 0.6 + 1.25))
    pads = cq.Workplane("XY")
    for x in (-1.59, 1.59):
        for y in (-1.27, 0.0, 1.27):
            pads = pads.union(box(0.9, 0.6, 0.05, x, y))
    assy = cq.Assembly(name="MiniCircuits_DB1627")
    assy.add(base, name="base", color=CERAMIC)
    assy.add(cap, name="cap", color=BLACK)
    assy.add(pads, name="pads", color=PIN)
    save(assy, "MiniCircuits_DB1627")


def dfn(name, w, l, height, pins_per_side, pitch, ep):
    body = box(w, l, height - 0.02, z=0.02)
    mark = cq.Workplane("XY").circle(0.15).extrude(0.02).translate((-w / 2 + 0.4, l / 2 - 0.4, height))
    leads = cq.Workplane("XY")
    first = (pins_per_side - 1) * pitch / 2
    for side in (-1, 1):
        for k in range(pins_per_side):
            leads = leads.union(box(0.4, 0.25, 0.2, side * (w / 2 - 0.2), first - k * pitch))
    leads = leads.union(box(ep[0], ep[1], 0.02))
    assy = cq.Assembly(name=name)
    assy.add(body, name="body", color=BODY)
    assy.add(mark, name="pin1", color=CERAMIC)
    assy.add(leads, name="leads", color=PIN)
    save(assy, name)


def oscillator_3225(name):
    base = box(3.2, 2.5, 0.35)
    lid = cq.Workplane("XY").box(3.0, 2.3, 0.65).edges("|Z").chamfer(0.15).translate((0, 0, 0.35 + 0.325))
    pads = cq.Workplane("XY")
    for x in (-1.05, 1.05):
        for y in (-0.825, 0.825):
            pads = pads.union(box(0.9, 0.8, 0.03, x, y))
    assy = cq.Assembly(name=name)
    assy.add(base, name="base", color=CERAMIC)
    assy.add(lid, name="lid", color=METAL)
    assy.add(pads, name="pads", color=PIN)
    save(assy, name)


usb_c_molex_105450()
bga("Infineon_PG-TFBGA-169_10x10mm_P0.75mm", 10.0, 13, 0.75, 0.35, 0.25, 1.2)
balun_db1627()
dfn("Texas_S-PDSO-N10_EP1.2x2mm", 2.5, 2.5, 0.8, 5, 0.5, (1.2, 2.0))
oscillator_3225("Oscillator_SMD_Abracon_ASE-4Pin_3.2x2.5mm")

import json
import os
import sys

import numpy as np
from CSXCAD import ContinuousStructure
from openEMS import openEMS
from openEMS.physical_constants import EPS0

cases = json.load(open(sys.argv[1]))
out = os.path.abspath(sys.argv[2])
only = sys.argv[3] if len(sys.argv) > 3 else None

for c in cases:
    if only and c["name"] != only:
        continue
    name = c["name"]
    L, w, h = c["len"], c["w"], c["h"]
    y0, y1 = c["y"]
    f0, f1, npts = c["f"]
    fc = 0.5 * (f0 + f1)
    cell = c["cell"]
    fdtd = openEMS(EndCriteria=1e-5, NrTS=400000)
    fdtd.SetGaussExcite(fc, 0.5 * (f1 - f0) * 1.1)
    fdtd.SetBoundaryCond(["PML_8"] * 6)
    csx = ContinuousStructure()
    fdtd.SetCSX(csx)
    mesh = csx.GetGrid()
    mesh.SetDeltaUnit(1e-3)

    kappa = 2 * np.pi * fc * EPS0 * c["er"] * c["tan"]
    sub = csx.AddMaterial("sub", epsilon=c["er"], kappa=kappa)
    sub.AddBox([0, y0, 0], [L, y1, h])
    if c["copper"] > 0:
        metal = csx.AddConductingSheet("cu", conductivity=5.8e7, thickness=c["copper"] * 1e-3)
    else:
        metal = csx.AddMetal("pec")
    metal.AddBox([0.5, -w / 2, h], [L - 0.5, w / 2, h], priority=10)
    metal.AddBox([0, y0, 0], [L, y1, 0], priority=10)
    xs = [0, 0.5, L - 0.5, L, 0.45, 0.55, L - 0.55, L - 0.45]
    ys = [y0, y1, -w / 2, w / 2]
    if c.get("stub"):
        x = L / 2
        metal.AddBox([x - w / 2, 0, h], [x + w / 2, c["stub"], h], priority=10)
        t = cell / 3
        xs += [x - w / 2 - t, x - w / 2 + 2 * t, x + w / 2 + t, x + w / 2 - 2 * t]
        ys += [c["stub"] + t, c["stub"] - 2 * t]
    ports = []
    for i, x in enumerate([0.5, L - 0.5]):
        ports.append(
            fdtd.AddLumpedPort(i + 1, 50, [x - 0.05, -w / 2, 0], [x + 0.05, w / 2, h], "z", excite=1 if i == 0 else 0, priority=5)
        )

    air = 8.0
    third = cell / 3
    xs = sorted(set(xs + [-air, L + air]))
    ys = sorted(set(ys + [y0 - air, y1 + air, -w / 2 - third, -w / 2 + 2 * third, w / 2 + third, w / 2 - 2 * third]))
    zs = list(np.linspace(0, h, 7)) + [-air, h + air]
    mesh.AddLine("x", xs)
    mesh.AddLine("y", ys)
    mesh.AddLine("z", zs)
    mesh.SmoothMeshLines("x", cell * 2, 1.3)
    mesh.SmoothMeshLines("y", cell * 2, 1.3)
    mesh.SmoothMeshLines("z", max(h / 6, cell), 1.3)
    mesh.SmoothMeshLines("all", 3e8 / f1 / 1e-3 / 20, 1.4)

    path = os.path.join(out, "openems_" + name)
    os.makedirs(path, exist_ok=True)
    fdtd.Run(path, cleanup=True, verbose=0)
    f = np.linspace(f0, f1, int(npts))
    for p in ports:
        p.CalcPort(path, f)
    s11 = ports[0].uf_ref / ports[0].uf_inc
    s21 = ports[1].uf_ref / ports[0].uf_inc
    with open(os.path.join(out, name + ".openems.s2p"), "w") as fh:
        fh.write(f"! openEMS {name}\n# Hz S RI R 50\n")
        for k in range(len(f)):
            fh.write(f"{f[k]:.6e} {s11[k].real:.6e} {s11[k].imag:.6e} {s21[k].real:.6e} {s21[k].imag:.6e} {s21[k].real:.6e} {s21[k].imag:.6e} {s11[k].real:.6e} {s11[k].imag:.6e}\n")
    print(name, "done", flush=True)

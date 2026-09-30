import ctypes
import ctypes.util
import json
import os
import re
import sys
import time

import numpy as np
from CSXCAD import ContinuousStructure
from CSXCAD.CSProperties import CSProperties
from openEMS import openEMS


def djordjevic_sarkar(er, tan, f_ref, f):
    w1, w2 = 2 * np.pi * 1e3, 2 * np.pi * 1e12
    g = lambda f: np.log10((w2 + 2j * np.pi * f) / (w1 + 2j * np.pi * f)) / np.log10(w2 / w1)
    delta = er * tan / -g(f_ref).imag
    return er + delta * (g(f) - g(f_ref).real)


def debye_band(f_lo, f_hi):
    x0, x1 = 2 * np.pi * f_lo / 30, 2 * np.pi * f_hi * 30
    span = np.log(x1 / x0)
    n = max(int(np.ceil(span / 0.7)), 1)
    h = span / n
    x = x0 * np.exp(h * np.arange(n + 1))
    a = np.full(n + 1, h / span)
    a[[0, -1]] *= 0.5
    return x, a


def debye_eps(inf, delta, x, a, f):
    return inf + np.sum(delta * a / (1 + 2j * np.pi * np.asarray(f)[..., None] / x), axis=-1)


def debye_fit(er, tan, f_lo, f_hi):
    x, a = debye_band(f_lo, f_hi)
    f_mid = np.sqrt(f_lo * f_hi)
    e = djordjevic_sarkar(er, tan, 1e9, f_mid)
    s = debye_eps(0.0, 1.0, x, a, f_mid)
    delta = -e.imag / -s.imag
    inf = e.real - delta * s.real
    f = np.geomspace(f_lo, f_hi, 50)
    fit, want = debye_eps(inf, delta, x, a, f), djordjevic_sarkar(er, tan, 1e9, f)
    err_er = np.max(np.abs(fit.real - want.real))
    err_tan = np.max(np.abs((fit.imag / fit.real) / (want.imag / want.real) - 1))
    print(f"Debye fit, {len(x)} poles: er off by {err_er:.4f}, tan off by {100 * err_tan:.2f}% over {f_lo / 1e9:.2f} to {f_hi / 1e9:.2f} GHz", flush=True)
    assert err_er < 0.01 and err_tan < 0.03
    return inf, delta * a, 1 / x

# XCHECK_ENGINE picks the openEMS engine (e.g. gpu, multithreaded; default: openEMS's own),
# XCHECK_EXACT=1 evaluates the end criteria on a fixed timestep schedule (GPU branch only), and
# XCHECK_TAG names the output (<name>.<tag>.s2p, default openems).
engine = os.environ.get("XCHECK_ENGINE")
exact = os.environ.get("XCHECK_EXACT") == "1"
tag = os.environ.get("XCHECK_TAG", "openems")
sheet_dz = float(os.environ.get("XCHECK_SHEET_DZ", "0"))
fine = float(os.environ.get("XCHECK_FINE", "0"))


def fill(lines, lo, hi, d):
    lines = sorted(set(lines))
    out = list(lines)
    for a, b in zip(lines, lines[1:]):
        if b <= lo or a >= hi:
            continue
        n = int(np.ceil((b - a) / d - 1e-9))
        out += [a + (b - a) * k / n for k in range(1, n)]
    return sorted(out)


def run_logged(fdtd, path, log):
    kw = {"verbose": 0}
    if engine:
        kw["engine"] = engine
    if exact:
        kw["exact_endcriteria"] = True
    sys.stdout.flush()
    saved = os.dup(1)
    t0 = time.time()
    with open(log, "w") as fh:
        os.dup2(fh.fileno(), 1)
        try:
            fdtd.Run(path, cleanup=True, **kw)
        finally:
            ctypes.CDLL(ctypes.util.find_library("c")).fflush(None)
            os.dup2(saved, 1)
            os.close(saved)
    wall = time.time() - t0
    text = open(log).read()
    m = re.search(r"Time for (\d+) iterations with ([0-9.e+]+) cells : ([0-9.e+-]+) sec", text)
    created = re.search(r"Create FDTD engine \((.*)\)", text)
    steps, cells, ts = int(m.group(1)), float(m.group(2)), float(m.group(3))
    print(
        f"{os.path.basename(path)}: engine {created.group(1) if created else '?'}, {cells:.0f} cells, {steps} steps, "
        f"run {wall:.2f}s, timestepping {ts:.2f}s ({cells * steps / ts / 1e6:.0f} MCells/s), setup+post {wall - ts:.2f}s",
        flush=True,
    )


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

    if c["tan"] > 0:
        f_lo = max(2 * fc - f1, f1 * 1e-3)
        inf, eps_delta, eps_relax = debye_fit(c["er"], c["tan"], f_lo, f1)
        sub = CSProperties.fromTypeName("DebyeMaterial", csx.GetParameterSet(), order=len(eps_delta), epsilon=float(inf))
        sub.SetName("sub")
        csx.AddProperty(sub)
        for k in range(len(eps_delta)):
            sub.SetDispersiveMaterialProperty(k, eps_delta=float(eps_delta[k]), eps_relax=float(eps_relax[k]))
    else:
        sub = csx.AddMaterial("sub", epsilon=c["er"])
    via = c.get("via")
    top = 2 * h if via else h
    sub.AddBox([0, y0, 0], [L, y1, top])
    if c["copper"] > 0:
        metal = csx.AddConductingSheet("cu", conductivity=5.8e7, thickness=c["copper"] * 1e-3)
    else:
        metal = csx.AddMetal("pec")
    xs = [0, 0.5, L - 0.5, L, 0.45, 0.55, L - 0.55, L - 0.45]
    ys = [y0, y1, -w / 2, w / 2]
    port_z = [(0, h), (0, h)]
    if via:
        x = L / 2
        r_drill, r_pad, r_anti = via["drill"] / 2, via["pad"] / 2, via["antipad"] / 2
        ring = lambda r: [[x + r * np.cos(a) for a in np.linspace(0, 2 * np.pi, 64, endpoint=False)],
                          [r * np.sin(a) for a in np.linspace(0, 2 * np.pi, 64, endpoint=False)]]
        metal.AddBox([0.5, -w / 2, top], [x, w / 2, top], priority=10)
        metal.AddBox([x, -w / 2, 0], [L - 0.5, w / 2, 0], priority=10)
        metal.AddBox([0, y0, h], [L, y1, h], priority=10)
        sub.AddCylinder([x, 0, h - 0.01], [x, 0, h + 0.01], r_anti, priority=20)
        metal.AddCylinder([x, 0, 0], [x, 0, top], r_drill, priority=30)
        for z in [0, top]:
            metal.AddPolygon(ring(r_pad), "z", z, priority=30)
        port_z = [(h, top), (h, 0)]
        xs += [x - r_anti, x + r_anti, x - r_drill, x + r_drill, x]
        ys += [-r_anti, r_anti, -r_drill, r_drill, 0]
    else:
        metal.AddBox([0.5, -w / 2, h], [L - 0.5, w / 2, h], priority=10)
        metal.AddBox([0, y0, 0], [L, y1, 0], priority=10)
    if c.get("stub"):
        x = L / 2
        metal.AddBox([x - w / 2, 0, h], [x + w / 2, c["stub"], h], priority=10)
        t = cell / 3
        xs += [x - w / 2 - t, x - w / 2 + 2 * t, x + w / 2 + t, x + w / 2 - 2 * t]
        ys += [c["stub"] + t, c["stub"] - 2 * t]
    ports = []
    for i, x in enumerate([0.5, L - 0.5]):
        ports.append(
            fdtd.AddLumpedPort(i + 1, 50, [x - 0.05, -w / 2, port_z[i][0]], [x + 0.05, w / 2, port_z[i][1]], "z", excite=1 if i == 0 else 0, priority=5)
        )

    air = 8.0
    third = (fine or cell) / 3
    xs = sorted(set(xs + [-air, L + air]))
    ys = sorted(set(ys + [y0 - air, y1 + air, -w / 2 - third, -w / 2 + 2 * third, w / 2 + third, w / 2 - 2 * third]))
    if fine:
        span = w / 2 + 0.2
        ys = fill(ys + [-span, span], -span, span, fine)
        if via:
            xs = fill(xs + [L / 2 - span, L / 2 + span], L / 2 - span, L / 2 + span, fine)
    zs = list(np.linspace(0, top, 7 if top == h else 13)) + [-air, top + air]
    if sheet_dz:
        zs += [z + s * sheet_dz for z in sorted({0, h, top}) for s in (-1, 1)]
    mesh.AddLine("x", xs)
    mesh.AddLine("y", ys)
    mesh.AddLine("z", zs)
    mesh.SmoothMeshLines("x", cell * 2, 1.3)
    mesh.SmoothMeshLines("y", cell * 2, 1.3)
    mesh.SmoothMeshLines("z", max(h / 6, cell), 1.3)
    mesh.SmoothMeshLines("all", 3e8 / f1 / 1e-3 / 20, 1.4)

    path = os.path.join(out, tag + "_" + name)
    os.makedirs(path, exist_ok=True)
    run_logged(fdtd, path, path + ".log")
    f = np.linspace(f0, f1, int(npts))
    for p in ports:
        p.CalcPort(path, f)
    s11 = ports[0].uf_ref / ports[0].uf_inc
    s21 = ports[1].uf_ref / ports[0].uf_inc
    with open(os.path.join(out, f"{name}.{tag}.s2p"), "w") as fh:
        fh.write(f"! openEMS {name} {tag}\n# Hz S RI R 50\n")
        for k in range(len(f)):
            fh.write(f"{f[k]:.6e} {s11[k].real:.6e} {s11[k].imag:.6e} {s21[k].real:.6e} {s21[k].imag:.6e} {s21[k].real:.6e} {s21[k].imag:.6e} {s11[k].real:.6e} {s11[k].imag:.6e}\n")
    print(name, "done", flush=True)

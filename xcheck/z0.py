import json
import os
import sys

import numpy as np
import skrf

d, short, long_ = sys.argv[1], sys.argv[2], sys.argv[3]
tags = sys.argv[4].split(",") if len(sys.argv) > 4 else ["agentee", "openems"]
freqs = [float(f) * 1e9 for f in (sys.argv[5].split(",") if len(sys.argv) > 5 else ["1.5", "3.5", "5"])]
cases = json.load(open(os.environ.get("XCHECK_CASES", os.path.join(os.path.dirname(__file__), "cases.json"))))
length = {c["name"]: c["len"] for c in cases}
dl = (length[long_] - length[short]) * 1e-3


def abcd(name, tag):
    n = skrf.Network(f"{d}/{name}.{tag}.s2p")
    s11, s21 = n.s[:, 0, 0], n.s[:, 1, 0]
    n.s = np.stack([np.stack([s11, s21], -1), np.stack([s21, s11], -1)], -2)
    return n.f, n.a


for tag in tags:
    f, a1 = abcd(short, tag)
    f2, a2 = abcd(long_, tag)
    assert np.allclose(f, f2)
    out = []
    for fx in freqs:
        k = np.argmin(np.abs(f - fx))
        w, v = np.linalg.eig(a2[k] @ np.linalg.inv(a1[k]))
        fwd, bwd = (0, 1) if (v[0, 0] / v[1, 0]).real > 0 else (1, 0)
        z_fwd = v[0, fwd] / v[1, fwd]
        z_bwd = -v[0, bwd] / v[1, bwd]
        z0 = np.sqrt(z_fwd * z_bwd)
        z0 = z0 if z0.real > 0 else -z0
        beta = np.angle(w[fwd]) / dl
        beta = beta if beta > 0 else beta + 2 * np.pi / dl
        eeff = (beta * 299792458.0 / (2 * np.pi * f[k])) ** 2
        out.append(f"{f[k] / 1e9:.2f} GHz Z0 {z0.real:.2f} ohm (fwd {abs(z_fwd):.2f}, bwd {abs(z_bwd):.2f}), eeff {eeff:.4f}")
    print(f"{tag} {short}/{long_}: " + "; ".join(out))

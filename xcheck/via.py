import sys
import warnings

import numpy as np
import skrf
from skrf.calibration.deembedding import IEEEP370_SE_NZC_2xThru

warnings.filterwarnings("ignore")

d, via, short, long_ = sys.argv[1:5]
tags = sys.argv[5].split(",") if len(sys.argv) > 5 else ["agentee", "openems"]
freqs = [float(f) * 1e9 for f in (sys.argv[6].split(",") if len(sys.argv) > 6 else ["1", "3", "5"])]
db = lambda x: 20 * np.log10(np.abs(x))


def load(name, tag):
    n = skrf.Network(f"{d}/{name}.{tag}.s2p")
    s11, s21 = n.s[:, 0, 0], n.s[:, 1, 0]
    n.s = np.stack([np.stack([s11, s21], -1), np.stack([s21, s11], -1)], -2)
    return n


def line_z0(a, b):
    out = []
    for m1, m2 in zip(a.a, b.a):
        w, v = np.linalg.eig(m2 @ np.linalg.inv(m1))
        fwd, bwd = (0, 1) if (v[0, 0] / v[1, 0]).real > 0 else (1, 0)
        z = np.sqrt((v[0, fwd] / v[1, fwd]) * (-v[0, bwd] / v[1, bwd]))
        out.append(z.real if z.real > 0 else -z.real)
    return np.array(out)


for tag in tags:
    v, thru, longer = load(via, tag), load(short, tag), load(long_, tag)
    f = v.f
    ks = [int(np.argmin(np.abs(f - x))) for x in freqs]
    z0 = line_z0(thru, longer)
    k15 = int(np.argmin(np.abs(f - 1.5e9)))
    renorm = v.copy()
    renorm.renormalize(np.full((len(f), 2), z0[k15]))
    alone = IEEEP370_SE_NZC_2xThru(dummy_2xthru=thru, name=tag, verbose=False).deembed(v)
    alone.renormalize(np.stack([z0, z0], -1))
    row = lambda x, p=1: " / ".join(f"{x[k]:.{p}f}" for k in ks)
    ghz = " / ".join(f"{f[k] / 1e9:g}" for k in ks)
    print(f"{tag} {via} at {ghz} GHz, line Z0 {z0[k15]:.2f} ohm at 1.5 GHz")
    print(f"  S11 {row(db(v.s[:, 0, 0]))} dB, S21 {row(db(v.s[:, 1, 0]), 3)} dB, {row(np.degrees(np.angle(v.s[:, 1, 0])))} deg")
    print(f"  S11 renormalised to the line Z0: {row(db(renorm.s[:, 0, 0]))} dB")
    print(f"  via alone (P370 NZC with {short} as the 2x-thru, line Z0 at each frequency): S11 {row(db(alone.s[:, 0, 0]))} dB, "
          f"|S11| {row(np.abs(alone.s[:, 0, 0]) * 1e3)} m, {row(np.degrees(np.angle(alone.s[:, 0, 0])), 0)} deg")

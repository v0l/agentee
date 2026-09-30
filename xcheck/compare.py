import os
import sys

import numpy as np
import skrf

names = sys.argv[2:]
d = sys.argv[1]
# XCHECK_A / XCHECK_B: the two result tags to compare (<name>.<tag>.s2p), default agentee / openems
tag_a = os.environ.get("XCHECK_A", "agentee")
tag_b = os.environ.get("XCHECK_B", "openems")
for name in names:
    a = skrf.Network(f"{d}/{name}.{tag_a}.s2p")
    o = skrf.Network(f"{d}/{name}.{tag_b}.s2p")
    f = o.f
    a = a.interpolate(skrf.Frequency.from_f(f, unit="Hz"))
    s11a, s21a = a.s[:, 0, 0], a.s[:, 1, 0]
    s11o, s21o = o.s[:, 0, 0], o.s[:, 1, 0]
    db = lambda x: 20 * np.log10(np.abs(x))
    ph = lambda x: np.unwrap(np.angle(x))
    d21 = db(s21a) - db(s21o)
    dph = np.degrees(ph(s21a) - ph(s21o))
    loss_a = -10 * np.log10(np.abs(s11a) ** 2 + np.abs(s21a) ** 2)
    loss_o = -10 * np.log10(np.abs(s11o) ** 2 + np.abs(s21o) ** 2)
    print(f"== {name} ({tag_a} / {tag_b})")
    print(f"  |S21| diff dB: max {np.max(np.abs(d21)):.3f}, mean {np.mean(np.abs(d21)):.3f}")
    print(f"  S21 phase diff deg: max {np.max(np.abs(dph)):.2f} (at {f[np.argmax(np.abs(dph))]/1e9:.2f} GHz)")
    print(f"  |S11 - S11| max {np.max(np.abs(s11a - s11o)):.3f}; S11 dB agentee max {np.max(db(s11a)):.1f}, openEMS max {np.max(db(s11o)):.1f}")
    print(f"  dissipated+radiated dB (1-|S11|^2-|S21|^2): agentee {np.interp([1e9, 3e9, 5e9], f, loss_a).round(4)}, openEMS {np.interp([1e9, 3e9, 5e9], f, loss_o).round(4)} at 1/3/5 GHz")
    k = np.argmin(np.abs(s21o))
    ka = np.argmin(np.abs(s21a))
    if db(s21o)[k] < -10:
        print(f"  notch: agentee {f[ka]/1e9:.3f} GHz {db(s21a)[ka]:.1f} dB, openEMS {f[k]/1e9:.3f} GHz {db(s21o)[k]:.1f} dB")
    for fx in [1e9, 3e9, 5e9]:
        i = np.argmin(np.abs(f - fx))
        print(f"  {fx/1e9:.0f} GHz: S21 {db(s21a)[i]:.3f}/{db(s21o)[i]:.3f} dB, {np.degrees(np.angle(s21a[i])):.1f}/{np.degrees(np.angle(s21o[i])):.1f} deg, S11 {db(s11a)[i]:.1f}/{db(s11o)[i]:.1f} dB")

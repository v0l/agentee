# The grid of openEMS's python/Tests/FreeSpace_Benchmark.py (n^3 cells of 1 mm, soft z dipole in
# the centre, one probe, no end criterion) for one boundary and engine, with the time split into
# setup and timestepping. Compare with `xcheck throughput`.
#
#   python xcheck/openems_freespace.py <engine> <n> <steps> <PML_8|PEC> <outdir> [noprobe]
#
# noprobe drops the field probe: openEMS then runs the timesteps in few long batches, which
# separates the kernel throughput from the per-batch cost of sampling the probe. Without a probe
# the GPU engine does not wait for the device before openEMS stops its clock, so the reported
# timestepping misses the work still queued (up to a few hundred timesteps): use long runs and the
# difference between two step counts, or keep the probe.
import ctypes
import ctypes.util
import os
import re
import sys
import time

import numpy as np
from CSXCAD import ContinuousStructure
from openEMS import openEMS

engine, n, steps, bc, out = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], sys.argv[5]
probe = "noprobe" not in sys.argv[6:]
path = os.path.join(os.path.abspath(out), f"freespace_{engine}_{n}_{steps}_{bc}{'' if probe else '_noprobe'}")
os.makedirs(path, exist_ok=True)

t0 = time.time()
fdtd = openEMS(NrTS=steps, EndCriteria=0)
fdtd.SetGaussExcite(5e9, 4e9)
fdtd.SetBoundaryCond([bc] * 6)
csx = ContinuousStructure()
fdtd.SetCSX(csx)
mesh = csx.GetGrid()
mesh.SetDeltaUnit(1e-3)
for ax in "xyz":
    mesh.AddLine(ax, np.arange(n + 1) * 1.0)
c = n / 2
exc = csx.AddExcitation("dipole", exc_type=0, exc_val=[0, 0, 1])
exc.AddBox([c, c, c - 1], [c, c, c + 1])
if probe:
    csx.AddProbe("et", p_type=2).AddPoint([c + 10, c, c])

log = path + ".log"
sys.stdout.flush()
saved = os.dup(1)
with open(log, "w") as fh:
    os.dup2(fh.fileno(), 1)
    try:
        fdtd.Run(path, cleanup=True, engine=engine)
    finally:
        ctypes.CDLL(ctypes.util.find_library("c")).fflush(None)
        os.dup2(saved, 1)
        os.close(saved)
wall = time.time() - t0
text = open(log).read()
m = re.search(r"Time for (\d+) iterations with ([0-9.e+]+) cells : ([0-9.e+-]+) sec", text)
speed = re.search(r"Speed: *([0-9.]+) MCells/s", text)
created = re.search(r"Create FDTD engine \((.*)\)", text)
its, cells, ts = int(m.group(1)), float(m.group(2)), float(m.group(3))
print(
    f"openems engine={engine} n={n} bc={bc}{'' if probe else ' noprobe'} steps={its}: wall {wall:.2f}s, timestepping {ts:.2f}s, "
    f"setup+post {wall - ts:.2f}s, {float(speed.group(1)):.0f} MCells/s ({created.group(1) if created else '?'})",
    flush=True,
)

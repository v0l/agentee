# HackRF Pro

The HackRF Pro (code name Praline) main board by Great Scott Gadgets, imported from
[greatscottgadgets/hackrf-pro](https://github.com/greatscottgadgets/hackrf-pro) at commit
`359b98f2451395408bc25dae64efa9fc518e93d0` with:

```sh
agentee import board hackrf-pro/praline.kicad_pcb --dir examples/hackrf-pro
```

The three STEP models under `3dmodels/` come from the upstream `praline.3dshapes`; see
`3dmodels/README.txt` for where they were sourced. Footprints point at them by `3dmodels/` paths.

The design is licensed under the CERN Open Hardware Licence Version 2, Permissive; see `LICENSE`.
It is here as a large real board to check, view and benchmark against. `check` does not pass on it
yet.

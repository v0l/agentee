# TODO

Shared list of open work. See `AGENTS.md` for how to log, take and finish an item.

## Library

- Component Search Engine (SamacSys), SnapMagic (API key on request from snapeda.com/get-api)
  and Ultra Librarian need an account for an automatic model lookup; their search endpoints
  return 403 without one. Their KiCad ZIPs already work as `library add --model part.zip`.

## Check (taken: master)

- `zone-islands` on `examples/sdr` is not deterministic: four runs of `agentee check examples/sdr
  --info` gave three different sets of removed islands (1V0 on In3.Cu comes and goes).

version: 1.5.1784485843
---
### Fixed
- Preserved guest command output that arrives after the control channel reports
  completion, replacing the 100 ms reader race with a bounded five-second
  transport-loss window and deterministic delayed-output regression coverage.
- Bounded Docker storage across the canonical Ironbank gate: capacity is
  checked before and after builder materialization, inactive incremental state
  is reclaimed under pressure, prior target volumes are flushed before a new
  canonical run, and successful runs flush compiler artifacts while retaining
  a bounded hot BuildKit cache.

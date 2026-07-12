version: 1.5.1783867436
---
### Fixed
- Kept one complete canonical `just test` release gate on Linux, then fanned
  the exact macOS and Linux package build/install jobs out only after it passed.
- Explicitly documented the temporary absence of macOS full-gate coverage:
  GitHub-hosted macOS lacks the nested virtualization required by Capsem and
  Colima, and the repository has no physical macOS runner. The signed,
  notarized exact `.pkg` install remains release-blocking, and the parallel
  macOS full gate is restored when physical runner capacity exists.

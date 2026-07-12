version: 1.5.1783820797
---
### Fixed
- Hardened tag-triggered release CI to run Clippy across the full workspace and
  all targets with every warning treated as a release-blocking error.
- Made macOS and Linux package-script failures write an actionable tester
  report with the failed install phase, detailed log path, and exact command to
  copy into a bug report; macOS also opens the report visibly.

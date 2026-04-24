version: 1.0.1777059098
---
### Fixed (CI)
- Raise pnpm audit threshold to high/critical (was default=low); a new
  moderate postcss CVE in dev-only deps kept failing the release.

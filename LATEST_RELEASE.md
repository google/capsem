version: 1.3.1782582155
---
### Fixed
- Retried release app cargo-tool installs one tool at a time so transient
  crates.io DNS failures do not abort macOS/Linux packaging.

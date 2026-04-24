version: 1.0.1777065213
---
### Fixed (CI)
- Codesign companion binaries with --options runtime + --timestamp;
  notary rejected the .pkg because the 8 companion binaries lacked
  hardened runtime.

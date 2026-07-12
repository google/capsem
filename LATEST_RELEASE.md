version: 1.5.1783827911
---
### Fixed
- Kept the full-workspace release Clippy gate clean on Rust 1.97 by using a
  byte string in the large-body MITM integration fixture and `Option::filter`
  for absent audit TTY values.

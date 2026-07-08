version: 1.5.1783554373
---
### Fixed
- Made binary-lane channel assembly materialize preserved profile config
  artifacts from the asset release source tag and verify their hashes before
  deploy, keeping stable/nightly package updates from corrupting immutable
  profile release paths.

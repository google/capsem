version: 1.5.1783857731
---
### Fixed
- Kept the integration release gate hermetic in long checkout paths by using a
  short process-scoped runtime root for Unix sockets, sessions, and the logger
  index while preserving isolated config and credential state.
- Stopped duplicate full CI runs when a release commit lands on `main`; pull
  requests remain merge-gated and one explicitly dispatched, globally
  serialized workflow now releases a single requested stable or nightly
  channel.
- Installed the locked Python environment before generating release-site CI
  fixtures so declared dependencies such as `blake3` are available.
- Made release publication wait for the exact notarized macOS package and both
  release Linux packages to install successfully instead of testing a separate
  pre-release debug package.

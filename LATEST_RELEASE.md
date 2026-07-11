version: 1.5.1783742640
---
### Fixed
- Retried transient release-channel HTTP failures during asset hydration,
  binary update checks, installer downloads, and post-release validators so
  installs survive dropped connections and IPv6 no-route hosts.

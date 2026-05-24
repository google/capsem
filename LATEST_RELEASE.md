version: 1.2.1779665141
---
### Fixed
- Fixed the Linux install test harness clean-state path to stop the systemd
  user unit before killing scoped Capsem processes, preventing `Restart=always`
  from racing tests that intentionally replace `capsem-service` with a broken
  binary.

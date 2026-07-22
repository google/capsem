version: 1.5.1784695365
---
### Fixed
- Made headless macOS package installs prove the exact non-root target user and
  package receipt with labeled assertions, while retaining unconditional user,
  app-path, receipt, and Installer diagnostics for failed release jobs.
- Started the local Docker backend before release-gate storage preflight so a
  correctly stopped Colima daemon cannot fail qualification before bootstrap.

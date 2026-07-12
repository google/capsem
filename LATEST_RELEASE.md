version: 1.5.1783894498
---
### Changed
- Added a Stage 0 clean-container install-harness preflight to `just test`,
  before audits/builds/VMs, plus ordering contracts and agent/skill policy. It
  proves the container-owned uv environment can launch pytest early while the
  complete Docker/systemd package install E2E remains mandatory later.

### Fixed
- Made the host exec-output transport retry interrupted socket reads instead
  of publishing an empty or partial buffer with the guest command's successful
  exit code. This prevents release replays from losing output emitted at
  process exit while preserving the real exit status.

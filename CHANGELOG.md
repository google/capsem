# Changelog

All notable changes to CAPSEM will be documented in this file.

## [0.3.1] - 2025-10-15

### Added

- [CAPSEM Proxy] Added Gemini proxy demo notebook.

### Fixed

- [CAPSEM Proxy] Fixed issue where PII policy was not being applied to tool call in some cases.

### Changed

- Changed directory structure to move capsem to capsem_python so it is clearer
and we can add other packages like capsem_js in the future.


## [0.3.0] - 2025-10-10

### Added
- [CAPSEM] PII Security Policy that can block, confirm, or log based on detected PII types in model and tools responses.

- [CAPSEM] config/ driven policy configuration for easy customization.

- [CAPSEM Proxy] `run-proxy.py` script to run proxy in production mode.

## [0.2.0] - 2025-10-09

### Added

- Added `capsem_proxy` package that allows proxying requests through CAPSEM to enforce privacy and security policies on external models.

- Added documentation for `capsem_proxy` in the README.

### Changed

- Updated the github structure to include the new `capsem_proxy` package by moving the `capsem` to a subdirectory.

- Changed the debug policy to block on the keyword "capsem_block" instead of "block" to avoid accidental triggers.


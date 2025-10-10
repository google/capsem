# Changelog

All notable changes to CAPSEM will be documented in this file.

## [1.1.1] - 2025-10-09

### Added

- Added `capsem_proxy` package that allows proxying requests through CAPSEM to enforce privacy and security policies on external models.

- Added documentation for `capsem_proxy` in the README.

### Changed

- Updated the github structure to include the new `capsem_proxy` package by moving the `capsem` to a subdirectory.

- Changed the debug policy to block on the keyword "capsem_block" instead of "block" to avoid accidental triggers.


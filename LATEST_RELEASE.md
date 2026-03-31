version: 0.14.20
---
### Fixed
- **CI release upload collision on per-arch VM assets** -- `gh release upload "$f#${arch}-${base}"` sets the display label, not the filename. Both arches uploaded `initrd.img`, causing a name collision. Fixed by renaming files to `${arch}-${base}` before upload.

version: 1.0.1776984283
---
### Fixed (CI)
- install-test: chown entire /src to capsem uid (was only /src/frontend);
  Tauri build.rs hit EACCES under the narrower chown.

version: 1.0.1776982455
---
### Fixed (CI)
- install-test container: chown full /src/frontend (not just node_modules)
  so vite/astro temp writes work when runner uid (1001) != container uid (1000).

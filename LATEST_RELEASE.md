version: 1.5.1784493824
---
### Added
- Added a scheduled/manual blocking dependency-audit workflow, while keeping
  upstream RustSec clock changes out of ordinary PR and candidate gates.
- `dev-ci` skill: CI triage procedure and stop-the-line red-gate discipline
  (named diagnosis before any rerun, streaks are P0, failure classification).
- Cross-agent skill index contract test (`tests/test_agent_skill_index.py`):
  every skill must be indexed in CLAUDE.md and GEMINI.md, no dangling skill
  references in any agent instruction file, discovery symlinks must resolve to
  canonical `skills/`, and both indexes must carry the AGENTS.md hard-contract
  pointer. The test runs in the PR gate's Python schema lane and in the full
  `just test` gate.

### Changed
- Pinned Rust 1.97.1 across local, CI, bootstrap, and builder environments;
  pinned external GitHub Actions to immutable commits; unified artifact
  uploads; and replaced source-built CI utilities with reviewed prebuilts.
- CI now runs on `main`, cancels only superseded PR runs, treats token-gated
  Codecov uploads as non-blocking, and uses structural docs/site smokes.
- Split `release-process` and `dev-testing` skills into sub-500-line spines
  with on-demand `references/` files (release graph, CI invariants, Apple
  signing, post-release verification, local/CI parity, test matrix, MCP debug
  tools) -- content moved verbatim.
- Brought GEMINI.md's skill index to full parity with CLAUDE.md and made both
  files' AGENTS.md pointers name the release-evidence and logger-DB hard
  contracts; indexed the previously missing `ironbank`, `dev-bug-review`,
  `meta-find-skills`, and `meta-skill-creation` skills.

### Fixed
- Bound `just test` to one clean committed `HEAD`, routed automatic benchmark
  output under `target/`, ignored new benchmark recordings until deliberately
  approved, and made missing release-site Astro fail immediately with the
  owning install step named.
- Made the public GitHub package release inert until candidate URLs, SHA-256,
  BLAKE3, and `install.sh` pass, then advance the user-visible channel.

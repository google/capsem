version: 1.6.1785322564
---
### Changed

- Standardized the blocking Python coverage floor at an exact 85% in ordinary
  CI and the complete local release gate.

### Fixed

- Corrected the `CLAUDE.md` project layout against the real tree: `guest/config/`
  has not existed since the ontology cleanup, so profile definitions now point at
  `config/profiles/<id>/` and are named as runtime product config rather than
  developer skill source, `src/capsem/builder/` is described as the admin-driven
  backend it became, and the four shipped crates plus `config/`, `tests/`, and
  `scripts/` are no longer missing from the map.
- Repointed the MITM skill's static CA keypair at `security/keys/`, where
  `net/cert_authority.rs` actually reads it from, instead of a `config/` path
  that the five-directory config contract would never allow.
- Retried transient release-catalog reads in the runtime preflight instead of
  letting one reset CDN connection fail the first gating step of both release
  lanes. Authoritative 4xx answers still fail closed on the first attempt.
- Covered the installed-product half of candidate channel preservation: a
  preserved transition must leave `manifest-metadata.json` on its packaged
  public channel, which is exactly what the published release gate reads back.
- Brought the build-provenance invariant into the test suite: the embedded
  build hash must carry the real source revision whenever one is readable,
  rather than relying on `check-build-provenance.sh` alone to keep a binary
  with no source identity out of a release.
- Made the host-builder container trust its bind-mounted `/src` checkout. On
  Linux the host UID differs from the container user, so git rejected the
  repository and the package build died after a successful compile; the same
  condition silently degrades the embedded build hash to `unknown`.
- Generalized the pnpm cache-ownership gate to follow the Just recipe graph
  instead of accepting one hardcoded recipe name, so every just-driven CI job
  can cache its pnpm store. Re-enabled that cache on the install and Linux
  gates, and factored the recipe reachability both contracts need into one
  shared `scripts/justfile-graph.py`.
- Installed the musl C toolchain in the ordinary CI install gate, which runs
  `just doctor` through `_cross-compile` and needs it to build guest binaries.
- Provisioned the tools each CI job actually invokes: `just` in the macOS
  `test` job, whose `tests/capsem-release/` contracts shell out to it, and
  pnpm/Node in `test-install` and `test-linux`, whose `just` recipes reach
  `_pnpm-install`. Both gaps failed only on CI, because local `just test` runs
  where every tool is already on PATH.
- Added a checked-in contract asserting every job in `ci.yaml`, `release.yaml`,
  and `release-assets.yaml` installs the tools its own steps invoke, resolving
  justfile recipe dependencies transitively so the fast local gate fails first
  instead of discovering provisioning drift in CI.
- Preserved manifest-declared channels during candidate package installation
  and accepted selected release graphs in the shared bootstrap suite.
- Made ordinary CI build the exact native release-mode Debian package before
  the install/glow-up gate, with an actionable fail-closed diagnostic when the
  expected package is absent.
- Refreshed embedded package provenance across cached commits and worktrees,
  and made every local and CI package builder reject a binary whose build
  identity does not match the exact release source.
- Made guest-kernel resolution fail closed when kernel.org is unavailable or
  the configured branch is EOL, and moved both architectures from EOL 7.0 to
  the supported 6.18 LTS branch.

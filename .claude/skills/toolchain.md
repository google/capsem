# Capsem Toolchain Skill

How to build, test, and ship Capsem. All workflows use `just` (not make).

## Daily Development

```bash
just dev                              # Hot-reload app (frontend + Rust, full Tauri)
just ui                               # Frontend-only dev server (mock mode, no VM)
just run                              # Build + boot VM interactively (~10s)
just run "echo ok"                    # Build + boot + run command + exit
```

`just run` is the daily driver. It cross-compiles the guest agent, repacks the initrd, builds the host binary, codesigns, and boots the VM. Pass a command string to run non-interactively.

## Testing

Three tiers, from fast to thorough:

| Tier | Command | What it does | VM? |
|------|---------|-------------|-----|
| Fast | `just test` | Unit tests (llvm-cov) + cross-compile agent + frontend type-check + build | No |
| Smoke | `just run "capsem-doctor"` | Repack + boot VM + run diagnostic suite | Yes |
| Full | `just full-test` | `test` + capsem-doctor + integration test + bench | Yes (3x) |

Always run `just test` before pushing. Run `just full-test` before releases.

### What each tier catches

- **`just test`**: Rust logic bugs, cross-compile failures (platform-specific types), frontend type errors, bundling issues
- **`just run "capsem-doctor"`**: Sandbox integrity, network isolation, MITM trust chain, runtime availability, MCP gateway
- **`just full-test`**: All of the above + telemetry pipeline correctness (fs/net/mcp/model/tool events) + performance regressions

## Release & Install

```bash
just release                          # full-test + build release .app + sign + DMG
just install                          # full-test + build release .app + sign + /Applications
```

Both gate on `full-test` passing first. `just release` produces `target/release/Capsem.dmg`. `just install` copies the `.app` to `/Applications` and launches it.

## VM Assets

```bash
just build-assets                     # Full rebuild: kernel + initrd + rootfs (~10 min, Docker/Podman)
```

Only needed when changing `Dockerfile.rootfs`, `capsem-bashrc`, `diagnostics/`, installed packages, or kernel config. Guest binary changes (capsem-init, capsem-agent, capsem-net-proxy, capsem-mcp-server, capsem-fs-watch) are handled by `just run` via initrd repack.

## Utilities

```bash
just bench                            # In-VM benchmarks (disk I/O, rootfs, CLI startup, HTTP)
just inspect-session                  # Latest session DB integrity + event summary
just inspect-session <id>             # Specific session
just update-fixture <path>            # Copy + scrub real session DB as test fixture
just update-prices                    # Refresh model pricing JSON
just clean                            # Remove all build artifacts
```

## Recipe Dependency Graph

```
run         -> _pack-initrd + _sign (-> _compile -> _frontend)
test        -> _ensure-tools
full-test   -> test + _sign
release     -> full-test + _frontend
install     -> full-test + _frontend
build-assets -> _ensure-tools + test
bench       -> _sign
```

Internal `_`-prefixed recipes are hidden from `just --list` but called as dependencies.

## When to Use Which

| Situation | Command |
|-----------|---------|
| Changed Rust code (host-side) | `just run` or `just test` |
| Changed guest binary (agent, net-proxy, mcp-server, fs-watch) | `just run` |
| Changed capsem-init | `just run` |
| Changed rootfs (Dockerfile, bashrc, diagnostics) | `just build-assets` then `just run` |
| Changed frontend | `just ui` (iterate) then `just test` (validate) |
| Verify telemetry pipelines | `just run "<exercise command>"` then `just inspect-session` |
| Pre-release validation | `just full-test` |
| Ship a release | `just release` (DMG) or `just install` (local) |

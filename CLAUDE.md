# Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework. Built with Rust, Tauri 2.0, and Astro.

## Build Commands

All workflows use `just` (not make):

- `just doctor` -- check all required tools are installed (run first on a new machine)
- `just dev` -- hot-reloading dev server (Tauri dev mode)
- `just ui` -- frontend-only dev server with mock data (no VM needed)
- `just run` -- cross-compile agent + repack initrd + build + codesign + boot VM (~10s)
- `just run "CMD"` -- same but run a command instead of interactive shell
- `just build-assets` -- doctor + full VM asset build from scratch (kernel, initrd, rootfs) via Docker/Podman
- `just test` -- unit tests + cross-compile check + frontend type-check (no VM)
- `just full-test` -- test + capsem-doctor + integration test + bench (boots VM multiple times)
- `just bench` -- in-VM benchmarks (scratch disk I/O, rootfs read, CLI startup latency, HTTP throughput)
- `just install` -- doctor + full-test + release `.app` + codesign + install to /Applications + launch
- `just clean` -- clean build artifacts
- `just inspect-session [id]` -- inspect session DB integrity and event summary (latest by default)
- `just update-fixture <path>` -- copy a real session DB as the test fixture (scrubs keys, syncs to frontend)
- `just update-prices` -- update model pricing JSON

### Dependency chains

```
doctor           read-only tool check (user-facing, fails on missing)
_install-tools   auto-installs rust targets/components/cargo tools (internal)
_check-assets    verifies VM assets exist, fails with "run just build-assets" if not

run            -> _check-assets + _pack-initrd -> _sign -> _compile -> _frontend
test           -> _install-tools
build-assets   -> doctor + _install-tools
full-test      -> test + _check-assets + _pack-initrd + _sign
install        -> doctor + full-test + _frontend
```

First-time setup: `just doctor` then `just build-assets`.
Daily dev: `just run` (fast, ~10s). Before release: `just release`.

## Project Layout

```
crates/capsem-core/       VM library (config, boot, serial, vsock, machine)
crates/capsem-app/        Tauri binary (GUI, CLI, commands, state)
crates/capsem-agent/      Guest PTY agent (vsock bridge, cross-compiled for aarch64-linux-musl)
frontend/                 Astro 5 + Svelte 5 + Tailwind v4 + DaisyUI v5
  src/lib/components/     Svelte components (App, Sidebar, StatusBar, Terminal, etc.)
  src/lib/views/          View components (TerminalView, NetworkView, SettingsView, SessionsView)
  src/lib/stores/         Svelte 5 rune stores (vm, network, settings, theme, sidebar)
  src/lib/icons/          SVG icon components
  src/lib/types.ts        TypeScript types mirroring Rust IPC structs
  src/lib/api.ts          Typed Tauri invoke/listen wrappers
  src/components/         Web components (capsem-terminal xterm.js)
  src/pages/index.astro   Thin shell rendering <App client:only="svelte" />
images/                   VM image tooling (Dockerfiles, build.py, capsem-init)
assets/                   Built VM assets (gitignored)
```

## Planning

The overall project plan and milestone roadmap is in `docs/overall_plan.md`.

## Architecture

- **Host**: Tauri app creates a VZVirtualMachine with a virtio serial console (boot logs) and a vsock device (terminal I/O + control + MITM proxy)
- **Guest**: Linux VM boots with `console=hvc0`, runs `capsem-init` as PID 1 which sets up air-gapped networking (dummy0 NIC + fake DNS + iptables), launches `capsem-net-proxy` and `capsem-pty-agent`
- **Guest Agent**: `capsem-pty-agent` creates a PTY pair, forks bash, and bridges master PTY <-> vsock (port 5001 for terminal data, port 5000 for control messages like resize/heartbeat)
- **Net Proxy**: `capsem-net-proxy` listens on TCP 127.0.0.1:10443, bridges each connection to host vsock port 5002. iptables redirects port 443 traffic here.
- **MITM Proxy**: Host terminates TLS from guest using a per-domain minted certificate (signed by static Capsem CA), inspects HTTP request (method, path, headers, body), applies domain+HTTP policy, forwards to real upstream over TLS, records full telemetry to web.db
- **Terminal I/O**: Frontend xterm.js `onData` -> Tauri `serial_input` command -> vsock fd (or serial fallback) -> guest PTY. Reverse: guest PTY -> vsock -> `CoalesceBuffer` (8ms/64KB) -> Tauri event -> xterm.js `write`
- **Serial**: Stays active for kernel boot logs. Terminal I/O switches to vsock once the guest agent sends `Ready`

### Vsock Ports
| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy (HTTPS connections) |
| 5003 | MCP gateway (tool routing, NDJSON passthrough) |
| 5005 | Filesystem events (inotify watcher telemetry) |

### Network Policy
- User config: `~/.capsem/user.toml` -- editable domain allow/block lists + HTTP rules
- Corp config: `/etc/capsem/corp.toml` -- enterprise lockdown (MDM-distributed)
- Merge: corp fields override user fields entirely; unspecified fields fall through to user, then hardcoded defaults
- HTTP rules: `[[network.rules]]` sections in TOML allow method+path matching per domain
- Per-session telemetry: `~/.capsem/sessions/<vm_id>/web.db` (domain, method, path, status code, headers, body preview)

### MITM CA
- Static CA: `config/capsem-ca.key` + `config/capsem-ca.crt` (ECDSA P-256, 100-year validity)
- Baked into rootfs via `update-ca-certificates` + certifi patch
- Guest trusts it via system store + env vars (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`)
- No security from the CA itself -- the guest is already fully sandboxed

## Key Files

- `images/capsem-init` -- guest init script (PID 1). Changes require `just run` to take effect (repacks initrd automatically).
- `images/capsem-bashrc` -- guest shell config (baked into rootfs, requires `just build`)
- `images/README.md` -- full documentation of the guest VM environment (packages, banner, tips, AI status)
- `crates/capsem-agent/src/main.rs` -- guest PTY agent (vsock bridge, cross-compiled)
- `crates/capsem-agent/src/net_proxy.rs` -- guest TCP-to-vsock relay (cross-compiled)
- `crates/capsem-core/src/net/` -- network modules (MITM proxy, cert authority, HTTP policy, domain policy, SNI parser, policy config, telemetry)
- `crates/capsem-core/src/net/mitm_proxy.rs` -- async MITM proxy (rustls + hyper): TLS termination, HTTP inspection, upstream bridging
- `crates/capsem-core/src/net/cert_authority.rs` -- CA loader + on-demand domain cert minting with RwLock cache
- `crates/capsem-core/src/net/http_policy.rs` -- method+path policy engine (extends domain-level policy)
- `config/defaults.toml` -- settings registry (all built-in settings, embedded at compile time). See `docs/config.md` for format reference.
- `config/capsem-ca.key` + `config/capsem-ca.crt` -- static MITM CA keypair (ECDSA P-256)
- `crates/capsem-app/src/commands.rs` -- Tauri IPC commands (serial_input, vm_status, terminal_resize, net_events)
- `crates/capsem-app/src/state.rs` -- per-VM state (serial + vsock fds)
- `crates/capsem-core/src/vm/serial.rs` -- serial console pipe setup (boot logs)
- `crates/capsem-core/src/vm/vsock.rs` -- vsock manager, control messages, coalescing buffer
- `crates/capsem-core/src/vm/machine.rs` -- VZVirtualMachine wrapper (serial + vsock devices)
- `frontend/src/components/capsem-terminal.ts` -- xterm.js web component (resize events)
- `frontend/src/pages/index.astro` -- main UI page

## Test Fixture (data/fixtures/test.db)

The UI mock data and `capsem-logger` roundtrip tests share a single SQLite fixture captured from a real Capsem session. No synthetic data generators -- real data only.

**Updating the fixture** after a real session:

```
just update-fixture ~/.capsem/sessions/<session-id>/web.db
```

This:
1. Checkpoints WAL into the main DB file
2. Scrubs any leaked API keys (`sk-ant-*`, `AIza*`, `Bearer` tokens)
3. Verifies no keys remain (aborts on failure)
4. Copies to both `data/fixtures/test.db` and `frontend/public/fixtures/test.db`

The fixture is loaded by:
- **Frontend mock mode** (`mock.ts`): sql.js loads it from `/fixtures/test.db` for browser-only dev
- **Rust tests** (`capsem-logger/tests/roundtrip.rs`): reads it as a pre-populated DB for query tests

## Testing

```
cargo test --workspace
```

**Testing policy -- TDD is mandatory. Every feature must ship with tests:**

0. **TDD workflow (red-green-refactor)**: Always write tests FIRST, before writing implementation code. The sequence is: (1) write failing tests that capture the expected behavior, (2) verify they fail for the right reason, (3) write the minimal implementation to make them pass, (4) refactor. Do NOT write implementation code without a failing test that motivates it.
1. **Unit tests**: Every new module, struct, or non-trivial function gets unit tests in a `#[cfg(test)] mod tests` block. Cover happy path, edge cases, and error paths.
2. **Adversarial tests**: This is a security product. Think like an attacker trying to escape the sandbox, bypass policy, or exploit edge cases. Every security-relevant feature (policy evaluation, allow/block lists, corp lockdown, input validation, guest injection) must include tests that actively try to break the invariants. Examples: can a user sneak a corp-blocked domain through another provider's domain list? Does an overlapping wildcard in allow+block always deny? Does malformed input (empty strings, unicode, huge payloads, invalid JSON) get rejected? Stress-test boundary conditions and write tests for the attacks you'd attempt yourself.
3. **Integration tests**: Features that cross crate boundaries or touch VM lifecycle get integration tests in `crates/capsem-core/tests/vm_integration.rs`.
4. **Run the full suite**: Before considering any work complete, run `cargo test --workspace` and `just test` (which includes cross-compile + frontend build). All tests must pass.
4b. **After any telemetry/logging change**: Run a real session and verify with `just inspect-session` that all tables (model_calls, tool_calls, tool_responses, mcp_calls, net_events, fs_events) are populated correctly with model names, token counts, and tool origins.
5. **Testable design**: Extract logic into standalone, testable functions/structs in `capsem-core` rather than embedding it in the app layer where it's coupled to Tauri. If you can't test it, refactor until you can.

## In-VM Diagnostics (capsem-doctor)

The `capsem-doctor` suite runs inside the guest VM to verify sandbox integrity, network isolation, and runtime environment. Tests are pytest-based, live in `images/diagnostics/`, and are baked into the rootfs via `Dockerfile.rootfs`.

**Running diagnostics:**
- `just run "capsem-doctor"` -- repack + build + sign + boot VM + run capsem-doctor + shut down (fast path, ~10s)
- Inside a running VM: `capsem-doctor` (all tests), `capsem-doctor -k sandbox` (subset), `capsem-doctor -x` (stop on first failure)

**Test categories:**

| File | Tests | What it verifies |
|------|-------|------------------|
| `test_sandbox.py` | Security boundaries | Read-only rootfs, binary permissions, setuid/setgid, kernel hardening (no modules, no debugfs, no IPv6, no swap, no kallsyms), process integrity (pty-agent, dnsmasq, no systemd/sshd/cron), network isolation (dummy0, fake DNS, iptables, allowed/denied domains, no real NICs) |
| `test_network.py` | MITM proxy & trust chain | CA in system store, CA in certifi, curl without -k, Python urllib HTTPS, CA env vars (SSL_CERT_FILE, REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS), HTTP port 80 blocked, non-443 ports blocked, direct IP blocked, multi-domain DNS faking, AI provider domains blocked |
| `test_environment.py` | VM configuration | TERM, HOME, PATH env vars, shell is bash, kernel version, aarch64 arch, mount points (/proc, /sys, /dev, /dev/pts), tmpfs verification |
| `test_runtimes.py` | Dev tool execution | Python3, Node.js, npm, pip3, git version checks; Python/Node execution with file I/O; git init/commit workflow |
| `test_utilities.py` | Tool availability | ~36 unix utilities (coreutils, text processing, network, system inspection) |
| `test_workflows.py` | File I/O patterns | Text write/read, JSON roundtrip (Python + Node), shell pipes, large file (10MB) |
| `test_ai_cli.py` | AI CLI sandboxing | claude/gemini/codex installed and executable without crashing |

**Adding new in-VM tests:**
1. Add test functions to the appropriate `images/diagnostics/test_*.py` file, or create a new `test_<category>.py`
2. Use `from conftest import run` for shell commands, `output_dir` fixture for temp files
3. Tests auto-skip outside the capsem VM (conftest.py checks for root + writable /root)
4. Rebuild rootfs with `just build` to pick up new/modified test files
5. Verify with `just run "capsem-doctor"`

## Ephemeral VM Model -- Invariants (do not break)

Every VM session is fully stateless. Two invariants in `images/capsem-init` must never be violated:

1. **`mke2fs` runs unconditionally** at boot -- the scratch disk is always formatted fresh. No ext4 detection, no skip.
2. **Overlay `upperdir` is always tmpfs** (`mount -t tmpfs tmpfs /mnt/b`). It must never be the scratch disk.

Breaking either invariant allows rootfs writes to survive across sessions, violating the sandbox model. `scripts/preflight.sh` (`check_ephemeral_model`) enforces both statically. `scripts/integration_test.py` (`check_persistence`) verifies ephemerality end-to-end by booting two consecutive VMs and confirming a file written in the first is absent in the second.

**To add packages to the VM:** edit `images/Dockerfile.rootfs` and run `just build-assets`. Never try to make the overlay upper layer persistent.

## Notes

- The binary must be codesigned with `com.apple.security.virtualization` or VZ calls crash. The justfile handles this.
- `capsem-init` uses `setsid` to give bash a controlling terminal so Ctrl-C (SIGINT) works. Without setsid, the tty has no foreground process group and signals are not delivered.
- The initrd is a gzip+cpio archive. `just run` automatically repacks it, replacing `/init` and bundling the cross-compiled guest binaries. Changes to the rootfs (bashrc, installed packages) require `just build-assets`.

## Initrd Repack: Fast Iteration for Guest Binaries

`just run` automatically repacks the initrd before every boot -- cross-compiles guest binaries, injects them into the initrd, and `capsem-init` prefers initrd-bundled copies over rootfs copies at boot. No separate repack step needed.

**Currently repacked binaries:**
- `capsem-init` -- PID 1 init script
- `capsem-pty-agent` -- PTY-over-vsock bridge agent
- `capsem-net-proxy` -- TCP-to-vsock relay for air-gapped HTTPS proxying
- `capsem-mcp-server` -- MCP stdio-to-vsock relay for AI agent tool access
- `capsem-fs-watch` -- inotify file watcher daemon for filesystem telemetry
- `capsem-doctor` -- VM self-diagnostic suite (bash script)
- `diagnostics/` -- pytest test files for capsem-doctor

**When adding a new guest binary**, update three places:
1. `_pack-initrd` recipe in `justfile` -- add the cross-compile + copy step
2. `capsem-init` in `images/capsem-init` -- add initrd-bundled fallback logic (check `/binary` before rootfs path)
3. This section in `CLAUDE.md` -- add it to the list above

**When to use which:**
- `just run` -- changed `capsem-init`, `capsem-agent`, `capsem-doctor`, `diagnostics/`, or any repacked binary (~10s)
- `just build-assets` -- changed `Dockerfile.rootfs`, `capsem-bashrc`, installed packages, or added new rootfs files (minutes)

## Guest Binary Security

All guest-side binaries (PTY agent, future credential helpers, etc.) are deployed read-only:
- **Rootfs**: `chmod 555` in `Dockerfile.rootfs` (rootfs itself is mounted read-only)
- **Initrd override**: `chmod 555` in `_pack-initrd` and `capsem-init` after copying to tmpfs
- Guest processes cannot modify these binaries at runtime

## Changelog

We use [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format in `CHANGELOG.md`. **Every user-visible change must get a changelog entry.**

- Add entries under `## [Unreleased]` using the categories: Added, Changed, Deprecated, Removed, Fixed, Security
- Write entries from the user's perspective (what changed), not implementation details
- Update the changelog as part of every commit that adds, fixes, or changes user-visible behavior
- When a release is cut, move Unreleased items into a new `## [X.Y.Z] - YYYY-MM-DD` section and bump the version (see Versioning)

## Commits

Every commit that touches code should:
1. Include the relevant `CHANGELOG.md` update in the same commit
2. Stage files explicitly (no `git add -A`)
3. Use conventional-ish messages: `feat:`, `fix:`, `chore:`, `docs:`
4. Author: Elie Bursztein <github@elie.net>

## Versioning

The project follows [Semantic Versioning](https://semver.org/). The single source of truth for version is `workspace.package.version` in the root `Cargo.toml`. Crates inherit it via `version.workspace = true`. The version in `crates/capsem-app/tauri.conf.json` must be kept in sync manually.

When bumping the version, update both:
1. `workspace.package.version` in `/Cargo.toml`
2. `version` in `crates/capsem-app/tauri.conf.json`

## CI Secrets (GitHub Actions)

All secrets are set on the repo and used by `release.yaml`:

| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` (Developer ID Application cert + private key) |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting the `.p12` file |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_API_ISSUER` | App Store Connect API issuer UUID (for notarization) |
| `APPLE_API_KEY` | App Store Connect API key ID (for notarization) |
| `APPLE_API_KEY_PATH` | Contents of the `.p8` private key file (for notarization) |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater signing private key (minisign) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password protecting the Tauri signing key |

Local backups of all credentials are in `private/` (gitignored):
- `private/apple-certificate/` -- `.p12`, `.p8`, base64, passwords, team ID
- `private/tauri/` -- signing key, public key, password

**p12 encryption gotcha**: macOS `security import` only supports legacy PKCS12 (3DES/SHA1). OpenSSL 3.x creates PBES2/AES-256-CBC by default, which Keychain rejects with a misleading "wrong password" error. If the p12 was created or re-exported with modern OpenSSL, run `scripts/fix_p12_legacy.sh` to convert it, then upload with `gh secret set APPLE_CERTIFICATE < private/apple-certificate/capsem-b64.txt`.

## Release Preflight

The CI release workflow runs a `preflight` job before anything else to fail fast on credential/config issues. Locally, run `scripts/preflight.sh` to validate:
- Required tools (openssl, codesign, cargo, pnpm, node, gh)
- Rust cross-compile target (aarch64-unknown-linux-musl)
- Apple certificate format and keychain import
- Base64 file in sync with p12

When adding new release prerequisites, add a `check_*` function to `scripts/preflight.sh`.

## Release Process

Releases are CI-only via `.github/workflows/release.yaml`. Push a `vX.Y.Z` tag to trigger the pipeline (preflight -> build-assets -> test -> build-app with codesign + notarize + DMG + GitHub Release).

`just install` runs `doctor` + `full-test` before building locally. The full-test gates are:

| Gate | What it does |
|------|-------------|
| Unit tests | `cargo llvm-cov` -- all workspace tests with coverage |
| Cross-compile | Build `capsem-agent` for `aarch64-unknown-linux-musl` |
| Frontend check | `pnpm run check && pnpm run build` |
| capsem-doctor | Boot VM, run sandbox/network/MCP/runtime/utility/AI CLI diagnostics |
| Integration test | Boot VM, exercise all 6 telemetry pipelines, verify session DB + main.db rollup |
| Benchmark | Boot VM, run `capsem-bench` (disk I/O, rootfs read, CLI startup, HTTP latency) |

All gates must pass before the `.app` bundle is built. Requires API keys in `~/.capsem/user.toml` (Gemini key needed for the integration test's model_calls verification).

To run the full validation suite without building: `just full-test`

## Frontend / UI Development

The frontend is Astro 5 + Svelte 5 + Tailwind v4 + DaisyUI v5. It runs in mock mode in any browser (no VM needed).

### Workflow

1. **Start the dev server**: `just ui` (Astro dev server on `http://localhost:5173`)
2. **Open in Chrome**: use the Chrome DevTools MCP to inspect and screenshot the UI -- take screenshots of each view (Terminal, Sessions, Network, Settings) and both themes (light/dark) to verify layout and visual polish
3. **Iterate**: edit Svelte components in `frontend/src/lib/`, the dev server hot-reloads
4. **Type check**: `cd frontend && pnpm run check` (astro check + svelte-check)
5. **Production build**: `cd frontend && pnpm run build` (catches bundling issues that dev mode misses)
6. **Verify in Tauri**: `just dev` runs the full Tauri app with hot-reloading (needs VM assets built)

### Mock mode

When `window.__TAURI_INTERNALS__` is absent (i.e. running in a browser via `just ui`), `src/lib/api.ts` auto-switches all IPC calls to return fake data from `src/lib/mock.ts`. Mock includes: VM state "running", 5 network events (3 allowed, 2 denied), 6 settings across 4 categories, VM state timeline with 4 transitions, and a terminal banner. All views are fully functional with mock data.

### Checking the UI visually

Use the Chrome DevTools MCP (`mcp__chrome-devtools__*` tools) to inspect the running UI:
- `list_pages` / `navigate_page` -- open `http://localhost:5173`
- `take_screenshot` -- capture the current view (use `fullPage: true`)
- `take_snapshot` -- get the a11y tree (element UIDs for clicking)
- `click` -- navigate between views by clicking sidebar buttons
- `list_console_messages` with `types: ["error", "warn"]` -- check for runtime errors

Walk through all four views (Console, Sessions, Network, Settings) and toggle the theme to verify both light and dark modes look correct.

### Key files

- `frontend/src/lib/components/App.svelte` -- root layout (sidebar + content + status bar)
- `frontend/src/lib/components/Sidebar.svelte` -- collapsible nav rail
- `frontend/src/lib/views/` -- one file per view (TerminalView, NetworkView, SettingsView, SessionsView)
- `frontend/src/lib/stores/` -- Svelte 5 rune stores (vm, network, settings, theme, sidebar)
- `frontend/src/lib/api.ts` -- typed Tauri IPC wrappers with auto-mock fallback
- `frontend/src/lib/mock.ts` -- fake data for browser dev mode
- `frontend/src/lib/types.ts` -- TS types mirroring Rust IPC structs
- `frontend/src/styles/global.css` -- Tailwind config with `@source` directives and DaisyUI plugin
### Design System

**Read `docs/design.md` before building or modifying any UI component.** It defines the color system, DaisyUI component usage policy, custom `@theme` tokens, and chart color semantics. Use the `frontend-design` skill for UI work.

### Chart Library (LayerChart v2)

Charts use [LayerChart](https://layerchart.com) v2 -- a composable Svelte charting library built on D3. **Full API docs are in `docs/libs/layercharts.md`** -- read it before building or modifying any chart component.

**Simplified chart components** (preferred for common patterns):
- `BarChart` -- vertical/horizontal bars, stacked/grouped series
- `PieChart` -- pie/donut charts with series support
- `AreaChart`, `LineChart` -- time-series and continuous data

**Key patterns used in stats views:**
- `series` prop: array of `{ key, label, color }` for multi-series charts
- `seriesLayout="stack"` for stacked bar charts
- `orientation="horizontal"` for horizontal bar charts
- `props={{ legend: { placement: 'bottom' } }}` for chart sub-component props
- Colors come from `css-var.ts` (never hardcoded in templates)

### Gotchas

- Tailwind v4 + `client:only` Svelte: Tailwind's Vite plugin cannot see `client:only` components in the SSR module graph. The `@source` directives in `global.css` explicitly include `.svelte` and `.ts` files.
- `vm-state-changed` payload is `{ state, trigger }` (object), not a plain string.
- Dynamic Svelte components: use `<svelte:component this={item.icon} />`, not `<item.icon />`.

## Logging

- Boot sequence instrumented with `tracing` spans (`FmtSpan::CLOSE` logs durations). Use `RUST_LOG=capsem=debug` for full boot timing breakdown, `RUST_LOG=capsem=info` for top-level only.

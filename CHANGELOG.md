# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Persistent logging system: three-layer tracing (stdout, per-launch JSONL file, Tauri UI layer) with per-VM log files in session directories (CLI + GUI)
- Logs view in sidebar with live event stream, boot timeline visualization, session history browser, level filtering, and auto-scroll
- Per-launch log files (`~/.capsem/logs/<timestamp>.jsonl`) with automatic 7-day cleanup
- Per-VM session logs (`~/.capsem/sessions/<id>/capsem.log`) with structured JSONL events for both CLI and GUI modes
- `load_session_log` and `list_log_sessions` Tauri commands for historical log access
- Error messages now included in `vm-state-changed` events for all error states
- Boot timeline state transitions emitted as structured tracing events
- Integration test verifies log file creation, JSONL validity, level filtering, boot timeline events, and timestamp format

### Added
- App auto-update: `createUpdaterArtifacts` enabled so CI produces `.tar.gz` + `.sig` updater files and `latest.json` -- the built-in updater now works
- `app.auto_update` setting (default: true) to gate the startup update check, with "Check for Updates" button in Settings > App
- Multi-version asset manifest (`manifest.json`) replaces single-version `B3SUMS` -- supports multiple release versions, merge across releases, and future checkpoint restore
- Version-scoped asset directories (`~/.capsem/assets/v{version}/`) with automatic migration from flat layout and cleanup of old versions
- `pinned.json` support for keeping specific asset versions during cleanup (for future checkpointing)
- `scripts/gen_manifest.py` for manifest generation in justfile and build.py

### Changed
- CI release workflow now accumulates manifest.json across releases and uploads it alongside rootfs
- `_pack-initrd` regenerates manifest.json on every `just run` via `scripts/gen_manifest.py`
- `build.rs` reads hashes from manifest.json (preferred) with B3SUMS fallback

### Fixed
- Tool calls no longer double-counted in stats -- MCP-proxied tool_calls (origin=mcp_proxy) filtered from native counts across all 6 tool queries
- Native tool response preview now displayed in unified tool list (was hardcoded NULL, now joined from tool_responses via call_id)
- Non-text content blocks (tool_reference, image) in Anthropic tool results now produce meaningful preview instead of empty string
- OpenAI multipart tool result content now extracted correctly
- `check_session.py` tool response matching fixed -- joins on call_id only (tool responses arrive in next model call with different model_call_id)

### Added
- First-run setup wizard -- 6-step guided configuration (Welcome, Security, AI Providers, Repositories, MCP Servers, All Set) that runs while the VM image downloads in the background
- Host config auto-detection -- wizard scans ~/.gitconfig, ~/.ssh/*.pub, environment variables, and `gh auth token` to pre-populate settings with detected values
- SSH public key setting (`vm.environment.ssh.public_key`) -- injected as /root/.ssh/authorized_keys in the guest VM at boot
- Re-run setup wizard button in Settings > VM to revisit configuration without overwriting existing settings
- Resumable asset downloads -- partial .tmp files are preserved across app restarts and continued via HTTP Range headers instead of re-downloading from scratch
- Security presets ("Medium" and "High") -- one-click security profiles selectable from Settings > Security
- Automatic migration of old setting IDs (`web.*`, `registry.*`) to new `security.*` namespace -- existing user.toml and corp.toml files work without manual changes

### Changed
- Settings restructured: "Web" and "Package Registries" merged under new "Security" top-level section with "Web", "Services > Search Engines", and "Services > Package Registries" sub-groups
- MCP gateway rewritten to use rmcp (official Rust MCP SDK) -- replaces hand-rolled JSON-RPC/SSE client with proper Streamable HTTP transport, automatic pagination, and typed tool/resource/prompt routing
- Upgraded reqwest from 0.12 to 0.13
- MCP server UI redesigned: collapsible server cards with URL/auth config, "verified"/"definition changed" status labels
- Tool origin telemetry expanded from 2 values (native/mcp) to 3 values (native/mcp_proxy/local)
- Auto-detected stdio MCP servers from Claude/Gemini settings shown with unsupported warning instead of silently dropped

### Removed
- Stdio bridge for MCP servers (`stdio_bridge.rs`) -- replaced by HTTP client

### Added
- `fetch_http` now supports `format=markdown` (new default) -- converts HTML to clean markdown preserving headings, links, lists, bold/italic, and code blocks
- Wikipedia (`en.wikipedia.org`, `*.wikipedia.org`) added to default allow list for MCP HTTP tools
- Auto-detect latest stable kernel version from kernel.org during `just build-assets`
- User-editable bashrc and tmux.conf as file settings in Settings > VM > Shell
- Filetype-aware syntax highlighting for file settings (bash, conf, json)

### Security
- Kernel hardening: heap zeroing (`INIT_ON_ALLOC`), SLUB freelist hardening, page allocator randomization, KPTI (`UNMAP_KERNEL_AT_EL0`), ARM64 BTI + PAC, `HARDENED_USERCOPY`, seccomp filter, cmdline hardening (`init_on_alloc=1 slab_nomerge page_alloc.shuffle=1`)

### Fixed
- MCP server now visible in `claude mcp list` -- was injected into wrong file (`settings.json` instead of `.claude.json`)
- Codex CLI MCP server config added (`~/.codex/config.toml`) -- was missing entirely
- Disabling an AI provider now takes effect immediately on existing keep-alive connections (policy was previously snapshot per-connection, not per-request, so in-flight HTTP/1.1 connections continued to allow requests after the provider was toggled off)
- MCP tool_responses no longer double-counted in multi-turn conversations (request parsers now extract only trailing tool results instead of full history)
- MCP call previews no longer truncated at 200 chars (removed hard truncation; 256KB cap_field safety net remains)
- `fetch_http` paginate now UTF-8 safe -- uses `floor_char_boundary` to avoid panics on multi-byte content (emoji, Cyrillic, CJK, etc.)
- `fetch_http` on subpaths (e.g. `elie.net/about`) now returns full page content -- replaced `tl` HTML parser with `scraper` (html5ever) which correctly handles minified/complex HTML
- `fetch_http` format default changed from `content` to `markdown` for better AI agent consumption
- MCP byte tracking: `bytes_sent`/`bytes_received` columns added to mcp_calls for full I/O auditability
- Builtin MCP tool HTTP requests now emit net_events with `conn_type=mcp_builtin` for network audit visibility
- Guest process_name resolution uses `/proc/{pid}/cmdline` (real binary name) instead of `/proc/{pid}/comm` (thread name), fixing "MainThread" attribution
- Gemini tool call_ids now include a counter suffix to distinguish multiple calls to the same function
- Claude Code no longer warns about missing `/root/.local/bin` directory (created at boot after scratch disk mount)
- tmux now has a clean minimal config: mouse support, no escape delay, proper 256-color/truecolor, high scrollback
- tmux sessions can now find `gemini` and other npm-global binaries (PATH was lost when tmux started a login shell that reset it via `/etc/profile`)
- `gh auth status` injection test no longer fails with fake test tokens (test now verifies token detection, not authentication)

### Changed
- `just install` now runs validation gates only (doctor + full-test); `.app` bundling is CI-only

### Added
- Documentation URLs for API key settings (links to provider console/settings pages)
- Repositories section in settings with git identity (author name/email) for VM commits
- Personal access token settings for GitHub and GitLab (enables git push over HTTPS via .git-credentials)
- GitLab as a repository provider with domain allow/block and token support
- Added `tmux` and `gh` to the default rootfs for terminal multiplexing and GitHub CLI support
- Token prefix hints in settings UI -- apikey inputs show expected format (e.g., `ghp_...`, `sk-ant-...`) with a warning if the entered value doesn't match
- `GH_TOKEN` / `GITHUB_TOKEN` env vars injected in VM when GitHub token is configured, enabling `gh` CLI without `gh auth login`
- `GITLAB_TOKEN` env var injected in VM when GitLab token is configured

### Changed
- Missing API key warnings now appear in the group header when collapsed, with a "Get key" link
- GitHub moved from "Package Registries" to "Repositories" section
- `registry.github.*` setting IDs renamed to `repository.github.*`
- Package Registries description updated to "Package manager registries"

### Fixed
- Git authentication in VM: switched from `.netrc` to `.git-credentials` + `credential.helper=store` so `git push` works out of the box
- "Get one" links in settings now open in host browser via `tauri-plugin-opener` (previously broken in Tauri webview)

### Security
- Git credential tokens now reject `@` and `:` characters (in addition to newlines) to prevent URL injection in `.git-credentials`

## [0.8.8] - 2026-03-07

### Added
- Proxy throughput benchmark (`capsem-bench throughput`): downloads 100 MB through the full MITM proxy pipeline and reports MB/s — baseline ~35 MB/s on Apple Silicon
- `capsem-bench` is now repacked into the initrd on every `just run`, so changes to the benchmark script take effect immediately without a full rootfs rebuild
- `ash-speed.hetzner.com` added to the default network allow list and integration test config for the throughput benchmark
- Rust integration test `mitm_proxy_download_throughput` (in `crates/capsem-core/tests/mitm_integration.rs`): validates 100 MB download through the proxy at the host level; marked `#[ignore]` so it runs only on demand
- `test_proxy_download_throughput` in `capsem-doctor` (`test_network.py`): in-VM Layer 7 test verifying end-to-end proxy throughput; skips gracefully if the speed-test domain is not in the allow list
- `docs/performance.md`: documents all benchmark modes, baseline numbers, proxy data path, and domain allow list setup
- `just run` now kills any existing Capsem instance before booting, preventing a stale GUI window from appearing alongside a CLI run
- Notarization credential verification in CI preflight job: validates Apple API key against `notarytool history` before spending time on build-assets and tests
- Notarization preflight check in `scripts/preflight.sh`: verifies `.p8` key, API Key ID, Issuer ID, and runs a live `notarytool history` test

### Fixed
- `capsem-init` now aborts boot (kernel panic) if the tmpfs mount for the overlay upper layer fails, preventing a silent degraded boot where writes land on the initramfs instead of the intended tmpfs
- `capsem-init` now creates `/mnt/b` before mounting tmpfs on it (missing `mkdir -p` caused the tmpfs mount to fail with "No such file or directory" on fresh initrds)
- CI release no longer hangs on first-time notarization: `--skip-stapling` flag submits for notarization without waiting for Apple's response (first-time notarization can take hours)

### Security
- Boot invariant enforcement: `capsem-init` fatal-exits on tmpfs or overlayfs mount failure rather than continuing with a wrong upper layer; preflight check verifies this abort is present

## [0.8.4] - 2026-03-06

### Added
- `apt-get install` support inside the VM: overlayfs mounts with `redirect_dir=on,metacopy=on` (requires `CONFIG_OVERLAY_FS_REDIRECT_DIR`, `CONFIG_OVERLAY_FS_INDEX`, `CONFIG_TMPFS_XATTR` in kernel config), enabling dpkg directory renames without EXDEV errors. Packages installed in a session are gone after shutdown (ephemeral model preserved).
- `apt-packages.txt`: declarative list of system packages baked into the rootfs — edit and `just build-assets` to add/remove packages.
- Debian apt sources switched to HTTPS (`deb.debian.org`, `security.debian.org`) in `Dockerfile.rootfs`; both domains added to the default network allow list so the MITM proxy forwards them.
- Package lists pre-populated at rootfs build time so `apt-get install` works inside a running VM without a prior `apt-get update`.
- `force-unsafe-io` dpkg config in `capsem-init`: skips redundant fsyncs on overlayfs.
- Claude Code installed as a native binary (downloaded directly from Anthropic's GCS release bucket) instead of via npm, removing the Node.js dependency for the Claude CLI.
- Ephemeral model preflight check (`check_ephemeral_model` in `scripts/preflight.sh`): statically verifies `capsem-init` never skips `mke2fs` and never uses the scratch disk as overlay upper layer.
- Ephemeral model end-to-end test (`check_persistence` in `scripts/integration_test.py`): boots two consecutive VMs, writes a sentinel file in the first, and asserts it is absent in the second.

### Changed
- `images/README.md` developer section now documents how to add packages from all sources (apt, pip, npm, runtime) with copy-paste examples.

### Security
- Ephemeral model invariants documented in `CLAUDE.md` and enforced by preflight + integration test to prevent accidental persistence anti-patterns from being introduced.

### Added
- `just doctor` command: checks all required dev tools, container runtime (docker/podman), Rust targets, and cargo tools are installed
- Release preflight checks (`scripts/preflight.sh`): validates Apple certificate format, keychain import, and base64 sync before CI release
- `scripts/fix_p12_legacy.sh`: converts OpenSSL 3.x p12 files to legacy 3DES format macOS Keychain accepts
- CI preflight job in release workflow: fails fast on certificate/credential issues before slow build jobs

### Changed
- Release builds are CI-only (removed `just release`); push a `vX.Y.Z` tag to trigger `.github/workflows/release.yaml`
- `just build-assets`, `just install` now run `just doctor` first to catch missing tools early
- `just run`, `just full-test`, `just bench` now verify VM assets exist before proceeding

### Fixed
- Apple certificate import in CI: re-exported p12 with legacy 3DES/SHA1 encryption (macOS rejects OpenSSL 3.x default PBES2/AES-256-CBC with misleading "wrong password" error)

### Added
- Configuration overrides via `CAPSEM_USER_CONFIG` and `CAPSEM_CORP_CONFIG` environment variables to support isolated testing and CI.
- Dedicated integration test configurations (`config/integration-test-user.toml` and `config/integration-test-corp.toml`) for reproducible end-to-end validation.
- Thin DMG distribution: rootfs excluded from app bundle, downloaded on first launch via asset manager with blake3 hash verification
- Asset manager (`asset_manager.rs`): checks, downloads, and verifies VM assets from GitHub Releases with streaming progress
- Download progress UI: full-screen progress bar shown during first-launch rootfs download
- CLI download support: `capsem "command"` auto-downloads rootfs with stderr progress if missing
- Squashfs support: boot_vm accepts both rootfs.squashfs (new) and rootfs.img (legacy) formats
- Release workflow uploads rootfs.squashfs as separate GitHub Release asset alongside the thin DMG
- Onboarding plan (`docs/onboarding.md`): first-launch wizard scope for credentials, MCP config, and guided setup
- AI stats tab: unified model analytics with stat cards (total calls, tokens, cost, models), model usage chart, token breakdown, cost-over-time, and provider distribution
- `StatCards.svelte` reusable component for stat card rows across all analytics tabs
- Chart color system (`css-var.ts`): provider hue families, model color assignment, file action colors, server palette -- all using oklch() constants (no CSS var lookups)
- LayerChart v2 API documentation (`docs/libs/layercharts.md`) for LLM-friendly chart development

### Changed
- Asset resolution in macOS app bundle now searches multiple paths in `Resources` (including nested Tauri v2 paths) for better reliability.
- Integration test isolated from host user settings and correctly maps `GOOGLE_API_KEY` to `GEMINI_API_KEY` for the internal VM CLI.
- Tauri asset bundling now uses a flat map to prevent deeply nested `_up_/_up_/assets` structures in the final package.
- `just dev` now automatically passes `CAPSEM_ASSETS_DIR` to ensure the VM boots during local development.
- Stats "Models" tab renamed to "Model" (AITab.svelte replaces ModelsTab.svelte)
- Network, Tools, and Files stats tabs rebuilt with LayerChart v2 simplified chart components (BarChart, PieChart) replacing raw D3/Chart.js primitives
- SQL queries expanded: per-model token/cost breakdowns, provider distribution, cost-over-time, tool success rates, file action breakdowns
- Wizard auto-show on first run removed (setup wizard is still accessible from sidebar)

### Fixed
- Integration test SQLite connection robustness improved by using plain paths instead of URI formatting.
- Anthropic API tracking: MITM proxy now strips `accept-encoding` for AI providers so SSE streaming responses arrive uncompressed. This fixes the issue where Anthropic usage and cost were recorded as NULL.
- AI telemetry pollution: `model_call` records are now strictly filtered to valid LLM API paths (e.g., `/v1/messages`), preventing metadata endpoints from generating spurious NULL traces.
- Fallback model extraction: Added regex-based fallback to extract the model name from truncated JSON request bodies when the 64KB preview buffer limit is reached.
- fs-watch telemetry drops: Fixed a race condition during VM boot where early vsock connections (like `fs-watch`) were dropped by the host before the terminal/control handshake completed.
- `scripts/run_signed.sh` now correctly refreshes the binary signature via `touch` after re-signing with entitlements.
- Build prerequisites documentation updated with `b3sum`, `tauri-cli`, and `musl-cross` toolchain requirements.
- capsem-doctor PATH: writable bin dirs (`/root/.npm-global/bin`, `/root/.local/bin`) now included so AI CLIs and npm globals are found
- Gemini CLI settings.json: added `homeDirectoryWarningDismissed` and `sessionRetention` to suppress first-run prompts
- AI provider domain-blocked test now skips when the provider is explicitly enabled by policy
- Integration test handles compressed session DBs (`session.db.gz`) after vacuum
- Integration test accepts `vacuumed` as valid terminal session status

### Changed
- capsem-doctor and diagnostics are now repacked into the initrd, so changes take effect with `just run` instead of requiring `just build-assets`
- `just full-test` now includes initrd repack to ensure latest guest code is deployed

### Added
- `config_lint()` function: validates all settings (JSON files, number ranges, choices, API key format, nul bytes, URL format) with clear human-readable error messages displayed inline in the settings UI
- `SettingsNode` tree API: backend exposes the TOML settings hierarchy as a nested tree with resolved values at leaves, replacing the flat list for UI rendering
- `get_settings_tree` and `lint_config` Tauri commands for the new tree-based settings UI
- UI debug skill (`.claude/skills/UI_debug.md`): comprehensive Chrome DevTools MCP-based visual verification checklist for the settings UI

### Changed
- File settings now store path and content together as `{ path, content }` objects instead of keeping `guest_path` in metadata -- path is the source of truth for MCP injection and guest config generation
- Guest config file permissions tightened from 0o644 to 0o600 (owner-only) since settings files may contain API keys
- JSON validation uses zero-allocation `serde::de::IgnoredAny` instead of parsing into `serde_json::Value`
- Settings UI fully rewritten: left nav and section content are auto-generated from the TOML settings tree. Adding new categories or settings to `defaults.toml` automatically appears in the UI with no frontend code changes. Replaced 6 hardcoded section components (ProvidersSection, McpSection, NetworkPolicySection, EnvironmentSection, ResourcesSection, AppearanceSection) and their icon imports with a single generic recursive renderer (`SettingsSection.svelte`)
- SubMenu component now supports optional icons (icon-less items render label only)

### Security
- File setting paths are validated: must start with `/`, must not contain `..`, warns on unusual paths not under `/root/` or `/etc/`

### Added
- File analytics section: stat cards, action breakdown chart, events-over-time chart, and searchable event table for filesystem activity tracking
- Setup wizard hook: auto-detects first run (no API keys configured) and shows a welcome view with provider setup shortcut
- Reveal/hide toggle for API key and password fields in provider settings
- Range hints (min/max) shown below number inputs in VM resource and appearance settings
- Dropdown rendering for settings with predefined choices

### Changed
- Analytics data separation: Models and MCP analytics sections now exclusively query session.db; cross-session data (sessions over time, avg calls per session) moved to Dashboard
- "Session stats" button in terminal footer now navigates to session-level AI analytics instead of cross-session dashboard
- MCP analytics stat cards expanded from 2 (total + avg/session) to 4 (total, allowed, warned, denied)

### Security
- main.db `query_raw` now enforces `PRAGMA query_only = ON` around user SQL execution, preventing write-through via SQL injection (e.g., `SELECT 1; DROP TABLE sessions`) in the `query_db` IPC command
- Read-only enforcement tests for both session.db (`DbReader`) and main.db (`SessionIndex`) query paths: INSERT, CREATE TABLE, DROP TABLE, and semicolon injection all verified to fail at the SQLite level

### Changed
- Unified SQL gateway: `query_db` IPC command now supports both session.db and main.db via `db` parameter ("session" or "main"), with bind parameter support via `params` array. Replaced 11 per-query Tauri commands (net_events, get_model_calls, get_traces, get_trace_detail, get_mcp_calls, get_file_events, get_session_history, get_global_stats, get_top_providers, get_top_tools, get_top_mcp_tools) with a single `query_db` gateway
- Frontend queries now run through `db.ts` (unified query layer) instead of individual api.ts wrappers, using parameterized SQL from `sql.ts`
- Removed `ModelCallResponse` Rust wrapper struct (was only needed for the deleted `get_model_calls` command)
- Justfile streamlined from 23 recipes to 13 public + 5 internal helpers: `run` now auto-repacks initrd (replaces separate `repack`), `test` includes cross-compile + frontend check (replaces `check`), `full-test` combines capsem-doctor + integration test + bench (replaces `smoke-test`/`integration-test`/`preflight`), `build-assets` replaces `build`, `inspect-session` replaces `check-session`, `release` now produces a DMG at `target/release/Capsem.dmg`
- Removed recipes: `compile`, `sign`, `frontend`, `rebuild`, `repack`, `repack-initrd`, `ensure-tools`, `smoke-test`, `integration-test`, `preflight` (functionality preserved as internal `_`-prefixed helpers or merged into public recipes)

### Fixed
- 12 compilation warnings eliminated across 3 files: dead code warnings in `capsem-fs-watch` cross-platform helpers (blanket `#![cfg_attr(not(target_os = "linux"), allow(dead_code))]`), unused `SessionStats` import in commands.rs, and test-only `close()` method gated with `#[cfg(test)]`
- Test fixture updated from integration test session with full pipeline coverage: denied net events, deleted file events, positive cost estimates, `origin` column on tool_calls
- `fixture_top_domains_non_empty` test assertion fixed: `count >= allowed + denied` accounts for error events that are counted in total but not in allowed/denied buckets
- `query_raw_real_type` test now validates REAL type serialization without requiring positive cost values in the fixture
- Integration test now exercises denied net events (curl to blocked domain), deleted file events (create + rm), cost estimation assertions, and tool origin verification (34 checks, up from 28)

### Added
- Session DB lifecycle management: sessions now progress through running -> stopped -> vacuumed -> terminated states. After a session stops, its DB is checkpointed, vacuumed, and gzip-compressed (`session.db.gz`), then WAL/SHM files are removed. Terminated sessions retain their main.db audit trail record even after disk artifacts are deleted.
- `vm.terminated_retention_days` setting (default 365): controls how long terminated session records are kept in main.db before permanent purging
- Periodic main.db WAL checkpoint every 5 minutes to prevent unbounded WAL growth
- DbWriter now checkpoints WAL on clean shutdown (drop)
- Startup vacuum recovery: any sessions that stopped but were not vacuumed (e.g. due to crash) are automatically compressed on next app launch
- `check-session` script now handles compressed session DBs (auto-decompresses `.gz` files)
- End-to-end integration test (`just integration-test`): boots a real VM, exercises all 6 telemetry pipelines (fs_events, net_events, mcp_calls, model_calls, tool_calls, main.db rollup), runs capsem-doctor MCP tests, asks Gemini to write a poem, and verifies every event type is correctly logged in the session DB
- Release preflight gates (`just preflight`): unit tests, cross-compile, capsem-doctor smoke test, integration test, and benchmarks must all pass before `just release` or `just install` builds the app
- In-VM benchmark recipe (`just bench`): standalone entry point for capsem-bench (disk I/O, rootfs read, CLI startup, HTTP latency)
- Tool origin tracking: `tool_calls` table now records `origin` ("native" or "mcp") and `mcp_call_id` columns to distinguish model built-in tools from MCP gateway tools
- `check-session` data quality warnings: flags model_calls with NULL model, tokens, or request_body_preview
- `check-session` tool lifecycle section: shows origin breakdown and MCP call correlation
- Diagnostic logging when streaming model_calls complete with NULL model, tokens, or preview fields

### Fixed
- Session backfill now looks for `session.db` instead of the old `info.db` filename
- MITM proxy AI telemetry: model name, token counts, and request body preview were NULL for all model_calls when `log_bodies` was disabled. The proxy now always captures up to 64KB of AI provider request/response bodies for metadata parsing regardless of the `log_bodies` setting.
- MITM proxy model resolution: added fallback chain (request body -> SSE stream -> response JSON -> URL path) so model name is extracted even for providers that put it in the URL (e.g. Gemini `/v1beta/models/gemini-2.5-flash:generateContent`)
- MITM proxy stream detection: streaming flag now detected from URL path (`streamGenerateContent` vs `generateContent`) instead of unreliable request body parsing
- MITM proxy non-streaming usage: token counts now parsed from JSON response body when SSE stream parsing yields no usage metadata
- MITM proxy tool origin: tool_calls now use `tool_origin()` for correct "native" vs "mcp" classification instead of hardcoding "native"
- MITM proxy tool responses: tool_result entries from AI request bodies are now correctly extracted (previously always empty when body capture was disabled)
- MITM proxy non-streaming response parsing now handles gzip-compressed response bodies (upstream often sends Content-Encoding: gzip)
- MITM proxy no longer creates model_call records for HEAD requests (connectivity probes from AI CLIs have no body/model/tokens)
- Telemetry event pipeline silently dropping events under burst load: `try_write()` in MITM proxy and fs-watch handler failed without logging when the 256-slot DB channel was full (e.g. during `npm install`). Replaced with async `write().await` via `tokio::spawn` for backpressure, and bumped channel capacity from 256 to 4096.
- MCP builtin tools (`fetch_http`, `grep_http`, `http_headers`) returning empty responses: `capsem-mcp-server` used `SHUT_RDWR` after stdin closed, killing in-flight gateway responses before they could be read back. Changed to `SHUT_WR` (half-close) so the reader thread collects all responses before shutdown.
- MCP `fetch_http` and `grep_http` now reject binary content (images, PDFs, audio, video, etc.) with a clear error instead of returning garbled text or UTF-8 decode errors
- MCP tools now reject non-HTTP schemes (`file://`, `ftp://`, `data:`, etc.) before any network request is made
- MCP `grep_http` now rejects empty patterns instead of matching every line

### Changed
- Settings registry migrated from hardcoded Rust to `config/defaults.toml` (TOML-based, embedded at compile time). Setting definitions use `String` fields instead of `&'static str`. No user-facing behavior change.
- Session culling now marks sessions as "terminated" instead of deleting main.db rows, preserving the audit trail. Old terminated records are purged after `vm.terminated_retention_days` (default 365 days).
- Schema migrated from v2 to v3 (additive: new `compressed_size_bytes` and `vacuumed_at` columns on sessions table)
- MCP built-in tools exposed without `builtin__` prefix: models now see `fetch_http`, `grep_http`, `http_headers` instead of `builtin__fetch_http` etc. -- cleaner tool names for AI agents
- MCP built-in tool descriptions rewritten with full documentation: HTML extraction behavior, output format, pagination, domain policy enforcement, and error conditions
- Per-session analytics (Traffic, AI Models, MCP views) now use `queryDb(sql)` with SQL constants instead of dedicated Tauri commands -- reduces Rust boilerplate and gives the frontend more flexibility
- Network store rewritten: individual SQL queries replace monolithic `getSessionStats()` call, adding SQL-driven avg latency, method distribution, and process distribution
- Dashboard session detail no longer shows file event count (global dashboard should only show global data)
- Rootfs switched from 2GB ext4 to 382MB squashfs (zstd, 64K blocks) -- 81% smaller for DMG distribution
- Boot sequence uses overlayfs (immutable squashfs lower + ephemeral tmpfs upper) -- writes to system paths silently go to tmpfs
- Test fixture (`data/fixtures/test.db`) is now captured from real sessions instead of generated by a Python script
- `just update-fixture <path>` replaces `just gen-test-db`: copies a real session DB, scrubs API keys, and syncs to `frontend/public/fixtures/`

### Removed
- Dead AI gateway server (`gateway/server.rs`, 997 lines): axum HTTP server on vsock:5004 was never wired up in main.rs. All AI traffic goes through the MITM proxy on vsock:5002. `extract_model_from_path`, `parse_non_streaming_usage`, and `tool_origin` helpers moved to `gateway/provider.rs` and `gateway/events.rs` where the MITM proxy can use them.
- `VSOCK_PORT_AI_GATEWAY` constant (port 5004) -- unused, never wired up
- `GatewayConfig` struct -- only used by the dead server
- `gateway_integration.rs` test file -- tests for the dead server
- `axum` dependency from capsem-core
- `get_session_stats`, `get_mcp_stats`, `get_file_stats` Tauri IPC commands -- replaced by frontend SQL via `queryDb()`
- `SessionStatsResponse` struct from commands.rs and `SessionStatsResponse`, `SessionStats`, `McpCallStats`, `FileEventStats` types from frontend
- `SessionsSection.svelte` -- orphan component never imported by AnalyticsView
- `data/fixtures/generate_test_db.py` -- synthetic data generator replaced by real session captures

### Added
- `sql.ts`: centralized SQL query constants for all per-session analytics (13 queries covering net stats, domains, time buckets, provider usage, tool usage, model stats, MCP stats, file stats, latency, method/process distribution)
- `queryOne<T>()` and `queryAll<T>()` typed helpers in `api.ts` for running SQL against the active session's info.db
- Analytics data architecture documented in `docs/architecture.md` (two-database design, data flow, query strategy, polling patterns)
- Frontend development skill file (`.claude/skills/frontend.md`)
- In-VM filesystem watcher (`capsem-fs-watch`): inotify-based daemon streams file create/modify/delete events to the host over vsock:5005 for real-time file activity telemetry
- `fs_events` audit table in `capsem-logger`: records every file operation with timestamp, action, path, and size
- `FileEvent` type with `WriteOp::FileEvent` variant and reader queries (`recent_file_events`, `search_file_events`, `file_event_stats`)
- `get_file_events` and `get_file_stats` Tauri IPC commands for the frontend
- Files view in frontend: summary cards (total/created/modified/deleted), searchable event table with action badges, 2s polling
- Files sidebar navigation item with document icon between Sessions and MCP Tools
- Mock file event data (13 entries) for browser dev mode
- MCP gateway wired to vsock:5003: host now accepts MCP connections from guest agents, fixing Gemini CLI hang on startup
- Built-in HTTP tools: `fetch_http`, `grep_http`, `http_headers` -- AI agents can fetch web content, search pages, and inspect headers from within the sandbox, all checked against domain policy
- MCP domain policy hot-reload: changing network settings in the UI immediately updates which domains built-in HTTP tools can access
- `capsem-doctor` MCP tests: 6 new in-VM diagnostic tests verifying MCP binary, initialize handshake, tools/list, allowed/blocked fetch, and fastmcp availability
- `fastmcp` Python package in guest rootfs for building custom MCP servers inside the VM
- MCP Proxy Gateway: AI agents in the guest VM can now use host-side MCP tools transparently via a unified `capsem-mcp-server` binary injected at boot
- `capsem-mcp-server` guest binary: lightweight NDJSON-over-vsock bridge (~90 lines) relaying MCP JSON-RPC between agents and the host gateway on vsock:5003
- MCP gateway host module (`capsem-core::mcp`): types, policy engine, stdio bridge, server manager, and vsock gateway for routing tool calls to host-side MCP servers
- Namespaced MCP tools: tools from multiple servers are exposed as `{server}__{tool}` to prevent collisions (e.g., `github__search_repos`, `slack__send_message`)
- Per-tool dynamic policy: each MCP tool can be set to allow (forward normally), warn (forward + flag), or block (return JSON-RPC error) with hot-reload via `Arc<RwLock<Arc<McpPolicy>>>`
- MCP server auto-detection: reads existing MCP configs from `~/.claude/settings.json` and `~/.gemini/settings.json` at boot
- `mcp_calls` audit table in `capsem-logger`: full telemetry for every MCP tool call (server, method, tool, decision, duration, error)
- `McpCall` event type with `WriteOp::McpCall` variant and `insert_mcp_call()` writer method
- `DbReader` MCP queries: `recent_mcp_calls(limit, search)` with text search across server/method/tool, `mcp_call_stats()` aggregation (total, allowed, denied, warned, by-server breakdown)
- Schema migration: existing databases automatically gain the `mcp_calls` table on open
- `get_mcp_calls` and `get_mcp_stats` Tauri IPC commands for the frontend
- `inject_capsem_mcp_server()`: automatically merges `{"capsem": {"command": "/run/capsem-mcp-server"}}` into Claude and Gemini settings.json at boot, preserving user-provided MCP server entries
- MCP Tools view in frontend: summary cards (total/warned/denied), per-server breakdown, searchable call log table with decision badges
- MCP sidebar navigation item with layers icon between Sessions and Settings
- Mock MCP data: 6 sample calls across 3 servers (github, filesystem, slack) for browser dev mode
- Generic usage details tracking: token breakdowns (cache_read, thinking) stored as extensible `usage_details` JSON map instead of individual columns -- zero schema changes when adding new token types
- OpenAI Responses API (`/v1/responses`) streaming support: parses `response.created`, `response.output_text.delta`, `response.reasoning_summary_text.delta`, `response.function_call_arguments.delta`, `response.output_item.added/done`, and `response.completed` SSE events
- OpenAI cached token parsing from `prompt_tokens_details.cached_tokens` and reasoning token parsing from `completion_tokens_details.reasoning_tokens`
- Gemini thinking token parsing from `thoughtsTokenCount` (was parsed but unused)
- Non-streaming response parsing: gateway now extracts model, input/output tokens, and usage details from non-streaming JSON responses (all three providers), enabling cost estimation and token tracking for non-streamed API calls
- Cache and thinking token counts shown in session stats and trace detail UI

### Changed
- `capsem-proto` simplified: removed `McpGuestMsg`/`McpHostMsg` enums and encode/decode functions in favor of raw NDJSON passthrough (less code, better performance)
- `capsem-init` deploys `capsem-mcp-server` from initrd (with rootfs fallback)
- `just repack` cross-compiles and bundles `capsem-mcp-server` alongside pty-agent and net-proxy
- Sessions view: trace detail panel now shows MCP tool calls inline with model calls
- Token details stored as flexible `usage_details TEXT` JSON column replacing individual token columns -- single schema handles all current and future token breakdowns
- Cost estimation accounts for cached tokens: `cache_read` tokens subtracted from effective input before pricing calculation
- Pricing function signature simplified: accepts `&BTreeMap<String, u64>` usage details map instead of individual token parameters

### Fixed
- MCP gateway no longer sends a JSON-RPC response for `notifications/initialized` (it's a notification, not a request) -- fixes protocol confusion in some MCP clients
- Token metrics double-counted in trace detail view when a model call had both request and response tool entries -- now only the first row per call shows metrics
- Non-streaming API responses (no `stream: true`) recorded with null tokens and $0.00 cost -- now properly parsed for all providers
- HEAD connectivity checks from AI CLIs (Claude, Gemini) no longer create empty model_call rows -- filtered at the gateway level

## [0.8.0] - 2026-02-28

### Added
- `capsem-logger` crate: unified audit database with dedicated writer thread, replacing three separate SQLite databases (`WebDb`, `GatewayDb`, `AiDb`) with a single `session.db` per VM session
- Dedicated writer thread using `tokio::sync::mpsc` channel with block-then-drain batching (up to 128 ops per transaction), eliminating `spawn_blocking` + `Arc<Mutex<>>` contention
- `DbWriter` / `DbReader` API: async writes via channel, read-only WAL concurrent readers, typed `WriteOp` enum for debuggable operations
- Unified schema: `net_events` (all HTTPS connections), `model_calls` (denormalized request+response), `tool_calls`, `tool_responses` tables in a single DB file
- Inline SSE event parsing in the MITM proxy for AI provider traffic (Anthropic, OpenAI, Google Gemini)
- Provider-agnostic LLM event types (`LlmEvent`, `StreamSummary`) with `collect_summary()` for structured audit logging
- Hand-rolled SSE wire-format parser with chunk-boundary-safe state machine (no crate dependency)
- Provider-specific SSE stream parsers: Anthropic (interleaved content blocks, thinking), OpenAI Chat Completions (tool calls, content filter), Google Gemini (complete events, synthetic call IDs)
- Request body parser extracting model, stream flag, system prompt preview, message/tool counts, and tool_result entries for tool call lifecycle linking
- `AiResponseBody`: hyper Body wrapper that does SSE parsing inline during `poll_frame` with zero added latency
- AI provider domain detection (`api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`) in the MITM proxy
- API key suffix extraction (last 4 chars, Stripe-style) from `x-api-key` and `Authorization: Bearer` headers
- Per-call cost tracking: gateway estimates USD cost using bundled model pricing data from pydantic/genai-prices
- Fuzzy model name matching for pricing: unknown model variants (date-stamped, custom-suffixed) now resolve to the correct pricing via progressive suffix stripping and longest-prefix fallback instead of silently returning $0.00
- Trace ID assignment in MITM proxy: multi-turn tool-use conversations are linked by shared trace IDs, enabling the Sessions view to render conversation spans
- SQL-driven session statistics: counts, token usage, cost, domain distribution, and time-bucketed charts all computed via SQLite queries
- New Tauri IPC commands: `get_session_stats` (full aggregate dashboard data), `get_model_calls` (model call history with search)
- LLM Usage section in Sessions view: API call count, input/output tokens, estimated cost, per-provider breakdown, model calls table, tool usage badges
- SQL-powered search in Network view: debounced search queries hit SQLite LIKE instead of client-side filtering
- `just update_prices` recipe to refresh bundled model pricing data
- `capsem-bench` in-VM performance benchmark tool: disk I/O (sequential read/write, random 4K IOPS) and HTTP throughput (ab-style concurrent requests with latency percentiles)
- `capsem-bench rootfs` benchmark: sequential and random 4K read performance on the read-only rootfs
- `capsem-bench startup` benchmark: cold-start latency for python3, node, claude, gemini, and codex CLIs (3 runs, min/mean/max)
- Rich table formatting for all capsem-bench output (replaces manual text formatting)
- Configurable VM CPU cores via `vm.cpu_count` setting (1-8, default 4)
- Configurable VM RAM via `vm.ram_gb` setting (1-16 GB, default 4 GB)
- 1 GB swap file on scratch disk for better memory pressure handling
- Search category in settings: Google Search (on by default), Perplexity, and Firecrawl toggles with domain-level policy
- Custom allow/block domain lists (`network.custom_allow`, `network.custom_block`) for user-defined domain rules
- Active Policy debug panel in Network view: collapsible section showing allowed/blocked domain lists, default action, corp managed status, and policy conflicts
- Policy conflict detection: domains appearing in both allow and block lists are flagged in the Network view

### Changed
- Terminal UI overhaul: borderless look with 10px padding, thin styled scrollbar, theme-matching background (full black in dark mode)
- Removed bottom status bar; session stats (tokens, tools, cost, VM status) now displayed inline below the terminal
- Sidebar reorganized: Console + Sessions in nav, Settings/theme/collapse in footer
- Network view moved into Settings as a collapsible "Network Statistics" section
- Sessions panel (charts, spans, analytics) now accessible from sidebar nav
- Session Statistics section added to bottom of Settings view
- MITM proxy and gateway server use `DbWriter` channel instead of `spawn_blocking` + `Arc<Mutex<>>` for all database writes
- Session telemetry stored in `session.db` (was `info.db`)
- VM Disk Performance Overhaul: 2M+ IOPS for random 4K reads (~8 GB/s) and ~20x speedup in random write throughput
- Network Proxy Overhaul: replaced synchronous thread-per-connection guest proxy with Tokio-based async implementation
- Structural Latency Elimination: `TCP_NODELAY` on both guest and host proxies, reducing proxy overhead to the physical network floor (~40ms median RTT)
- VM CPU default increased from 2 to 4 cores
- VM RAM default increased from 512 MB to 4 GB
- Scratch disk default increased from 8 GB to 16 GB
- Node.js V8 heap cap raised from 512 MB to 2 GB to match higher RAM
- Network store is now SQL-driven: counts and charts read from `get_session_stats` instead of counting JS arrays
- Session info response expanded with LLM metrics (model call count, tokens, tool calls, estimated cost)
- `net_events` command accepts optional `search` parameter for SQL-backed filtering
- `get_session_info` is now async with `spawn_blocking` for proper non-blocking DB access
- Rootfs disk caching mode changed from `Automatic` to `Cached` for aggressive host page cache retention on the read-only disk
- Host-side disk settings: enabled host-level caching (`VZDiskImageCachingMode::Cached`) and disabled synchronization barriers (`VZDiskImageSynchronizationMode::None`)
- Guest-side kernel tuning: `capsem-init` now sets I/O scheduler to `none`, `read_ahead_kb` to 4096, and `nr_requests` to 256 for all VirtIO devices
- Filesystem optimizations: `noatime,nodiratime,noload` mount options for rootfs and scratch disks
- Scratch disk format optimization: `mke2fs -m 0` to reclaim reserved root blocks
- `elie.net` moved from a Package Registry toggle to the default custom allowed domains list
- `network.log_bodies` and `network.max_body_capture` moved from Network to VM category
- Session settings (`session.retention_days`, `session.max_sessions`, `session.max_disk_gb`) moved from Session to VM category
- Mock data now mirrors the full backend settings registry (~35 settings across 7 categories)
- Settings view categories displayed in fixed order: AI Providers, Search, Package Registries, Network, Guest Environment, Appearance, VM
- Settings view categories collapsed by default (click to expand)
- Network view: allowed/blocked domain lists are now separate collapsible groups within Active Policy

### Fixed
- VM status indicator now shows correct color (blue for running, yellow for booting) instead of defaulting to no color due to state casing mismatch between Rust and frontend
- MITM proxy now assigns trace IDs and estimates costs for AI model calls, enabling Sessions view to display LLM statistics
- Fixture-dependent test assertions in capsem-logger replaced with data-agnostic checks to prevent breakage on fixture regeneration
- Benign "error shutting down connection" warnings in the host proxy logs are now filtered

### Removed
- Dead `gateway/audit.rs` module (839 lines, never compiled) superseded by capsem-logger
- `GatewayDb` (redundant flat table, replaced by `model_calls` in unified schema)
- `AiDb` (normalized 4-table schema, merged into `capsem-logger`)
- `WebDb` (replaced by `net_events` table in unified schema)
- `StreamAccumulator` (unused since `AiResponseBody` replaced it)
- `registry.elie.allow` setting (replaced by `network.custom_allow` default)
- `registry.debian.allow` setting (rootfs is read-only, packages cannot be installed at runtime)
- `domainlist` setting type from frontend (custom allow/block use standard `text` type with ID-based chip rendering)

### Security
- Terminal input batching thread now caps coalesced buffer at 64 KB, preventing unbounded memory growth if the IPC channel is flooded faster than the inner try_recv loop can drain
- Sanitize HTTP headers in telemetry logs: allowlisted headers (content-type, host, server, etc.) stored verbatim; all others (authorization, x-api-key, cookies) have values replaced with BLAKE3 hash prefix (`hash:<12-char-hex>`) to prevent credential leakage while preserving header presence and enabling correlation

## [0.7.0] - 2026-02-26

### Changed
- Terminal output uses poll-based binary IPC (`terminal_poll`) instead of JSON event emission, eliminating ~4x serialization overhead
- Terminal input batched with 5ms window (up to 4KB) to reduce IPC round-trips per keystroke
- Vsock read buffer increased from 8KB to 64KB and mpsc channel from 256 to 8192 entries
- CoalesceBuffer defaults changed from 10ms/64KB to 5ms/10MB for higher throughput
- Terminal output queue with 64-entry backpressure cap prevents OOM when frontend stops polling

## [0.6.0] - 2026-02-26

### Added
- Guest dev environment: `pip install`, `uv pip install`, `npm install -g` all work out of the box on the read-only rootfs
- Python venv auto-activated at boot with `--system-site-packages` (packages install to `/root/.venv`)
- `pip` and `python` aliased to `uv pip` and `uv run python` (faster, no root warning)
- AI CLIs (claude, gemini, codex) installed to writable scratch disk at boot so auto-update works
- npm global prefix redirected to writable `/root/.npm-global` for `npm install -g`
- Pre-installed Python packages declared in `images/requirements.txt`: numpy, requests, httpx, pandas, scipy, scikit-learn, matplotlib, pillow, pyyaml, beautifulsoup4, lxml, tqdm, rich
- Pre-installed npm globals declared in `images/npm-globals.txt` (AI CLIs)
- Login banner shows AI tool status: ready (blue), no API key (purple), disabled by policy (purple)
- Host injects `CAPSEM_ANTHROPIC_ALLOWED`, `CAPSEM_OPENAI_ALLOWED`, `CAPSEM_GOOGLE_ALLOWED` env vars at boot
- Configurable login banner (`images/banner.txt`) and random developer tips (`images/tips.txt`)
- Removed PEP 668 EXTERNALLY-MANAGED marker from rootfs
- `just build` upgrades all tools to latest: apt packages, pip, npm, node, nvm, uv
- Claude Code yolo mode: `~/.claude/settings.json` with `bypassPermissions` + `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and `~/.claude.json` state file to skip onboarding, trust dialogs, and keybinding prompts
- Gemini CLI yolo mode: `~/.gemini/settings.json` with `approvalMode: "yolo"`, telemetry/auto-updates disabled, folder trust disabled, and Gemini's own sandbox disabled (capsem provides the sandbox)
- Metadata-driven env var injection: settings declare `env_vars` in metadata instead of hardcoded mappings
- Built-in guest environment settings (`guest.shell.term`, `guest.shell.home`, `guest.shell.path`, `guest.shell.lang`, `guest.tls.ca_bundle`) configurable via user.toml and corp.toml
- Individual vsock boot messages (`SetEnv`, `FileWrite`, `BootConfigDone`) replacing single `BootConfig` frame, eliminating the 8KB frame size limit for boot configuration
- Guest boot log at `/var/log/capsem-boot.log` recording clock sync, env vars, file writes, and handshake status
- Per-service domain settings (`ai.*.domains`) with user-editable comma-separated domain patterns
- AI provider API key injection into guest VM environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`)
- Google AI (`ai.google.allow`) enabled by default for out-of-the-box Gemini CLI support
- Per-session unique IDs (`YYYYMMDD-HHMMSS-XXXX`) replacing hardcoded "default"/"cli" VM IDs
- Session index database (`~/.capsem/sessions/main.db`) tracking metadata across sessions
- `get_session_info` and `get_session_history` Tauri IPC commands for the Sessions view
- Session retention settings: `session.retention_days`, `session.max_sessions`, `session.max_disk_gb`
- Age-based, count-based, and disk-based session culling at startup
- Migration from legacy `session.json` files to `main.db` on startup
- Request count snapshotting (`count_by_decision`) when sessions stop
- Svelte 5 + Tailwind v4 + DaisyUI v5 frontend framework replacing vanilla JS
- Single Svelte island architecture: `<App client:only="svelte" />` in Astro shell
- Sidebar navigation with collapsible icon rail (Console, Sessions, Network, Settings)
- Network events view with filterable table, expandable rows showing headers/body
- Settings view with categorized editor, type-aware inputs, corp lock indicators
- Sessions view with VM state timeline from state machine history
- Terminal view wrapping existing xterm.js web component with Tauri event wiring
- Status bar showing VM state indicator, HTTPS call count, allowed/denied stats
- Light/dark theme toggle with localStorage persistence and system preference fallback
- Svelte 5 rune stores for VM state, network events, settings, theme, and sidebar
- TypeScript IPC layer (`types.ts` + `api.ts`) with typed wrappers for all Tauri commands
- `svelte-check` added to `just check` and `pnpm run check` pipelines
- Generic typed settings system replacing TOML-based policy config -- each setting has ID, type, category, default, metadata, and optional `enabled_by` parent toggle
- Per-setting corp override: corporate settings (`/etc/capsem/corp.toml`) lock individual settings, not entire sections
- Setting metadata with domain patterns, HTTP method permissions, numeric bounds, and text choices
- `get_settings` and `update_setting` Tauri IPC commands for the settings UI
- Settings architecture documentation in `docs/architecture.md`
- Policy override security documentation in `docs/security.md`

### Changed
- Increased vsock MAX_FRAME_SIZE from 8KB to 256KB for generous boot payloads
- Boot handshake protocol now sends env vars and files as individual messages instead of a single `BootConfig` payload
- Sessions view redesigned: current session info cards, network analytics, session history table (replaced CPU/memory/binary stats that VZ doesn't expose)
- Per-session telemetry renamed from `web.db` to `info.db` (legacy `web.db` still read for backward compatibility)
- Each VM boot creates a fresh telemetry database, eliminating stale request carryover between sessions
- Network policy replaced with simplified rule-based system: per-domain read/write verb control with defaults (GET allowed, POST denied)
- Configuration format changed from section-based TOML (`[network]`, `[guest]`, `[vm]`) to flat settings map (`[settings]` with dotted keys like `"registry.github.allow"`)
- Domain allow/block lists now derived from setting toggles and their metadata (e.g., toggling `registry.github.allow` controls `github.com`, `*.github.com`, `*.githubusercontent.com`)
- AI provider domains moved from explicit block-list to disabled-by-default toggles with domain metadata
- Guest environment variables stored as `guest.env.*` settings instead of `[guest].env` table
- VM settings (scratch disk size) stored as `vm.scratch_disk_size_gb` setting instead of `[vm]` section
- Removed SNI-based pre-TLS policy check; all policy enforcement at HTTP level
- Removed generativelanguage.googleapis.com from block-list (Gemini API testing)
- MITM proxy streams request and response bodies instead of buffering in memory
- Upstream TLS config cached per-VM instead of recreated per-request
- Default `log_bodies` changed from false to true

### Fixed
- Denied domains now record HTTP method, path, and status in telemetry (TLS handshake completes, denial at HTTP 403 level)
- Guest receives proper HTTP 403 response with reason for denied requests instead of cryptic TLS connection error
- "Invalid Date" in Session/Network views: timestamps now serialize as epoch seconds instead of SystemTime objects
- Legacy "default"/"cli" sessions migrated as "crashed" instead of carrying over stale "running" status
- web.db now records query string, matched rule, and 403 status for denied requests
- Upstream connection failures record error reason in telemetry

### Removed
- `get_vm_stats` command and `VmStats`/`BinaryCall` types (VZ framework doesn't expose guest metrics)
- Hardcoded `DEFAULT_VM_ID` constant -- replaced by dynamic session IDs
- `session.json` files -- replaced by `main.db` session index (migrated automatically)
- SNI parser module (`sni_parser.rs`) -- domain extracted from TLS handshake instead

### Security
- Env var sanitization: reject keys containing `=` or NUL bytes, values containing NUL (prevents agent crash / kernel panic)
- Blocked env var list: LD_PRELOAD, LD_LIBRARY_PATH, IFS, BASH_ENV, and other dangerous variables rejected during boot
- Boot allocation caps: max 128 env vars, 64 files, 10MB total file data
- FileWrite path traversal protection: reject paths containing `..`
- Defense-in-depth: guest agent validates env vars and file paths independently of host
- Body size limit (100MB) prevents OOM from malicious guest payloads
- Replaced unsafe borrow_fd with safe fd cloning
- Corp-locked settings cannot be modified by user, enforced at the merge level

## [0.5.0] - 2026-02-25

### Added
- Ephemeral scratch disk for `/root` workspace (8GB default, configurable via `[vm].scratch_disk_size_gb` in `~/.capsem/user.toml`)
- Per-session directory structure (`~/.capsem/sessions/<vm_id>/`) with session metadata (`session.json`)
- Stale session cleanup on startup: leftover scratch images deleted, orphaned "running" sessions marked as "crashed"
- Block device identifiers (`rootfs`, `scratch`) for stable device naming in the guest (`/dev/disk/by-id/virtio-*`)
- uv fast Python package installer available to guest AI agents

### Changed
- Guest `/root` workspace now uses ext4 on a virtio block device instead of RAM-backed tmpfs, increasing usable space from ~512MB to 8GB+
- Upgraded Node.js from Debian's v18 to v24 LTS via nvm
- Replaced pip3 with uv for in-VM Python package management (certifi, pytest)

### Fixed
- gemini CLI crashing with `SyntaxError: Invalid regular expression flags` due to Node.js 18 lacking the 'v' regex flag
- AI CLI smoke test was too lenient -- now verifies `--help` runs without JS runtime errors instead of only checking for signal crashes

## [0.4.0] - 2026-02-25

### Added
- Host-side state machine (`HostState`) with validated transitions, timing history, and structured perf logging
- Per-state message validation: host validates both outbound and inbound vsock control messages against lifecycle stage
- New Tauri IPC commands for Svelte UI: `get_guest_config`, `get_network_policy`, `set_guest_env`, `remove_guest_env`, `get_vm_state`
- Structured `vm-state-changed` events with JSON payloads (state + trigger) instead of plain strings
- Protocol documentation (`docs/protocol.md`): wire format, message reference, state machine diagrams, boot handshake, security invariants
- Zero-trust guest binary security rule documented in `docs/security.md`
- `write_policy_file()` for TOML serialization of user.toml changes from the UI
- MITM transparent proxy: full HTTP inspection (method, path, status code, headers, body preview) for all HTTPS traffic from the guest VM
- Static Capsem MITM CA certificate (ECDSA P-256, 100-year validity) baked into the guest rootfs trust store
- On-demand domain certificate minting with RwLock cache for TLS termination
- HTTP-level policy engine: method+path rules on top of domain allow/block lists (`[[network.rules]]` in user.toml)
- Extended telemetry: `web.db` now records HTTP method, path, status code, request/response headers, and body previews
- CA trust environment variables (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`) injected via BootConfig
- certifi CA bundle patching in rootfs for Python SDK compatibility (requests, openai, anthropic)
- Schema migration for existing `web.db` databases (adds new columns without data loss)
- Clock synchronization -- guest VM clock is set from host at boot time (fixes TLS cert validation, git, curl)
- Environment variable injection via vsock boot config (`BootConfig`/`BootReady` handshake)
- `[guest]` section in `user.toml` for custom guest environment variables
- `--env KEY=VALUE` CLI flag for one-off env injection (`capsem --env FOO=bar echo $FOO`)
- `capsem-proto` crate -- shared protocol types for host/guest communication
- Clock sync diagnostic test in `capsem-doctor`
- In-VM diagnostic test suite expanded: MITM CA trust chain tests (system store, certifi, curl without -k, Python urllib), network edge cases (HTTP port 80, non-443 ports, direct IP, AI provider blocking, multi-domain DNS), process integrity (pty-agent, dnsmasq, no systemd/sshd/cron), deeper kernel hardening (no modules loaded, no debugfs, no IPv6, no swap, no kallsyms, ro cmdline), environment validation (TERM, HOME, PATH, arch, kernel version, mount points), and 14 additional unix utility checks
- `just test` recipe runs workspace tests with coverage summary via `cargo-llvm-cov`
- `just ensure-tools` auto-installs `cargo-llvm-cov` and `llvm-tools-preview` on fresh clones
- Air-gapped networking: `curl https://elie.net` now works from inside the guest VM
- Host-side SNI proxy inspects TLS ClientHello, enforces domain allow-list, and bridges to the real internet
- Domain policy engine with allow-list, block-list, and wildcard pattern matching (`*.github.com`)
- Configurable domain policy via `~/.capsem/user.toml` and `/etc/capsem/corp.toml` (corp overrides user)
- Per-session `web.db` (SQLite) recording every HTTPS connection attempt for auditing
- Guest-side `capsem-net-proxy` binary: TCP-to-vsock relay for transparent HTTPS proxying
- Default developer allow-list: GitHub, npm, PyPI, crates.io, Debian repos, elie.net
- AI provider domain blocking at SNI level (api.anthropic.com, api.openai.com, googleapis.com)
- `net_events` Tauri command for querying recent network events from the frontend
- Per-VM network isolation: each VM gets its own policy, web.db, and connection handlers

### Changed
- SNI proxy replaced by MITM transparent proxy for full HTTP-level traffic inspection and policy enforcement
- Domain policy (`DomainPolicy`) wrapped by `HttpPolicy` which adds method+path rules while preserving backward compatibility
- `load_merged_policy()` now returns `HttpPolicy` instead of `DomainPolicy`
- HTTPS proxy connections spawn as async tokio tasks instead of blocking threads
- Control protocol split into disjoint `HostToGuest`/`GuestToHost` enums with reserved variants for file operations and lifecycle management
- Guest agent boot sequence restructured: vsock connects first, receives clock + env from host before forking bash
- Max control frame size bumped from 4KB to 8KB to accommodate env var payloads
- `just build`, `just repack`, and `just check` now run tests with coverage as a gate before proceeding
- Kernel now includes IP stack + netfilter (CONFIG_INET=y, iptables REDIRECT) for air-gapped networking
- Rootfs includes iproute2, iptables, and dnsmasq for guest network setup
- capsem-init sets up dummy0 NIC, fake DNS, and iptables rules at boot
- `just repack` now includes `capsem-net-proxy` alongside `capsem-pty-agent`
- Refactored VM smoke test into pytest-based diagnostic suite (`capsem-doctor`)
- Split tests into focused modules: sandbox security, utilities, runtimes, AI CLIs, workflows
- Added sandbox security tests (rootfs read-only, no kernel modules, no /dev/mem, network isolation, no setuid/setgid)
- Added Python and Node.js execution tests (actual code runs, not just version checks)
- Added AI CLI sandbox verification (binaries execute without crashing)
- Network sandbox tests updated: verify air-gapped proxy (allowed/denied domains) instead of raw network block

### Fixed
- MITM proxy TLS handshake failure: rustls crypto provider was not initialized, causing silent panics on every proxy connection
- MITM proxy now uses explicit `builder_with_provider()` instead of relying on global crypto state, eliminating the class of bug entirely
- `just build` failure: Dockerfile.rootfs could not find CA cert (build context was `images/`, cert was in `config/`)
- `just build` failure: certifi not installed when CA bundle patching step runs
- Kernel `CONFIG_KALLSYMS=n` was silently ignored because the option requires `CONFIG_EXPERT=y` to be configurable
- Kernel cmdline now includes `ro` for read-only rootfs mount
- `just smoke-test` now returns non-zero exit code on test failures
- In-VM diagnostic test fixes: `/proc/modules` absent is valid (CONFIG_MODULES=n), bash test checks availability not current shell, CA bundle tests grep base64 instead of DER-encoded CN, Python TLS test verifies handshake not HTTP status

### Deprecated
- `sni_proxy::handle_connection` -- use `mitm_proxy::handle_connection` for full HTTP inspection

### Security
- `CONFIG_EXPERT=y` in kernel defconfig ensures all hardening options (KALLSYMS=n, MODULES=n, etc.) are respected by `make olddefconfig`
- Kernel symbol table (`/proc/kallsyms`) now empty -- eliminates kernel ASLR bypass vector
- MITM proxy enables full HTTP audit trail: every request method, path, status code, and headers are logged to web.db
- HTTP-level policy rules allow fine-grained control (e.g., allow GET but deny POST to specific paths)
- Default-deny domain policy: only explicitly allowed domains are reachable from the guest
- No DNS leaves the VM: all resolution is faked to a local IP
- Corporate policy (`/etc/capsem/corp.toml`) overrides user settings for enterprise lockdown
- Per-VM isolation prevents cross-VM network interference

## [0.3.0] - 2026-02-24

### Added
- PTY-over-vsock terminal communication replacing serial broadcast channel
- Guest PTY agent (`capsem-pty-agent`) for high-throughput terminal I/O with full PTY support
- Terminal resize support (`stty size` reflects window dimensions)
- vsock control channel with MessagePack framing for structured commands (resize, heartbeat)
- Kernel vsock support (`CONFIG_VSOCKETS`, `CONFIG_VIRTIO_VSOCKETS`)
- Multi-VM-ready app state architecture (`vm_id`-keyed `HashMap`)
- Output coalescing (10ms/64KB) to prevent frontend IPC saturation
- Boot-time command execution via vsock (`Exec`/`ExecDone` control messages)
- CLI mode (`capsem "command"`) routes commands through vsock PTY agent with exit code propagation

### Changed
- Terminal input now routes through vsock when connected, falling back to serial
- Guest init script (`capsem-init`) launches PTY agent instead of direct bash/setsid
- CLI mode rewritten from serial I/O to vsock-based execution with proper exit codes
- `just repack` now cross-compiles and bundles the PTY agent into the initrd for fast iteration
- Serial forwarding stops once vsock connects, eliminating duplicate output
- M5 redesigned: zero-trust network boundaries with SNI proxy domain filtering, AI provider domain blocking, and real-time file telemetry via fanotify
- M6 redesigned: active AI audit gateway with 9-stage event lifecycle (PII scrubbing, tool call interception, secret scanning), replaces passive proxy approach
- M7 redesigned: hybrid MCP architecture -- local tools run sandboxed in-VM, remote tools route through host gateway with credential injection
- M8 redesigned: per-session audit databases with zstd-compressed blobs, OverlayFS config write-back, enterprise observability (Prometheus, OTLP, corporate policy via MDM)

### Fixed
- Shell prompt not appearing after command execution (stderr was redirected to /dev/hvc0, sending readline prompt through buffered serial path instead of vsock PTY)

### Security
- Removed serial console fallback: missing PTY agent halts boot instead of opening an unprotected shell
- Replaced scattered `unsafe { File::from_raw_fd }` + `mem::forget` with centralized `borrow_fd` helper using `ManuallyDrop`
- Added T13 threat (AI Traffic Audit Bypass) documenting the enforcement chain: iptables -> vsock bridge -> SNI proxy -> audit gateway
- Updated T3 (Data Exfiltration) with fswatch telemetry, PII engine, and secret scanning mitigations
- Updated T5 (Credential Theft) with gateway key injection and PII scrubbing on model calls
- Updated T11 (Network Exfiltration) with AI domain blocking at SNI proxy and 9-stage lifecycle enforcement
- Added Corporate Security Profile section with MDM-distributable policy.toml for enterprise deployments

## [0.2.0] - 2026-02-24

### Added
- blake3 integrity checking of VM assets (B3SUMS)
- Kernel hardening configuration for guest VM
- Proper terminal signal handling (setsid for controlling tty)
- Boot-up tracing spans for timing diagnostics
- Utility helpers for VM lifecycle

### Fixed
- Utility module fixes

## [0.1.0] - 2026-02-23

### Added
- Native macOS app using Tauri 2.0 with Astro frontend
- Linux VM sandboxing via Apple Virtualization.framework
- Virtio serial console with bidirectional I/O (xterm.js <-> guest /dev/hvc0)
- Custom capsem-init (PID 1) with chroot and setsid
- Docker/Podman-based VM asset build pipeline (kernel, initrd, rootfs)
- `just` task runner workflows (build, repack, dev, run, release, install)
- Codesigning with com.apple.security.virtualization entitlement
- xterm.js terminal web component
- Tauri auto-updater plugin integration

### Changed
- Complete rewrite from Python proxy architecture (v1) to native Rust/Tauri VM app

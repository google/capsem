# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Terminal UI overhaul: borderless look with 10px padding, thin styled scrollbar, theme-matching background (full black in dark mode)
- Removed bottom status bar; session stats (tokens, tools, cost, VM status) now displayed inline below the terminal
- Sidebar reorganized: Console + Sessions in nav, Settings/theme/collapse in footer
- Network view moved into Settings as a collapsible "Network Statistics" section
- Sessions panel (charts, spans, analytics) now accessible from sidebar nav
- Session Statistics section added to bottom of Settings view

### Fixed
- Fixture-dependent test assertions in capsem-logger replaced with data-agnostic checks to prevent breakage on fixture regeneration

### Security
- Sanitize HTTP headers in telemetry logs: allowlisted headers (content-type, host, server, etc.) stored verbatim; all others (authorization, x-api-key, cookies) have values replaced with BLAKE3 hash prefix (`hash:<12-char-hex>`) to prevent credential leakage while preserving header presence and enabling correlation

### Added
- Fuzzy model name matching for pricing: unknown model variants (date-stamped, custom-suffixed) now resolve to the correct pricing via progressive suffix stripping and longest-prefix fallback instead of silently returning $0.00
- Configurable VM CPU cores via `vm.cpu_count` setting (1-8, default 4)
- Configurable VM RAM via `vm.ram_gb` setting (1-16 GB, default 4 GB)
- 1 GB swap file on scratch disk for better memory pressure handling
- Per-call cost tracking: gateway estimates USD cost using bundled model pricing data from pydantic/genai-prices
- SQL-driven session statistics: counts, token usage, cost, domain distribution, and time-bucketed charts all computed via SQLite queries
- New Tauri IPC commands: `get_session_stats` (full aggregate dashboard data), `get_model_calls` (model call history with search)
- LLM Usage section in Sessions view: API call count, input/output tokens, estimated cost, per-provider breakdown, model calls table, tool usage badges
- SQL-powered search in Network view: debounced search queries hit SQLite LIKE instead of client-side filtering
- `just update_prices` recipe to refresh bundled model pricing data
- `capsem-bench rootfs` benchmark: sequential and random 4K read performance on the read-only rootfs (where binaries and node_modules live)
- `capsem-bench startup` benchmark: cold-start latency for python3, node, claude, gemini, and codex CLIs (3 runs, min/mean/max)
- Rich table formatting for all capsem-bench output (replaces manual text formatting)
- Search category in settings: Google Search (on by default), Perplexity, and Firecrawl toggles with domain-level policy
- Custom allow/block domain lists (`network.custom_allow`, `network.custom_block`) for user-defined domain rules
- Active Policy debug panel in Network view: collapsible section showing allowed/blocked domain lists, default action, corp managed status, and policy conflicts
- Policy conflict detection: domains appearing in both allow and block lists are flagged in the Network view

### Changed
- VM Disk Performance Overhaul: Smashed past **2 Million IOPS** for random 4K reads (~8 GB/s) and achieved **~20x speedup** in random write throughput.
- Network Proxy Overhaul: Replaced synchronous thread-per-connection guest proxy with a **Tokio-based async implementation**.
- Structural Latency Elimination: Enabled `TCP_NODELAY` on both guest and host proxies, reducing proxy overhead to the physical network floor (~40ms median RTT).
- VM CPU default increased from 2 to 4 cores
- VM RAM default increased from 512 MB to 4 GB
- Scratch disk default increased from 8 GB to 16 GB
- Node.js V8 heap cap raised from 512 MB to 2 GB to match higher RAM
- Network store is now SQL-driven: counts and charts read from `get_session_stats` instead of counting JS arrays
- Session info response expanded with LLM metrics (model call count, tokens, tool calls, estimated cost)
- `net_events` command accepts optional `search` parameter for SQL-backed filtering
- `get_session_info` is now async with `spawn_blocking` for proper non-blocking DB access
- Rootfs disk caching mode changed from `Automatic` to `Cached` for aggressive host page cache retention on the read-only disk
- `elie.net` moved from a Package Registry toggle to the default custom allowed domains list
- `network.log_bodies` and `network.max_body_capture` moved from Network to VM category
- Session settings (`session.retention_days`, `session.max_sessions`, `session.max_disk_gb`) moved from Session to VM category
- Mock data now mirrors the full backend settings registry (~35 settings across 7 categories)
- Settings view categories are now displayed in a fixed order: AI Providers, Search, Package Registries, Network, Guest Environment, Appearance, VM
- Settings view categories are collapsed by default (click to expand)
- Network view: allowed/blocked domain lists are now separate collapsible groups within Active Policy
- Network events for `elie.net` and `www.google.com` now show as allowed (matching their default policy)
- Host-side disk settings: enabled host-level caching (`VZDiskImageCachingMode::Cached`) and disabled synchronization barriers (`VZDiskImageSynchronizationMode::None`).
- Guest-side kernel tuning: `capsem-init` now sets I/O scheduler to `none`, `read_ahead_kb` to 4096, and `nr_requests` to 256 for all VirtIO devices.
- Filesystem optimizations: Added `noatime,nodiratime,noload` mount options for rootfs and scratch disks.
- Scratch disk format optimization: `mke2fs` now uses `-m 0` to reclaim reserved root blocks.

### Fixed
- Benign "error shutting down connection" warnings in the host proxy logs are now filtered.

### Removed
- `registry.elie.allow` setting (replaced by `network.custom_allow` default)
- `registry.debian.allow` setting (rootfs is read-only, packages cannot be installed at runtime)
- Invented `network.allowlist` and `network.blocklist` settings from mock data (never existed in backend)
- `domainlist` setting type from frontend (custom allow/block use standard `text` type with ID-based chip rendering)

### Added
- `capsem-logger` crate: unified audit database with dedicated writer thread, replacing three separate SQLite databases (`WebDb`, `GatewayDb`, `AiDb`) with a single `session.db` per VM session
- Dedicated writer thread using `tokio::sync::mpsc` channel with block-then-drain batching (up to 128 ops per transaction), eliminating `spawn_blocking` + `Arc<Mutex<>>` contention
- `DbWriter` / `DbReader` API: async writes via channel, read-only WAL concurrent readers, typed `WriteOp` enum for debuggable operations
- Unified schema: `net_events` (all HTTPS connections), `model_calls` (denormalized request+response), `tool_calls`, `tool_responses` tables in a single DB file
- `capsem-bench` in-VM performance benchmark tool: disk I/O (sequential read/write, random 4K IOPS) and HTTP throughput (ab-style concurrent requests with latency percentiles). Outputs structured JSON to stdout for telemetry collection.
- Inline SSE event parsing in the MITM proxy for AI provider traffic (Anthropic, OpenAI, Google Gemini)
- Provider-agnostic LLM event types (`LlmEvent`, `StreamSummary`) with `collect_summary()` for structured audit logging
- Hand-rolled SSE wire-format parser with chunk-boundary-safe state machine (no crate dependency)
- Provider-specific SSE stream parsers: Anthropic (interleaved content blocks, thinking), OpenAI Chat Completions (tool calls, content filter), Google Gemini (complete events, synthetic call IDs)
- Request body parser extracting model, stream flag, system prompt preview, message/tool counts, and tool_result entries for tool call lifecycle linking
- `AiResponseBody`: hyper Body wrapper that does SSE parsing inline during `poll_frame` with zero added latency
- AI provider domain detection (`api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`) in the MITM proxy
- API key suffix extraction (last 4 chars, Stripe-style) from `x-api-key` and `Authorization: Bearer` headers

### Changed
- MITM proxy and gateway server use `DbWriter` channel instead of `spawn_blocking` + `Arc<Mutex<>>` for all database writes
- Session telemetry stored in `session.db` (was `info.db`)

### Removed
- `GatewayDb` (redundant flat table, replaced by `model_calls` in unified schema)
- `AiDb` (normalized 4-table schema, merged into `capsem-logger`)
- `WebDb` (replaced by `net_events` table in unified schema)
- `StreamAccumulator` (unused since `AiResponseBody` replaced it)
- Gateway TODO comment about separate vsock:5004 routing (replaced by inline MITM proxy SSE parsing)

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

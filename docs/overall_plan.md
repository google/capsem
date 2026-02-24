# Capsem VM - Sandboxed AI Agent Execution Platform

## Context
Building a green-field native macOS Rust application that spawns sandboxed Linux VMs (via Apple Virtualization.framework) to securely run AI agent CLIs (Claude Code, Gemini CLI). Replaces the existing capsem.org project with a fully native, Rust-first implementation. The goal is security-by-default: immutable base OS, network filtering, filesystem controls, MCP gateway proxying, and API traffic inspection.

## Architecture Overview

```
+------------------------------------------+
|  Tauri 2.0 App (macOS)                   |
|  +------------------------------------+  |
|  | WebView UI (Svelte 5)              |  |
|  | - Session manager                  |  |
|  | - Terminal (xterm.js via PTY/vsock) |  |
|  | - Stats dashboard                  |  |
|  | - MCP approval dialogs             |  |
|  +------------------------------------+  |
|           |  Tauri IPC                    |
|  +------------------------------------+  |
|  | Rust Backend (tokio runtime)       |  |
|  | - VM lifecycle manager             |  |
|  | - AI gateway (axum, native API)    |  |
|  | - Network proxy (vsock-bridged)    |  |
|  | - MCP gateway + policy engine      |  |
|  | - Session store (SQLite)           |  |
|  | - macOS Keychain key store         |  |
|  +------------------------------------+  |
|           |                              |
|  +------------------------------------+  |
|  | Apple Virtualization.framework     |  |
|  | (objc2-virtualization v0.3.2)      |  |
|  +------------------------------------+  |
|       | vsock only  | VirtioFS           |
|       | (NO NIC)    | (shared dirs)      |
+-------|-------------|--------------------+
        v             v
+------------------------------------------+
| Debian bookworm-slim ARM64 VM            |
| - Immutable squashfs root + overlayfs    |
| - NO network interface (air-gapped)      |
| - dummy0 NIC + fake DNS -> SNI routing   |
| - guest-agent IS PID 1 (no systemd)      |
| - TCP bridged via vsock (SNI + Gateway)  |
| - Rosetta 2 for x86_64 binary compat    |
| - Claude Code / Gemini CLI               |
| - Pre-warmed npm/pip caches (overlay)    |
+------------------------------------------+
```

### Critical Design Decisions (from architecture review)

1. **Debian over Alpine**: Alpine uses musl libc. AI agents frequently `pip install numpy/pandas` which require glibc manylinux wheels. Debian bookworm-slim adds ~40MB but guarantees 100% Python wheel compatibility.

2. **Air-gapped VM with Fake-IP SNI routing**: No `VZNetworkDeviceAttachment`. But a NIC-less VM has no default route and no DNS, so `curl` would fail at `gethostbyname()` and the kernel would return `ENETUNREACH` before iptables ever fires. **Fix**: Guest-agent creates a `dummy0` interface + default route, runs a fake DNS server on 127.0.0.1:53 that resolves ALL domains to `10.0.0.1`, and redirects TCP to vsock. Host-side proxy ignores the fake IP, extracts the real domain from the TLS SNI field, validates against the allow-list, and routes upstream. Zero DNS leaks, zero DNS rebinding attacks.

3. **Layer 7 AI API Gateway (not an OpenAI translator)**: Claude Code uses native Anthropic `/v1/messages` API with prompt caching, tool-use schemas, etc. We do NOT translate formats. Instead of complex TLS MITM, we configure the agents via environment variables (e.g., `ANTHROPIC_BASE_URL=http://10.0.0.1:8080`) to send plain HTTP traffic to a dedicated vsock port. The host-side Axum gateway acts as a Layer 7 reverse proxy: it receives native Anthropic/Gemini JSON payloads, inspects/logs them, injects API keys, and initiates a brand new HTTPS connection to the upstream provider. Zero protocol translation, but full HTTP reconstruction.

4. **PTY over vsock (not serial console)**: Serial ports cannot handle terminal resize (`SIGWINCH`). The guest-agent allocates `/dev/ptmx` pseudo-terminals and wires I/O over vsock. Serial is used ONLY for kernel boot logs.

5. **No VM snapshots**: Apple Virtualization.framework cannot snapshot VMs with VirtioFS shares attached. We use fast cold boot (<50ms target) + persistent overlay disk for session continuity instead.

6. **Tauri scaffolded from day one**: The tokio runtime runs inside Tauri's `setup` hook from Milestone 1. Avoids painful async-to-sync refactoring later.

7. **Rosetta 2 in-guest**: Mount `VZLinuxRosettaDirectoryShare` + register via binfmt_misc. ARM64 VM can execute x86_64 Linux binaries transparently.

8. **Clock sync on resume**: `SyncTime` vsock message corrects guest clock drift after pause/resume, preventing TLS certificate validation failures.

9. **MCP server sandboxing**: Host-side MCP servers wrapped in macOS `sandbox-exec` Seatbelt profiles, confined to workspace directory only.

10. **Guest-agent IS init (PID 1, no systemd)**: Kernel cmdline `init=/usr/bin/guest-agent`. Guest-agent mounts /proc, /sys, /dev, sets up dummy0 + fake DNS, mounts overlayfs, starts vsock listeners. Eliminates systemd's 1-2s boot overhead and dozens of unnecessary services. Target boot time: <50ms.

11. **Security-scoped bookmarks for workspace paths**: macOS app sandbox revokes folder access on quit. We persist `NSURL` security-scoped bookmarks (not string paths) in SQLite. On session resume, resolve bookmark + call `startAccessingSecurityScopedResource()` before booting VM. Without this, VirtioFS would crash on resume.

12. **Graceful shutdown on Cmd+Q**: Intercept `WindowEvent::CloseRequested` in Tauri, call `api.prevent_close()`, send `Shutdown { graceful: true }` over vsock, guest-agent runs `sync` + unmounts overlay disk + ACPI poweroff. Only exit after VM reaches `Stopped` state. Prevents ext4 corruption on the persistent overlay disk.

13. **OverlayFS on top of read-only VirtioFS caches**: `npm install` and `pip install` write `.lock` files and metadata to cache dirs even on 100% cache hit. Read-only VirtioFS would cause instant crash. **Fix**: Stack ephemeral overlayfs inside the guest on top of each read-only VirtioFS cache mount. Host cache stays clean, tools write temp data to tmpfs upper layer.

14. **Tauri IPC Multiplexing and Backpressure**: High-volume data streams (PTY output, AI gateway logs, MCP audit trails) can overwhelm the Tauri IPC channel, causing UI lag or memory exhaustion. We **multiplex** these logical streams over session-specific **Tauri 2.0 Channels** rather than using global events. To prevent "head-of-line blocking" and UI freezes during bursts, we implement **credit-based backpressure**: the Svelte frontend grants "rendering credits" to the Rust backend, which in turn throttles vsock consumption from the guest until more credits are available.

---

## Milestone 1: Tauri Scaffold + Boot a Linux VM from Rust

**Goal**: Prove we can boot a Debian ARM64 VM from Rust inside a Tauri app shell, see kernel output on a serial console.

**Deliverable**: A Tauri app that boots a Linux VM. Kernel boot messages appear in the app's debug console. A placeholder "VM is running" UI.

**Key crates**:
- `tauri` v2 - app shell with async setup hook
- `objc2` + `objc2-virtualization` + `objc2-foundation` - Apple framework bindings
- `block2` - Objective-C block support for completion handlers
- `anyhow`, `thiserror` - error handling
- `tokio` - async runtime (spawned inside Tauri setup)
- `tracing` + `tracing-subscriber` - structured logging

**Project structure** (Tauri from the start):
```
capsem/
  Cargo.toml                    # workspace root
  crates/
    capsem-core/                # core library (VM, session, gateway, etc.)
      Cargo.toml
      src/
        lib.rs
        vm/
          mod.rs
          config.rs             # VmConfig builder (cpu, ram, kernel, initrd, disk)
          machine.rs            # VZVirtualMachine create/start/stop
          boot.rs               # VZLinuxBootLoader setup
          serial.rs             # serial console I/O via file handles
    capsem-app/                 # Tauri app binary
      Cargo.toml
      src/
        main.rs                 # Tauri main with tokio runtime in setup hook
        commands.rs             # Tauri IPC commands (stub)
        state.rs                # AppState with VM handle
      tauri.conf.json
      capabilities/
        default.json
      Info.plist                # com.apple.security.virtualization entitlement
  frontend/                     # Svelte frontend (scaffold only)
    package.json
    src/
      routes/+page.svelte      # placeholder UI
      app.html
  entitlements.plist            # code signing entitlements
  scripts/
    fetch-kernel.sh             # download Debian ARM64 kernel + initrd
```

**Tests**:
- Unit: VmConfig builder validates cpu/ram bounds
- Unit: VmConfig builder rejects missing kernel path
- Integration: Boot VM, capture serial output, assert "Linux version" appears
- Integration: VM start -> stop lifecycle completes without crash
- Integration: Tauri app launches, tokio runtime active in setup hook

**Setup needed**:
- Download Debian ARM64 cloud kernel (`vmlinuz`) and initrd from Debian cloud images
- Create a minimal ext4 rootfs image (script provided)
- Code-sign with `com.apple.security.virtualization` entitlement

**NOT included**: No vsock, no VirtioFS, no networking, no agent CLI, no guest-agent.

---

## Milestone 2: Guest Agent as PID 1 + vsock Control Channel

**Goal**: Build a Rust guest-agent that IS the init process (PID 1). It boots the VM in <50ms, mounts essential filesystems, and provides a structured vsock control channel. No systemd.

**Deliverable**: Host sends structured JSON commands over vsock, guest-agent executes them and returns results. VM boots to "ready" in under 50ms. Serial console remains for kernel boot logs only.

**New modules**:
```
  crates/
    capsem-core/src/
      vm/
        vsock.rs                # VZVirtioSocketDeviceConfiguration, host-side listener
        lifecycle.rs            # state machine: Created->Booting->Ready->Running->Stopped
      protocol/
        mod.rs
        messages.rs             # host<->guest message types (serde, JSON)
        framing.rs              # length-prefixed message framing over vsock stream
    capsem-guest-agent/         # Rust binary cross-compiled for aarch64-unknown-linux-gnu
      Cargo.toml
      src/
        main.rs                 # PID 1 init: mount /proc /sys /dev, setup, vsock listen
        init.rs                 # early boot: mount filesystems, set hostname, basic /dev
        executor.rs             # runs shell commands, captures output
        health.rs               # uptime, memory, disk stats
```

**Guest-agent as PID 1 boot sequence**:
1. Mount `/proc`, `/sys`, `/dev` (devtmpfs), `/dev/pts`, `/tmp` (tmpfs)
2. Set hostname
3. Mount squashfs root overlay (if not already kernel-mounted)
4. Start vsock listeners on ports 5000 (control), 5001 (terminal), 5002 (network)
5. Signal "Ready" to host via vsock port 5000

**Kernel cmdline**: `console=hvc0 root=/dev/vda ro init=/usr/bin/guest-agent`

**vsock protocol messages**:
- `Health` -> `HealthResponse { uptime_secs, mem_total, mem_free, agent_version }`
- `Exec { command, args, env, cwd }` -> `ExecResult { exit_code, stdout, stderr }`
- `Signal { pid, signal }`
- `Shutdown { graceful: bool }` -> guest-agent runs `sync`, unmounts, triggers `reboot(LINUX_REBOOT_CMD_POWER_OFF)`
- `SyncTime { unix_timestamp_secs }` -> `SyncTimeResult { ok: bool }` (calls `settimeofday`)

**Guest-agent setup**:
- Cross-compile: `cargo build --target aarch64-unknown-linux-gnu` (statically linked with musl for init binary)
- Baked into rootfs at `/usr/bin/guest-agent`
- PID 1: must handle SIGCHLD (reap zombies), must not exit (kernel panic if PID 1 dies)

**Tests**:
- Unit: Message serialization/deserialization round-trips for all types
- Unit: Frame encoding/decoding with 0-byte, 1KB, 1MB payloads
- Unit: Lifecycle state machine rejects invalid transitions
- Integration: Boot VM, guest-agent starts as PID 1, responds to Health within 100ms
- Integration: Exec `echo hello` via vsock, get "hello" back
- Integration: Exec `uname -a` returns aarch64 Linux
- Integration: SyncTime sets guest clock correctly
- Integration: Shutdown command triggers clean VM stop (no ext4 corruption)
- Benchmark: Time from VM start to Health response < 100ms

**NOT included**: No PTY, no VirtioFS, no networking, no dummy0/DNS (added in M5).

---

## Milestone 3: Immutable Debian Base Image Builder

**Goal**: Automated build of the Debian bookworm-slim ARM64 base image with immutable squashfs root, overlayfs, Rosetta 2, and pre-installed agent CLIs.

**Deliverable**: A build script/tool that produces a ready-to-boot VM image. Image boots immutable with overlay writes going to tmpfs.

**New modules**:
```
  crates/
    capsem-core/src/
      image/
        mod.rs
        builder.rs              # orchestrates image build
        debian.rs               # Debian package list, debootstrap config
        overlay.rs              # overlayfs init script + systemd units
        cache.rs                # npm/pip cache pre-warming logic
        rosetta.rs              # Rosetta 2 binfmt_misc registration
  scripts/
    build-base-image.sh         # debootstrap + customization + mksquashfs
    setup-rosetta.sh            # binfmt_misc registration for x86_64
```

**Image contents**:
- Debian bookworm-slim ARM64 (NO systemd): bash, curl, git, ca-certificates, iptables, iproute2
- Node.js 22 LTS + npm
- Python 3.11 + pip (Debian's default python3)
- Claude Code CLI (`npm install -g @anthropic-ai/claude-code`)
- Gemini CLI (`npm install -g @google/gemini-cli`)
- Guest-agent binary at `/usr/bin/guest-agent` (PID 1 init from Milestone 2)
- Rosetta 2 binfmt_misc setup (mount point + registration script)
- Pre-populated npm global cache and pip cache directories
- Immutable: squashfs root + overlayfs (tmpfs upper for ephemeral, sparse .raw disk upper for persistent sessions)
- No systemd, no services -- guest-agent handles all init duties

**Tests**:
- Unit: Package list generation is correct and complete
- Unit: Overlay init script produces valid systemd mount units
- Integration: Built image boots successfully in VM
- Integration: `cat /proc/mounts` shows squashfs on `/` with overlay
- Integration: Write to `/` fails with EROFS (read-only)
- Integration: Write to `/tmp`, `/home`, `/var/tmp` succeeds (overlay)
- Integration: `claude --version` succeeds inside VM
- Integration: `gemini --version` succeeds inside VM
- Integration: `pip install requests` succeeds (glibc wheels work)
- Integration: npm/pip caches are warm (install is near-instant for cached pkgs)
- Integration: Rosetta 2 registered (x86_64 ELF runs via `/usr/libexec/rosetta`)

**NOT included**: No network bridge, no VirtioFS workspace sharing yet.

---

## Milestone 4: VirtioFS Shared Directories + PTY over vsock

**Goal**: Host shares workspace directories into VM via VirtioFS. Terminal sessions use proper PTY allocation over vsock (not serial).

**Deliverable**: xterm.js in Tauri connects to a real PTY inside the VM. Users can interact with a shell that handles resize, colors, cursor movement correctly.

**New modules**:
```
  crates/
    capsem-core/src/
      vm/
        virtfs.rs               # VZVirtioFileSystemDeviceConfiguration setup
        rosetta.rs              # VZLinuxRosettaDirectoryShare config
      terminal/
        mod.rs
        pty_proxy.rs            # host-side: vsock <-> Tauri 2.0 Channel bridge with backpressure
    capsem-guest-agent/src/
      pty.rs                    # allocate /dev/ptmx, fork, wire to vsock
      resize.rs                 # handle SIGWINCH from host resize events
  frontend/src/
    lib/components/
      Terminal.svelte           # xterm.js component, sends/receives via Tauri channels + backpressure
    lib/stores/
      terminal.ts              # terminal state + Tauri event bindings
```

**VirtioFS shares**:
- Tag `workspace` -> user project dir (read-write, selected via native picker or config)
- Tag `cache-npm` -> `~/.capsem/cache/npm` (read-only from host)
- Tag `cache-pip` -> `~/.capsem/cache/pip` (read-only from host)
- Tag `config` -> session config dir (read-only, agent settings, MCP config)
- Tag `rosetta` -> `VZLinuxRosettaDirectoryShare` (Rosetta 2 translation layer)

**Cache overlay trick** (prevents npm/pip crash on read-only VirtioFS):
Package managers write `.lock` files and metadata to cache dirs even on cache hits.
Read-only VirtioFS would crash them. Fix: stack ephemeral overlayfs in guest:
```
# Guest-agent init sequence for caches:
mount -t virtiofs cache-npm /mnt/cache/npm          # read-only host cache
mount -t overlay overlay \
  -o lowerdir=/mnt/cache/npm,upperdir=/tmp/npm_upper,workdir=/tmp/npm_work \
  /root/.npm                                         # npm sees writable cache
# Same for pip -> /root/.cache/pip
```
Host cache stays perfectly clean. Temp writes go to tmpfs.

**vsock protocol additions**:
- `SpawnPty { cols, rows, shell, env, cwd }` -> `SpawnPtyResult { pty_id }`
- `ResizePty { pty_id, cols, rows }` -> `Ok`
- `PtyData { pty_id, data: Vec<u8> }` (bidirectional, on vsock port 5001)
- `ClosePty { pty_id }` -> `Ok`

**Tests**:
- Unit: VirtioFS config builder produces valid device configs
- Integration: Mount host directory in VM via VirtioFS tag, file visible both sides
- Integration: Write file from VM `/workspace/test.txt`, appears on host
- Integration: Read-only share (`cache-npm`) rejects writes from VM
- Integration: SpawnPty returns pty_id, PtyData flows bidirectionally
- Integration: ResizePty changes terminal dimensions (verify via `stty size`)
- Integration: xterm.js in Tauri renders shell prompt, accepts input, shows output
- Integration: `ls --color=auto` produces ANSI color codes rendered correctly
- Integration: Ctrl+C sends SIGINT to process in PTY

**NOT included**: No network bridge, no AI gateway, no MCP gateway.

---

## Milestone 5: vsock Network Bridge + Fake-IP SNI Router + Domain Filtering

**Goal**: VM has NO real network interface. A synthetic network stack (dummy0 + fake DNS + SNI routing) bridges all TCP over vsock with HTTPS-only enforcement and domain allow/block lists.

**Deliverable**: `curl https://api.anthropic.com` works from inside VM (allowed domain). `curl https://evil.com` is blocked. There is physically no way to bypass the filter.

**The "Fake-IP SNI Router" architecture**:
```
VM (no real NIC):
  1. guest-agent creates dummy0 interface + default route
  2. guest-agent runs fake DNS on 127.0.0.1:53
     - ALL domains resolve to 10.0.0.1 (single fake IP)
  3. /etc/resolv.conf -> nameserver 127.0.0.1
  4. iptables routing splits traffic:
     - AI Gateway traffic (e.g., port 8080) -> vsock-bridge on 127.0.0.1:3129 -> vsock:5004
     - All OTHER TCP traffic -> SNI vsock-bridge on 127.0.0.1:3128 -> vsock:5002
  5. App calls: curl https://github.com
     -> DNS resolves github.com to 10.0.0.1
     -> TCP connect to 10.0.0.1:443
     -> iptables REDIRECT to vsock-bridge on 127.0.0.1:3128
     -> vsock-bridge sends raw bytes over vsock:5002 to host

Host (vsock:5002):
  5. Receives TCP stream
  6. Reads first bytes: TLS ClientHello
  7. Extracts SNI field: "github.com"
  8. Checks domain allow-list -> allowed
  9. Resolves REAL DNS for github.com on host
  10. Opens REAL TLS connection to github.com:443
  11. Bridges bidirectionally
```

**New modules**:
```
  crates/
    capsem-core/src/
      network/
        mod.rs
        proxy.rs                # host-side TCP proxy (tokio, per-connection)
        tls_sni.rs              # SNI extraction from TLS ClientHello bytes
        filter.rs               # domain allow/block list engine (glob patterns)
        policy.rs               # per-session network policy config
        dns.rs                  # host-side DNS resolution (trust-dns-resolver)
    capsem-guest-agent/src/
      network/
        mod.rs
        dummy_nic.rs            # create dummy0, add default route
        fake_dns.rs             # UDP DNS server: all queries -> 10.0.0.1
        iptables.rs             # REDIRECT rules: all TCP to local vsock bridge
        bridge.rs               # TCP listener on 127.0.0.1:3128 -> vsock:5002
```

**Guest-agent network boot sequence** (called from init.rs):
1. `ip link add dummy0 type dummy && ip link set dummy0 up`
2. `ip addr add 10.0.0.1/32 dev dummy0`
3. `ip route add default dev dummy0`
4. Start fake DNS on 127.0.0.1:53 (resolve everything to 10.0.0.1)
5. Write `nameserver 127.0.0.1` to /etc/resolv.conf
6. `iptables -t nat -A OUTPUT -p tcp -d 10.0.0.1 --dport 8080 -j REDIRECT --to-ports 3129` (AI Gateway)
7. `iptables -t nat -A OUTPUT -p tcp -j REDIRECT --to-ports 3128` (Catch-all SNI Router)
8. Start TCP bridge on 127.0.0.1:3128 -> vsock:5002 (SNI)
9. Start TCP bridge on 127.0.0.1:3129 -> vsock:5004 (AI Gateway)

**Default network policy**:
- Allow only TLS (port 443 implied by SNI routing -- non-TLS has no SNI, gets rejected)
- Default allow: `api.anthropic.com`, `*.googleapis.com`, `registry.npmjs.org`, `files.pythonhosted.org`, `pypi.org`, `github.com`, `*.github.com`
- Default block: everything else (no SNI match = connection dropped)
- Zero DNS leaks (fake DNS never leaves VM)
- No UDP forwarding, no ICMP, no raw sockets

**Tests**:
- Unit: SNI extraction from real TLS ClientHello byte captures
- Unit: Domain filter glob matching (`*.anthropic.com` matches `api.anthropic.com`)
- Unit: Policy evaluation priority (explicit block > allow > default deny)
- Unit: Fake DNS server returns 10.0.0.1 for all A queries
- Integration: VM `curl https://api.anthropic.com` succeeds (returns 401, no key yet)
- Integration: VM `curl https://evil.com` fails (domain not in allow-list)
- Integration: VM `curl http://example.com` fails (no SNI in plain HTTP -> rejected)
- Integration: VM `ping 8.8.8.8` fails (no real NIC, ICMP impossible)
- Integration: VM `dig @8.8.8.8 google.com` fails (UDP not bridged)
- Integration: Custom session policy adds `*.example.com` to allow list, works
- Security: `unset HTTPS_PROXY && curl https://evil.com` still fails (no bypass possible)
- Security: DNS resolution in VM always returns 10.0.0.1 (host does real resolution)

**NOT included**: No API inspection, no MCP gateway.

---

## Milestone 6: AI Gateway - Transparent API Proxy

**Goal**: Model API calls pass through a host-side gateway that inspects native Anthropic/Gemini payloads, injects API keys, logs everything, and applies rate limits. No protocol translation.

**Deliverable**: Claude Code and Gemini CLI work normally inside VM, but all API traffic is logged with full request/response bodies. API keys never enter the VM.

**Architecture**:
```
VM:
  claude-code -> POST http://10.0.0.1:8080/v1/messages (native Anthropic format)
  gemini-cli  -> POST http://10.0.0.1:8080/v1/gemini/... (native Gemini format)
     (routed via dedicated iptables rule -> vsock:5004 to host)

Host:
  Axum gateway on vsock:5004 listener (Layer 7 HTTP Reverse Proxy):
    /v1/messages     -> inject x-api-key header -> proxy to https://api.anthropic.com
    /v1/gemini/*     -> inject API key param   -> proxy to https://generativelanguage.googleapis.com
    Log request body, stream response, log response, count tokens
```

**New modules**:
```
  crates/
    capsem-core/src/
      gateway/
        mod.rs
        server.rs               # axum router, per-session middleware
        anthropic.rs            # Anthropic /v1/messages passthrough + inspection
        google.rs               # Gemini API passthrough + inspection
        key_store.rs            # macOS Keychain via security-framework
        logger.rs               # structured request/response logging to SQLite
        cost.rs                 # token counting + cost estimation per provider/model
        rate_limit.rs           # per-session token-bucket rate limiter
        streaming.rs            # SSE stream passthrough with token counting
```

**Key crates**:
- `axum` - HTTP server
- `reqwest` - upstream HTTP client with streaming
- `security-framework` - macOS Keychain for API keys
- `async-stream` - SSE stream inspection

**How it works**:
- Gateway binds to `localhost:<port>`, reachable from VM via vsock network bridge
- VM env vars: `ANTHROPIC_BASE_URL=http://10.0.0.1:8080` (or vsock-mapped host addr)
- Gateway receives native API request, deserializes *just enough* to extract: model, token counts, tool definitions
- Injects `x-api-key` (Anthropic) or `?key=` (Google) from macOS Keychain
- Streams raw bytes to upstream provider over HTTPS
- Logs: timestamp, session_id, provider, model, input_tokens, output_tokens, cost_estimate, latency, tool_names_used
- Does NOT modify request/response payloads (transparent proxy)

**Tests**:
- Unit: Anthropic request body parsing (extract model, tool names, estimate input tokens)
- Unit: Gemini request body parsing
- Unit: Cost estimation for Claude 3.5 Sonnet, Gemini 2.5 Pro, etc.
- Unit: Rate limiter token bucket (allow, then throttle, then recover)
- Unit: SSE stream line-by-line forwarding preserves all events
- Integration: Mock Anthropic upstream -> gateway -> client, full round trip
- Integration: Streaming response forwarded correctly with token count
- Integration: API keys present in upstream request, absent from VM env
- Integration: Rate limiter returns 429 when exceeded
- Integration: Cost logger writes correct records to SQLite

**NOT included**: No MCP gateway, no UI for stats.

---

## Milestone 7: MCP Gateway + Host-Side Server Sandboxing

**Goal**: MCP tool calls from agents are intercepted by a gateway that applies security policies. Host-side MCP servers run in macOS Seatbelt sandboxes.

**Deliverable**: Agents can use MCP tools, but dangerous operations are blocked or require host approval. MCP servers cannot escape workspace directory.

**New modules**:
```
  crates/
    capsem-core/src/
      mcp/
        mod.rs
        gateway.rs              # MCP JSON-RPC proxy (vsock <-> stdio bridge)
        policy.rs               # tool allow/block/approval-required policies
        audit.rs                # MCP call audit logging to SQLite
        approval.rs             # approval queue (tokio::sync::watch for UI notification)
        sandbox.rs              # macOS sandbox-exec profile generation
      protocol/
        mcp_types.rs            # MCP JSON-RPC message types (request, response, notification)
    capsem-guest-agent/src/
      mcp_stub.rs               # in-VM MCP "server" that forwards over vsock to host gateway
```

**Architecture**:
```
VM:
  claude-code --mcp-config /config/mcp.json
    -> connects to mcp-stub (localhost stdio)
      -> forwards JSON-RPC over vsock:5003 to host

Host:
  MCP Gateway (vsock:5003):
    -> receives JSON-RPC
    -> evaluates policy (allow/block/approval)
    -> if approved: spawn real MCP server in sandbox-exec jail
    -> forward request, return response
    -> audit log everything
```

**Seatbelt sandboxing** (macOS native):
- Dynamically generate `.sb` profile per session
- MCP server process confined to: workspace directory (r/w), /tmp (r/w), system libs (r/o)
- Cannot read: ~/.ssh, ~/.aws, ~/.config, ~/Documents, etc.
- Cannot write: anywhere outside workspace
- Cannot access network (MCP servers talk through our gateway only)

**Policies**:
- Allow: read file, list directory, search (within workspace)
- Block: write outside workspace, shell commands with `rm -rf`, network access
- Approval required: shell commands, file deletion, git push, external API calls
- Per-session configurable policy TOML

**Tests**:
- Unit: MCP JSON-RPC parsing for all message types
- Unit: Policy evaluation for various tool calls
- Unit: Seatbelt profile generation with correct paths
- Unit: Audit log entry generation
- Integration: MCP tool call flows through gateway to real MCP server
- Integration: Blocked tool call returns proper JSON-RPC error
- Integration: Approval-required call queues (waits for signal)
- Integration: MCP server cannot read ~/.ssh/id_rsa (Seatbelt blocks)
- Integration: MCP server can read/write files in workspace

**NOT included**: No approval UI (queue is wired but approval is auto-accept in CLI mode).

---

## Milestone 8: Session Management + Persistence

**Goal**: Full session lifecycle with persistence. Sessions survive app restart via SQLite + persistent overlay disk.

**Deliverable**: Create session, run agent, stop, resume later with history and workspace intact.

**New modules**:
```
  crates/
    capsem-core/src/
      session/
        mod.rs
        manager.rs              # orchestrates full session lifecycle
        persistence.rs          # SQLite-backed session store
        config.rs               # per-session configuration (agent, policy, workspace)
        history.rs              # terminal scrollback + command history
        overlay_disk.rs         # sparse .raw file as persistent overlayfs upper
      db/
        mod.rs
        schema.rs               # SQLite migrations
        queries.rs              # typed queries (sqlx or rusqlite)
```

**Session resume strategy** (NO VM snapshots due to VirtioFS limitation):
- On stop: host sends `Shutdown { graceful: true }` -> guest-agent runs `sync`, unmounts overlay disk, triggers ACPI poweroff
- Persistent overlay disk (sparse `.raw` file per session) preserves `/home`, `/var`, `/etc` changes
- On resume: boot fresh VM, mount same overlay disk as upperdir, mount same workspace
- Terminal scrollback history stored in SQLite (replay on reconnect)
- Session metadata: agent type, model, network policy, workspace path (as security-scoped bookmark), cumulative cost

**Security-scoped bookmarks**:
- When workspace folder selected via NSOpenPanel, create NSURL bookmark with `NSURLBookmarkCreationWithSecurityScope`
- Store bookmark `Vec<u8>` (base64) in SQLite alongside session
- On resume: resolve bookmark -> `startAccessingSecurityScopedResource()` -> boot VM with VirtioFS
- On stop: `stopAccessingSecurityScopedResource()`
- Without this, macOS app sandbox revokes folder access on quit -> VirtioFS mount fails on resume

**Key crates**:
- `rusqlite` - SQLite (or `sqlx` with SQLite feature)
- `serde` + `serde_json` - config serialization

**Clock sync on resume**:
- Session manager sends `SyncTime` immediately after VM reaches Ready state
- Prevents TLS cert validation failures from clock drift

**Tests**:
- Unit: Session CRUD operations against SQLite
- Unit: Schema migrations apply cleanly (up and down)
- Unit: Config serialization round-trip
- Unit: Overlay disk creation (sparse file, correct size)
- Integration: Create session -> exec command -> stop -> resume -> previous files still exist
- Integration: App restart -> session list restored from SQLite
- Integration: Terminal scrollback replayed on session reconnect
- Integration: Clock sync after resume (guest time matches host within 2s)
- Integration: Concurrent sessions with separate overlay disks don't interfere
- Integration: Session delete cleans up overlay disk file

**NOT included**: No stats UI yet.

---

## Milestone 9: Full Tauri UI - Session Manager + Terminal + Workspace Picker

**Goal**: Polished native macOS app with session management, workspace selection, and terminal interaction.

**Deliverable**: A real app. Create sessions with native folder picker, interactive terminal, session list with status.

**Frontend structure**:
```
  frontend/src/
    lib/
      components/
        SessionList.svelte        # list of sessions with status badges
        CreateSession.svelte      # dialog: agent type, workspace picker, policy
        Terminal.svelte            # xterm.js, PTY over vsock via Tauri events
        SessionDetail.svelte      # session info, config, basic stats
        WorkspacePicker.svelte    # native macOS NSOpenPanel via Tauri dialog plugin
      stores/
        sessions.ts               # session state, Tauri IPC calls
        terminal.ts               # terminal I/O event bridge
        theme.ts                  # light/dark mode
      api/
        tauri.ts                  # typed Tauri invoke wrappers
    routes/
      +page.svelte                # main app layout
      +layout.svelte              # app shell with sidebar
    app.html
    app.css                       # TailwindCSS
```

**Tauri IPC commands**:
- `list_sessions` -> `Vec<SessionSummary>`
- `create_session { agent, workspace_path, network_policy }` -> `SessionId`
- `delete_session { id }` -> `Ok`
- `start_session { id }` -> `Ok` (boots VM, starts agent)
- `stop_session { id }` -> `Ok`
- `resume_session { id }` -> `Ok`
- `attach_pty { session_id, pty_id, channel }` -> `Ok` (binds PTY stream to a Tauri 2.0 Channel for backpressured output)
- `terminal_input { session_id, data: Vec<u8> }` (host -> guest PTY)
- `pick_directory` -> `Option<String>` (native folder picker)
- `get_session_config { id }` -> `SessionConfig`

**Tests**:
- Unit: Tauri commands return correct types (mock backend)
- Unit: Svelte components render without errors (vitest + testing-library)
- Integration: Create session from UI -> VM boots -> terminal shows shell prompt
- Integration: Type in terminal -> command executes -> output appears
- Integration: Terminal resize -> guest PTY resizes (stty size matches)
- Integration: Session list updates in real-time on status change
- Integration: Native folder picker returns valid path
- E2E: Open app -> create Claude session -> run `echo hello` -> see output -> stop

**NOT included**: No stats dashboard, no MCP approval UI, no cost tracking UI.

---

## Milestone 10: Stats Dashboard + MCP Approval UI + Live Monitoring

**Goal**: Rich monitoring and human-in-the-loop approval for sensitive operations.

**Deliverable**: Dashboard with API costs, token usage, MCP audit trail, live network activity, and approval dialogs.

**New frontend components**:
```
  frontend/src/lib/components/
    StatsOverview.svelte          # per-session + aggregate: cost, tokens, calls
    ApiCallLog.svelte             # timeline of API calls with expandable details
    McpAuditLog.svelte            # MCP tool call history with allow/block/pending status
    ApprovalDialog.svelte         # modal: tool name, args, approve/deny buttons
    CostChart.svelte              # cost over time (per session, cumulative)
    NetworkActivity.svelte        # live allowed/blocked connection log
```

**Backend additions**:
- `tokio::sync::broadcast` channels for real-time event streaming to Tauri frontend
- Tauri event emitters for: `api-call`, `mcp-call`, `mcp-approval-needed`, `network-event`
- Approval flow: MCP gateway emits `mcp-approval-needed` -> UI shows dialog -> user clicks approve/deny -> gateway unblocks

**Tests**:
- Unit: Stats aggregation (sum tokens, costs across calls)
- Unit: Cost calculation accuracy against known pricing
- Integration: API call appears in log within 1s
- Integration: MCP approval dialog blocks tool call until user clicks approve
- Integration: Approve -> tool executes, result returned to agent
- Integration: Deny -> agent receives JSON-RPC error
- Integration: Network activity log shows allowed and blocked connections
- E2E: Run Claude Code task -> see live API calls -> approve MCP tool -> see cost update

---

## Milestone 11: Polish, Security Hardening, Multi-Session

**Goal**: Production-quality security, concurrent multi-session, error recovery, code signing.

**Deliverable**: Run 5+ agents simultaneously with full isolation. Graceful error handling. Signed/notarized app.

**Focus areas**:
- **Graceful shutdown on Cmd+Q**: Intercept `WindowEvent::CloseRequested`, call `api.prevent_close()`, send `Shutdown { graceful: true }` to ALL running VMs, wait for guest-agent to `sync` + unmount + ACPI poweroff, only then `app_handle.exit(0)`. Prevents ext4 corruption on persistent overlay disks.
- VM isolation: no cross-session data leakage (separate vsock CIDs, separate overlay disks)
- Network hardening: verify zero-NIC airgap + fake-IP SNI routing holds under all conditions
- Error recovery: VM crash -> session shows error, can restart with persistent overlay intact (fsck on mount)
- Resource limits: per-session CPU count, memory cap, disk quota
- Cleanup: session delete removes overlay disk, vsock ports released, security-scoped bookmark released
- macOS code signing + notarization for distribution
- Auto-update mechanism (Tauri updater plugin)
- Config file: `~/.capsem/config.toml` for default policies, workspace paths, CLI mode

**Tests**:
- Security: VM process cannot access host filesystem outside VirtioFS shares
- Security: VM cannot send any network packet (no NIC verification)
- Security: API keys not in VM env, not in VM memory (search /proc/*/environ)
- Security: Session A cannot access Session B's workspace or overlay
- Security: MCP server in Seatbelt cannot read ~/.ssh/id_rsa
- Stress: 5 concurrent sessions, each running an agent, all isolated
- Recovery: Kill VM process -> session shows error, resume boots fresh VM with overlay
- Recovery: App crash -> relaunch -> sessions listed, resumable
- Resource: Session with 1 CPU / 512MB cannot exceed limits

---

## Milestone 12: Kernel Hardening -- Custom Minimal Kernel

**Goal**: Replace the stock Debian kernel with a custom-compiled minimal kernel, enforce SELinux mandatory access control, and strip the rootfs of all unnecessary binaries/files. Three layers of hardening: kernel attack surface reduction, MAC policy enforcement, and filesystem minimization.

**Deliverable**: A custom aarch64 Linux kernel (~2-4MB vs ~30MB stock) with `CONFIG_MODULES=n`, SELinux in enforcing mode with a tight policy, and a rootfs stripped to only the binaries the agent actually needs. No USB, no HID, no DRM, no sound, no loadable modules, no compilers, no package managers, no setuid binaries.

**Security verdict**: Strongest kernel security posture achievable. `CONFIG_MODULES=n` means even a root agent cannot dynamically load kernel code -- the kernel simply lacks the machinery to do so. This neutralizes kernel rootkits, malicious `.ko` files, and module-based persistence.

**Caveat**: Patch management. We own the kernel. When a high-severity Linux kernel CVE drops, `apt-get upgrade` won't save us. We must pull the patched upstream source, recompile via our Docker pipeline, and redeploy. The Docker-based build makes this mechanical (change the kernel source tag, rebuild) but it requires active monitoring of kernel security advisories.

**Kernel config (enabled)**:
```
# Core
CONFIG_64BIT=y
CONFIG_ARM64=y
CONFIG_SMP=y

# Virtio (all built-in, no modules)
CONFIG_VIRTIO=y
CONFIG_VIRTIO_PCI=y
CONFIG_VIRTIO_CONSOLE=y
CONFIG_VIRTIO_BLK=y
CONFIG_HW_RANDOM_VIRTIO=y
CONFIG_VIRTIO_BALLOON=y          # memory management
CONFIG_VIRTIO_FS=y               # future VirtioFS (M4)
CONFIG_VSOCK=y                   # future vsock (M2)
CONFIG_VIRTIO_VSOCKETS=y

# Filesystems
CONFIG_EXT4_FS=y
CONFIG_SQUASHFS=y                # future immutable root (M3)
CONFIG_OVERLAY_FS=y              # writable overlay
CONFIG_PROC_FS=y
CONFIG_SYSFS=y
CONFIG_DEVTMPFS=y
CONFIG_DEVTMPFS_MOUNT=y
CONFIG_TMPFS=y

# Terminal / console
CONFIG_TTY=y
CONFIG_VT=y
CONFIG_SERIAL_CORE=y
CONFIG_HVC_DRIVER=y

# Minimal networking (for future M5 dummy0 + iptables)
CONFIG_NET=y
CONFIG_INET=y
CONFIG_NETFILTER=y
CONFIG_IP_NF_IPTABLES=y
CONFIG_IP_NF_NAT=y
CONFIG_DUMMY=y

# Security
CONFIG_MODULES=n                 # THE BIG ONE: no loadable modules
CONFIG_SECURITY=y
CONFIG_SECCOMP=y
CONFIG_STRICT_KERNEL_RWX=y
CONFIG_SECURITY_SELINUX=y        # mandatory access control
CONFIG_SECURITY_SELINUX_BOOTPARAM=y
CONFIG_DEFAULT_SECURITY_SELINUX=y
CONFIG_AUDIT=y                   # SELinux needs audit subsystem
```

**Kernel config (disabled)**:
```
CONFIG_USB_SUPPORT is not set
CONFIG_HID is not set
CONFIG_DRM is not set
CONFIG_SOUND is not set
CONFIG_WLAN is not set
CONFIG_BLUETOOTH is not set
CONFIG_INPUT is not set           # no keyboard/mouse
CONFIG_NFS_FS is not set
CONFIG_CIFS is not set
CONFIG_WIRELESS is not set
CONFIG_RFKILL is not set
CONFIG_GPU is not set
CONFIG_FB is not set              # no framebuffer
CONFIG_MODULES is not set         # no loadable modules
```

**Build pipeline**:
```
images/
  Dockerfile.kernel-custom    Cross-compile minimal kernel in Docker
  kernel-config               Checked-in .config for reproducibility
```

The Dockerfile cross-compiles using Debian's `gcc-aarch64-linux-gnu` toolchain. The kernel source is fetched from kernel.org at a pinned tag (e.g., `v6.6.80`). The output is a `vmlinuz` and optional built-in initramfs.

**SELinux mandatory access control**:

SELinux provides a second layer of defense beyond filesystem permissions. Even if the agent gains root, SELinux policy restricts what root can do. The policy is baked into the rootfs at build time; the agent cannot modify it (read-only rootfs).

Policy goals:
- Agent process (bash, claude-code, node, python) confined to a `capsem_agent_t` domain
- `capsem_agent_t` can: read/write workspace (`/workspace`), read system libs, execute allowed binaries, write to tmpfs mounts
- `capsem_agent_t` cannot: write to `/usr`, `/bin`, `/sbin`, `/lib`, `/etc`; access raw block devices; mount filesystems; load kernel modules; change SELinux policy; access `/proc/kcore`, `/proc/kallsyms`, or other sensitive proc entries; use `ptrace` on PID 1
- PID 1 (capsem-init / guest-agent) runs as `capsem_init_t` with full system access (it needs to mount, chroot, etc.)
- Transition: `capsem_init_t` -> `capsem_agent_t` when bash is exec'd in the chroot
- SELinux mode: enforcing (not permissive). Violations are denied, not just logged.
- Kernel cmdline: `security=selinux selinux=1 enforcing=1`

Build integration:
- SELinux policy source (`.te`, `.fc`, `.if` files) checked into `images/selinux/`
- Policy compiled during rootfs build (`checkpolicy`, `semodule_package`)
- Filesystem labels applied during rootfs build (`setfiles`)
- `libselinux` installed in rootfs (required for label-aware tools)

**Rootfs binary/file stripping**:

The stock Debian rootfs contains hundreds of binaries the agent doesn't need and an attacker could abuse. We strip the rootfs to a minimal set during the Docker build.

Binaries to KEEP (allowlist):
```
# Shell and core utilities
/bin/bash /bin/sh
/usr/bin/env /usr/bin/cat /usr/bin/ls /usr/bin/cp /usr/bin/mv
/usr/bin/mkdir /usr/bin/rm /usr/bin/chmod /usr/bin/chown
/usr/bin/grep /usr/bin/sed /usr/bin/awk /usr/bin/sort
/usr/bin/head /usr/bin/tail /usr/bin/wc /usr/bin/tr
/usr/bin/find /usr/bin/xargs /usr/bin/tee
/usr/bin/echo /usr/bin/printf /usr/bin/test /usr/bin/expr
/usr/bin/date /usr/bin/sleep /usr/bin/id /usr/bin/whoami
/usr/bin/uname /usr/bin/hostname /usr/bin/dirname /usr/bin/basename
/usr/bin/readlink /usr/bin/realpath /usr/bin/stat
/usr/bin/diff /usr/bin/patch /usr/bin/touch
/usr/bin/tar /usr/bin/gzip /usr/bin/gunzip
/usr/bin/du /usr/bin/df /usr/bin/free

# Developer tools (needed by AI agents)
/usr/bin/git
/usr/bin/node /usr/bin/npm /usr/bin/npx
/usr/bin/python3 /usr/bin/pip3
/usr/bin/curl /usr/bin/wget

# Session management
/usr/bin/setsid /usr/bin/stty /usr/bin/tty

# Build tools (needed for native Python extensions like numpy)
/usr/bin/gcc /usr/bin/g++ /usr/bin/make /usr/bin/cc
/usr/bin/ld /usr/bin/as /usr/bin/ar

# Debug tools (agents need these for development)
/usr/bin/strace /usr/bin/ltrace /usr/bin/gdb /usr/bin/ldd

# Package managers (agents install project dependencies)
/usr/bin/pip3 /usr/bin/pip
/usr/bin/apt /usr/bin/apt-get /usr/bin/dpkg

# AI agent CLIs (installed globally)
claude, gemini (via npm global)
```

Binaries/files to REMOVE (blocklist):
```
# NOTE: gcc, make, pip, npm, strace, gdb are KEPT. Agents need compilers
# for native Python extensions, package managers for project deps, and
# debug tools for development. SELinux policy confines what they can
# write to (workspace + tmpfs only).

# Dangerous system tools
mount, umount, fdisk, mkfs.*, fsck.*, losetup
insmod, rmmod, modprobe, lsmod (redundant with CONFIG_MODULES=n but belt-and-suspenders)
iptables, ip, route, ifconfig, ss, netstat (agent doesn't manage networking)
su, sudo, chroot, unshare, nsenter (no privilege escalation tools)
dd (raw disk access)
nc, ncat, socat, nmap (network attack tools -- should not be in rootfs anyway)
crontab, at, batch (no scheduled execution)

# Setuid/setgid binaries (remove ALL setuid bits)
find / -perm /6000 -exec chmod ug-s {} \;

# Unnecessary directories
/usr/share/doc, /usr/share/man, /usr/share/info, /usr/share/locale (except C)
/usr/share/zoneinfo (except UTC)
/var/cache/apt, /var/lib/apt (no apt)
/usr/games
```

Build integration:
- A `strip-rootfs.sh` script runs as the final stage of `Dockerfile.kernel`
- Allowlist-based: start by removing everything in `/usr/bin`, `/usr/sbin`, `/sbin`, then copy back only allowed binaries
- Remove all setuid/setgid bits
- Remove all `.a` static libraries and `.h` header files
- Final rootfs size target: <200MB (vs ~500MB+ stock with dev tools)

**Migration path**:
1. Start from Debian's `defconfig` for arm64
2. Iteratively disable subsystems, boot-test after each change
3. When stable, set `CONFIG_MODULES=n` and rebuild
4. Verify all needed drivers are built-in (virtio_pci, virtio_blk, etc.)
5. Replace `Dockerfile.kernel` with `Dockerfile.kernel-custom`
6. `just build` produces the new kernel; everything else unchanged

**Tests**:

Kernel:
- Boot: custom kernel boots successfully, reaches capsem-init
- Boot: `/dev/vda` appears (virtio_blk built-in)
- Boot: `/dev/hvc0` works (virtio_console built-in)
- Boot: `random: crng init done` appears quickly (hw_random_virtio built-in)
- Security: `insmod /tmp/evil.ko` fails with "modules disabled" or similar
- Security: `/proc/modules` is empty or absent
- Security: `lsmod` shows nothing
- Security: no USB, HID, DRM messages in `dmesg`
- Size: `vmlinuz` < 5MB (vs ~30MB stock)
- Performance: boot time equal or faster than stock kernel

SELinux:
- Security: `getenforce` returns `Enforcing`
- Security: agent process runs as `capsem_agent_t` (`id -Z`)
- Security: `touch /usr/bin/evil` denied by SELinux (even as root)
- Security: `echo x > /etc/passwd` denied by SELinux
- Security: writing to `/workspace/` succeeds (allowed by policy)
- Security: writing to `/tmp/` succeeds (tmpfs, allowed)
- Security: `cat /proc/kcore` denied by SELinux
- Security: `cat /proc/kallsyms` denied by SELinux
- Security: `setenforce 0` denied (agent cannot disable SELinux)
- Integration: `pip install requests` succeeds (installs to workspace/tmpfs)
- Integration: `npm install` succeeds in workspace
- Integration: `gcc -o hello hello.c` succeeds in workspace
- Integration: Claude Code / Gemini CLI run normally under policy

Filesystem stripping:
- Security: no setuid/setgid binaries in rootfs (`find / -perm /6000` returns empty)
- Security: `su` not found, `sudo` not found, `chroot` not found
- Security: `mount` not found, `umount` not found
- Security: `dd` not found, `nc` not found, `nsenter` not found
- Security: no `.h` files outside workspace, no `.a` static libraries in system dirs
- Security: `/usr/share/doc` absent, `/usr/share/man` absent
- Size: rootfs < 200MB (vs ~500MB+ unstripped)
- Integration: `git`, `node`, `python3`, `gcc`, `pip`, `npm`, `curl`, `strace`, `gdb` all present and functional

**NOT included**: Secure Boot chain (kernel signature verification by the hypervisor). Apple's Virtualization.framework does not support this.

---

## Per-Execution Structured Logging (Future)

Each VM execution should produce a per-execution SQLite database stored in the app data directory. Schema TBD but must include: timestamps, event names, durations, and structured fields. This enables post-hoc analysis of boot performance, hash verification costs, and VM lifecycle timing. Currently deferred -- tracing spans with `FmtSpan::CLOSE` are the interim solution. Implementation should land alongside or after Milestone 8 (Session Management + Persistence) since that milestone introduces SQLite infrastructure via `rusqlite`.

---

## Verification Plan

**Per-milestone**:
1. `cargo test --workspace` - all unit + integration tests pass
2. `cargo clippy --workspace -- -D warnings` - no lint warnings
3. Manual smoke test per milestone deliverable
4. For milestones 4+: `pnpm test` (frontend) + `cargo tauri dev` (full app)

**End-to-end (after Milestone 11)**:
1. Launch signed app from /Applications
2. Configure Anthropic API key (stored in macOS Keychain)
3. Create Claude Code session: pick workspace, default network policy
4. Terminal opens, Claude Code starts, give it a coding task
5. Observe: API calls logged in real-time, tokens counted, cost estimated
6. Claude triggers MCP tool call -> approval dialog appears -> approve it
7. View stats dashboard: cumulative cost, call timeline
8. Stop session, quit app, relaunch, resume session -> workspace intact, history replayed
9. Create second Gemini session running simultaneously
10. Verify both sessions isolated (separate terminals, separate stats)
11. Try `curl https://evil.com` from inside VM -> blocked
12. Verify API keys not visible inside VM environment

---

## Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| VM framework | Apple Virtualization.framework via `objc2-virtualization` v0.3.2 | Native, fast, no QEMU dependency |
| Guest OS | **Debian bookworm-slim ARM64** | glibc for Python wheel compat (not Alpine/musl) |
| Guest init | **Guest-agent as PID 1** (no systemd) | <50ms boot, zero unnecessary services |
| Immutability | squashfs root + overlayfs (tmpfs or persistent disk upper) | True immutability, fresh or resumable |
| Host-guest comm | **vsock only** + VirtioFS | No NIC = air-gapped VM, zero bypass risk |
| IPC Streaming | **Tauri 2.0 Channels + Credit-based Backpressure** | Prevents high-volume streams (PTY, logs) from freezing the UI |
| Network control | **Fake-IP SNI router** (dummy0 + fake DNS + vsock bridge) | Solves no-NIC routing, zero DNS leaks |
| API proxying | **Layer 7 Native HTTP Proxy** (not OpenAI translator) | Preserves prompt caching, tool schemas, streaming |
| Terminal | **PTY over vsock** (not serial) | Proper resize, colors, cursor, SIGWINCH |
| MCP gateway | JSON-RPC proxy over vsock + Seatbelt sandbox | Full control + host MCP server confinement |
| Frontend | Tauri 2.0 + Svelte 5 (scaffolded from M1) | No async retrofit pain |
| Persistence | SQLite + persistent overlay disk (no VM snapshots) | VirtioFS blocks snapshots; cold boot is fast |
| Workspace paths | **macOS security-scoped bookmarks** (not string paths) | Survives app sandbox quit/resume cycle |
| Cache sharing | **OverlayFS on read-only VirtioFS** | npm/pip write .lock files; overlay catches writes |
| App quit | **Graceful shutdown interceptor** | Prevents ext4 corruption on Cmd+Q |
| API key storage | macOS Keychain via `security-framework` | Native OS secure storage |
| x86_64 compat | Rosetta 2 via `VZLinuxRosettaDirectoryShare` | Transparent Intel binary execution |
| Clock sync | `SyncTime` vsock message on resume | Prevents TLS cert failures |
| Kernel | **Custom minimal kernel** with `CONFIG_MODULES=n` (M12) | Eliminates kernel rootkits, shrinks attack surface by 90% |

---

## Rust Ecosystem Reference

| Crate / Project | Role | Notes |
|-----------------|------|-------|
| `objc2-virtualization` v0.3.2 | Virt.framework bindings | Core VM management |
| `objc2` + `objc2-foundation` + `block2` | ObjC interop | Completion handlers, NSURL, etc. |
| `tauri` v2 | App shell + IPC | Async setup hook for tokio |
| `axum` | AI gateway HTTP server | Routes for /v1/messages, /v1/gemini |
| `reqwest` | Upstream HTTP client | Streaming support for SSE |
| `tokio` | Async runtime | Everywhere |
| `rustls` / `tokio-rustls` | TLS for upstream connections | Gateway -> provider |
| `security-framework` | macOS Keychain | API key storage |
| `rusqlite` | SQLite | Session persistence, audit logs |
| `serde` + `serde_json` | Serialization | Protocol messages, API payloads, config |
| `tracing` + `tracing-subscriber` | Structured logging | Throughout |
| `anyhow` + `thiserror` | Error handling | anyhow for app, thiserror for libs |
| `async-stream` | SSE stream inspection | Token counting mid-stream |
| `xterm.js` | Terminal emulation | Frontend, PTY rendering |
| TensorZero / Traceloop Hub | Architecture reference | Gateway patterns, provider abstractions |

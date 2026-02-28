# Security

Capsem's primary purpose is to provide a secure sandbox for running AI agents. The core assumption is that the agent inside the VM is adversarial -- it may be compromised, hallucinating, or deliberately malicious. Every design decision flows from this.

## Threat Model: Rogue AI Agent

The principal threat is an AI agent (Claude Code, Gemini CLI, or any future agent) that goes rogue. This could happen because the agent is jailbroken, the model provider is compromised, the agent misinterprets instructions, or the agent is deliberately exploring its boundaries. We assume the agent has:

- Full root access inside the VM
- Ability to write and execute arbitrary code (compiled binaries, scripts, kernel modules)
- Knowledge of the sandbox architecture (Capsem is open source)
- Motivation to escape, persist, exfiltrate, or cause damage

### T1: VM Escape

**Threat**: Agent exploits a vulnerability in the hypervisor or virtio device emulation to execute code on the host.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Hypervisor bug in Virtualization.framework | Apple maintains the hypervisor; we rely on hardware-enforced Stage 2 page tables on Apple Silicon. No custom device emulation code. | Inherited from Apple |
| Malformed virtio device interaction | We use only Apple's built-in virtio backends (console, block, entropy). No custom device emulation. Attack surface is Apple's code. | Inherited from Apple |
| Exploiting serial console parsing | Serial console is raw byte passthrough (NSPipe). No parsing, no protocol, no structured data. Nothing to exploit on the host side. | Implemented |
| vsock protocol bugs | Length-prefixed framing (4-byte BE + MessagePack) with 256KB max frame size. Disjoint type system: host only deserializes `GuestToHost`, guest only deserializes `HostToGuest`. Cross-type decoding fails at the serde tag level (different tag namespaces). All deserialization via `rmp-serde` with tagged enums. | Implemented |

### T2: Persistence Across Sessions

**Threat**: Agent writes a backdoor that survives VM restart, so future sessions are compromised from boot.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Write to rootfs | Rootfs mounted read-only (`mount -o ro`). Agent cannot modify system binaries, cron, init scripts, or installed packages. | Implemented |
| Write to initrd/kernel | These are in the host .app bundle, outside the VM. The VM cannot access or modify them. BLAKE3 hashes verified at boot. | Implemented |
| Write to persistent overlay disk | Overlay covers `/root`, `/tmp`, `/run`, `/var/log`, `/var/tmp` only. Currently all tmpfs (wiped on reboot). Future persistent overlay (M8) will be per-session and deletable. | Implemented (tmpfs) |
| Modify kernel modules | Custom kernel compiled with `CONFIG_MODULES=n`. No module loading infrastructure exists. Even root cannot load `.ko` files. | Implemented |
| Write backdoor to /usr/bin or /etc | Read-only rootfs today. Future SELinux (M12) `capsem_agent_t` policy denies writes to system dirs even if rootfs were writable. | Implemented + Planned (M12) |
| Install persistence via package manager | Future rootfs stripping (M12) removes `su`, `sudo`, `chroot`. SELinux policy prevents writing outside workspace/tmpfs. Agent can `pip install` in workspace but not modify system. | Planned (M12) |
| Cronjob / systemd timer | No systemd in the VM. No cron daemon. PID 1 is capsem-init (our script), not a service manager. | Implemented |
| Modify capsem-init | capsem-init is inside the initrd, which is on the host, BLAKE3-verified. Unreachable from inside the VM. | Implemented |

### T3: Data Exfiltration

**Threat**: Agent steals source code, credentials, or other sensitive data from the workspace and sends it to an external server.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Direct network access | No real network interface exists in the VM. No real NIC drivers compiled into kernel (`CONFIG_NETDEVICES=n`). Only `dummy0` exists for synthetic routing via vsock-backed SNI proxy. | Implemented |
| DNS exfiltration | No real DNS. Fake DNS (dnsmasq) resolves everything to 10.0.0.1; no queries leave the VM. | Implemented |
| Covert channel via serial console | Serial goes to xterm.js in the UI. An agent could encode data in terminal output, but the user sees everything. No automated forwarding to external services. | Accepted risk (user-visible) |
| HTTPS to unauthorized domain | Host-side SNI proxy enforces domain allow-list (user.toml + corp.toml). AI provider domains blocked at SNI level. Everything else is default-deny. All decisions logged to per-session web.db. | Implemented |
| DNS tunneling | Fake DNS inside VM never contacts real DNS servers. Host does real resolution only for allowed domains. | Implemented |
| Plain HTTP to bypass SNI | No SNI in plain HTTP = connection rejected by host proxy. HTTPS-only enforcement. Only port 443 is redirected to the vsock proxy. | Implemented |
| File exfiltration via workspace | `capsem-fswatch` daemon monitors `/workspace` via fanotify, streaming real-time file events (create/edit/delete) to host over vsock:5005. Host has full visibility into what the agent writes. | Planned (M5) |
| Future gateway (M6): data hidden in API prompts | `on_model_call` PII engine scrubs sensitive data from prompts before forwarding upstream. `on_tool_response` scans tool output for secrets. All traffic logged to per-session audit.db. | Planned (M6) |
| Future VirtioFS (M4): read host files outside workspace | VirtioFS shares scoped to specific directories. No share = no access. Cache shares are read-only. | Planned (M4) |
| Timing / power side channels | Out of scope. Mitigated by Apple Silicon hardware, not by us. | Out of scope |

### T4: Resource Exhaustion / Denial of Service

**Threat**: Agent consumes excessive CPU, memory, disk, or other resources to make the host unusable.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| CPU spin | VM is configured with fixed vCPU count (currently 2). macOS scheduler limits VM to its allocated cores. | Implemented |
| Memory exhaustion | VM is configured with fixed RAM (currently 512MB). Cannot allocate beyond this. Host memory is not affected. | Implemented |
| Disk fill | Writable areas are tmpfs (RAM-backed, bounded by VM memory). No unbounded disk writes possible. Future persistent overlay (M8) will have disk quotas. | Implemented (tmpfs) |
| Fork bomb | Contained within VM's memory limit. PID namespace is per-VM. Host is unaffected. | Implemented |
| Serial console flood | Broadcast channel has bounded capacity (256 messages). Lagged receivers skip messages. No unbounded memory growth on host. | Implemented |

### T5: Credential Theft

**Threat**: Agent attempts to steal API keys, SSH keys, cloud credentials, or other secrets.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Read host ~/.ssh, ~/.aws, ~/.config | VM has no access to host filesystem. No VirtioFS share for these paths. | Implemented |
| Read API keys from environment | Dummy API keys in VM env (`sk-capsem-gateway`). Real keys stored in macOS Keychain, injected by host gateway (M6) into upstream HTTPS requests after traffic leaves VM. Keys never enter VM. | Planned (M6) |
| Read API keys from gateway traffic | Gateway uses plain HTTP inside VM (no TLS to inspect). Keys injected on the host side after the request leaves the VM. Agent sees requests with dummy keys only. | Planned (M6) |
| Exfiltrate keys via prompts to LLM | `on_model_call` PII engine (M6) scans outbound prompts for API key patterns and replaces with `[REDACTED-N]` tokens before forwarding upstream. `on_tool_response` scans tool output for secrets before returning to the LLM. | Planned (M6) |
| Keylog or ptrace other processes | Only the agent's own processes run in the VM. No other user sessions, no other agents (per-VM isolation). | Implemented |
| Future MCP (M7): remote MCP server leaks host secrets | Remote MCP tools route through host MCP gateway (vsock:5003). Credentials injected by gateway, never exposed to VM. Local MCP tools run inside VM sandbox (no host access). | Planned (M7) |

### T6: Tampering with Host Application

**Threat**: Agent attempts to modify the Capsem application itself to weaken future security controls.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Modify .app bundle | VM cannot access host filesystem. .app bundle is codesigned; macOS rejects modified bundles. | Implemented |
| Modify VM assets on disk | Assets are inside the .app bundle (codesigned) or in a separate directory. VM cannot reach host filesystem. BLAKE3 verified at boot. | Implemented |
| Exploit auto-updater | Updates are minisign-verified against a public key embedded in the binary. Attacker must compromise the signing key (GitHub Actions secret), not just the release. | Implemented |
| Supply chain: compromised dependency | Cargo.lock pins all versions. CI uses cargo-auditable for post-hoc vulnerability scanning. SBOM generated per release. | Implemented |

### T7: Inter-Session Attacks

**Threat**: An agent in one session attacks another concurrent session, or a compromised session poisons future sessions.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Cross-VM memory access | Hardware-enforced isolation. Each VM has its own Stage 2 page tables. | Inherited from Apple |
| Shared filesystem | No shared filesystems between sessions currently. Future: each session gets its own VirtioFS mounts and overlay disk (M8). | Implemented (no sharing) |
| Shared network | Per-VM network state: each VM gets its own domain policy, web.db, and SNI proxy connection handlers. | Implemented |
| Poisoned persistent overlay | Future overlay disks are per-session. Session deletion wipes the disk. Fresh sessions start clean. | Planned (M8) |

### T8: XSS via Terminal Output

**Threat**: Agent crafts terminal output containing malicious payloads that escape the xterm.js sandbox and execute in the Tauri WebView context, potentially gaining access to Tauri IPC commands.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| ANSI escape injection into WebView DOM | xterm.js renders inside a closed Shadow DOM web component. Terminal content is processed by xterm.js's parser, not injected as raw HTML. No `innerHTML` usage. | Implemented |
| Craft output that looks like HTML/JS and escapes xterm.js | xterm.js treats all input as terminal data, not markup. Characters like `<script>` are rendered as literal text in the terminal canvas/WebGL renderer. | Implemented |
| Exploit xterm.js vulnerability to execute JS | xterm.js is a widely-audited library. We pin versions in pnpm-lock.yaml. A bug here would be a 0-day in xterm.js itself. | Dependency risk |
| If JS executes in WebView: invoke Tauri IPC | Tauri 2.0 capability system restricts which commands the WebView can call. Only `serial_input` and `vm_status` are exposed. Even with XSS, attacker can only send keystrokes to their own VM. | Implemented |
| If JS executes in WebView: read host filesystem | Tauri's WebView has no filesystem access. `fs` plugin is not installed. No `shell:open`, no `dialog:open` without user interaction. | Implemented |
| Overlong escape sequences / terminal bomb | xterm.js has built-in limits on escape sequence parsing. The broadcast channel drops lagged messages. Terminal rendering is bounded by the WebGL viewport. | Implemented |

### T9: Malicious Packages

**Threat**: Agent uses `pip install`, `npm install`, or `apt-get` to download packages containing malware, reverse shells, or data exfiltration payloads.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| `pip install evil-package` (with post-install script) | Only allow-listed domains reachable. pypi.org is on the allow-list but exfiltration to non-listed domains fails. Post-install scripts run in the sandbox. | Implemented |
| Package pre-cached in rootfs image | Rootfs is built from a controlled Dockerfile with explicit package lists. No arbitrary packages. Build is reproducible from source. | Implemented |
| Download from pypi.org | pypi.org is on the domain allow-list (needed for legitimate installs). A malicious package's post-install script could run, but it cannot exfiltrate because only allow-listed domains are reachable. | Implemented |
| Malicious package opens reverse shell | No inbound connections possible (no real NIC). Outbound only to allow-listed domains via SNI proxy. Reverse shell to attacker's server fails unless attacker controls an allowed domain. | Implemented |
| Package exfiltrates via allowed domain (e.g., npmjs.org PUT) | Host-side gateway could inspect and log all traffic. For registry domains, restrict to GET only (read-only package downloads). | Planned (policy refinement) |
| npm/pip cache poisoning via VirtioFS | Cache shares are read-only VirtioFS with ephemeral overlayfs. Agent's writes go to tmpfs upper layer, wiped on reboot. Host cache is never modified. | Planned (M4) |
| Malicious package loads kernel module | Custom kernel compiled with `CONFIG_MODULES=n`. No dynamic module loading possible. | Implemented |
| Malicious package modifies system binaries | Rootfs is read-only. `/usr/bin`, `/usr/lib`, etc. are immutable. Future SELinux (M12) enforces this even if rootfs mount changes. | Implemented + Planned (M12) |
| Malicious package installs setuid binary | Future rootfs stripping (M12) removes all setuid bits at build time. `nosuid` mount option on tmpfs overlays. | Planned (M12) |
| Malicious package runs privilege escalation exploit | Future rootfs stripping (M12) removes `su`, `sudo`, `chroot`, `nsenter`, `unshare`. SELinux confines agent to `capsem_agent_t` regardless of UID. | Planned (M12) |

### T10: Rootfs Image Poisoning

**Threat**: An attacker (or compromised build system) tampers with the rootfs image to include backdoors that affect all future VM boots.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Modified rootfs.img on disk | BLAKE3 hash verified at boot. Hash is compiled into the binary. Tampered image = hash mismatch = VM refuses to boot. | Implemented |
| Compromised Dockerfile.kernel or build.py | Build scripts are checked into git. Changes are visible in code review. CI builds from a specific commit hash. | Implemented |
| Compromised base image (debian:bookworm-slim) | Docker image pulled by digest in CI for reproducibility. SBOM documents all packages. Debian packages are signed by Debian's archive key. | Partial (pin by digest in CI) |
| Backdoored Debian package in apt repo | Debian packages are GPG-signed. apt verifies signatures. A compromised mirror would need Debian's signing key. | Inherited from Debian |
| Supply chain attack on npm global install (Claude Code, Gemini CLI) | These are installed during `just build` from pinned versions. BLAKE3 hash of the resulting rootfs detects any change. | Implemented |
| Modified rootfs in .app bundle post-codesign | macOS codesigning covers all resources in the bundle. Modifying rootfs.img invalidates the signature. Gatekeeper blocks launch. | Implemented (release builds) |

### T11: Data Exfiltration via Network

**Threat**: An agent attempts to exfiltrate workspace data to an attacker-controlled server through the network channels.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| HTTPS POST to attacker's server | Domain allow-list at SNI proxy. Only explicitly allowed domains are reachable. Attacker's domain is not on the list. Default deny. All decisions logged to per-session web.db. | Implemented |
| Direct HTTPS to AI provider (bypass gateway) | AI provider domains (`api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`) are **blocked** at the SNI proxy level. All model traffic must flow through the audit gateway (vsock:5004). Agent cannot reach AI APIs without going through the 9-stage lifecycle. | Implemented (SNI block) / Planned (M6 gateway) |
| Exfiltrate via allowed domain (e.g., github.com API) | No GitHub auth token in VM (credentials injected by host MCP gateway). Without auth, public writes are limited. `on_tool_response` (M6) scans for secrets. | Implemented (no creds) / Planned (M7 gateway) |
| Exfiltrate via DNS TXT records | Fake DNS inside VM (dnsmasq `--address=/#/10.0.0.1`). No real DNS queries ever leave the VM. DNS tunneling is impossible. | Implemented |
| Encode data in TLS SNI field | SNI is the domain name only (max 253 bytes). Not a practical exfiltration channel. Host validates SNI against allow-list; random SNI values are rejected. | Implemented |
| Exfiltrate via HTTP headers to allowed API | AI gateway (M6) logs full request/response bodies to per-session audit.db. `on_model_call` PII engine scrubs sensitive data. `on_tool_response` scans for secrets. Anomalous requests visible in audit trail. | Planned (M6) |
| Steganography in legitimate API requests | Agent embeds data in prompt text sent to AI provider. Mitigation: all API traffic logged and inspectable in audit.db. PII engine catches common patterns. Not fully preventable without breaking functionality. | Accepted risk (logged + PII-scrubbed) |
| Exfiltrate via timing / packet size side channels | Impractical over vsock bridge. All traffic is proxied; timing is noisy. | Low risk |

### T12: Social Engineering the User

**Threat**: Agent manipulates terminal output or MCP approval dialogs to trick the user into taking dangerous actions on the host.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Fake system prompts ("run this command on host") | Terminal output is clearly scoped to the VM. No mechanism for the VM to execute host commands. | Implemented |
| Unicode / ANSI tricks to hide malicious output | xterm.js renders standard terminal escapes. User sees what the agent produces. Education, not technology, is the mitigation here. | Accepted risk |
| Future MCP (M7): trick user into approving dangerous tool call | `on_tool_call` lifecycle stage (M6) pauses the LLM stream and presents tool name + full arguments in Tauri approval UI. User must explicitly approve. Dangerous patterns (rm -rf, git push --force) auto-flagged by policy engine. Corporate policy can pre-block specific tools. | Planned (M6, M7) |

### T13: AI Traffic Audit Bypass

**Threat**: Agent attempts to bypass the AI audit gateway to communicate with LLM providers without inspection, logging, or policy enforcement.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Direct HTTPS to api.anthropic.com (bypass gateway) | AI provider domains are **blocked at the SNI proxy**. The SNI proxy rejects connections to `api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`. The only path to these providers will be through the audit gateway (vsock:5004). | Implemented (SNI block) / Planned (M6 gateway) |
| Override `*_BASE_URL` env var to point elsewhere | Agent can change env vars, but iptables REDIRECT rules are set by capsem-init (PID 1, before agent starts). Port 443 traffic always redirects to vsock proxy regardless of env vars. Even if the agent changes the URL, the traffic still hits the SNI proxy. | Implemented |
| Use a different port to reach AI provider | Only port 443 is redirected to the vsock proxy. Connections on other ports have no route to the outside (dummy0 interface with no real gateway). | Implemented |
| Compile custom HTTP client to avoid `*_BASE_URL` | Same enforcement: iptables + SNI proxy. The network path is enforced at the kernel level, not at the application level. No userspace workaround exists. | Implemented |
| Use a non-AI proxy/relay to tunnel to AI provider | The relay domain would need to be on the SNI allow-list. Default-deny policy means only explicitly allowed domains are reachable. Relay domain would be blocked. | Implemented |
| Modify iptables rules to remove REDIRECT | iptables rules set by capsem-init. Agent runs as root but can modify rules. Mitigation: future SELinux (M12) policy denies `CAP_NET_ADMIN`. Also, fswatch + audit trail would detect network config changes. | Accepted risk / Planned (M12) |

The enforcement chain is: **iptables REDIRECT (kernel) -> capsem-net-proxy (guest vsock bridge) -> SNI proxy (host) -> [future: audit gateway (host)]**. Three independent layers must be bypassed simultaneously.

---

## Security Controls (Implemented)

### Hardware-backed VM isolation

Apple's Virtualization.framework provides hardware-enforced isolation via Apple Silicon's hypervisor:

- Each VM runs in its own address space, enforced by the CPU's Stage 2 page tables.
- The VM cannot access host memory, host filesystems, or host processes.
- The only communication channels are those explicitly configured: serial console (and future vsock).

### Air-gapped networking (SNI proxy)

The VM has no real network interface (`CONFIG_NETDEVICES=n`). Instead, a synthetic air-gapped network stack provides controlled HTTPS access:

- **dummy0 interface** with IP `10.0.0.1/24` -- no real NIC, no real gateway.
- **Fake DNS** (dnsmasq): all domain lookups resolve to `10.0.0.1`. No DNS leaves the VM.
- **iptables REDIRECT**: port 443 traffic -> `capsem-net-proxy` on TCP `127.0.0.1:10443`.
- **capsem-net-proxy**: bridges each TCP connection to host via vsock:5002.
- **Host SNI proxy**: reads TLS ClientHello, extracts SNI hostname, evaluates against domain policy (allow/block lists from `user.toml` + `corp.toml`), bridges to real server if allowed, rejects otherwise.
- **Default deny**: unlisted domains are rejected. Only explicitly allowed domains are reachable.
- **AI provider blocking**: `api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com` blocked at SNI level.
- **Per-session telemetry**: every connection attempt logged to `~/.capsem/sessions/<vm_id>/web.db` (SQLite) with timestamp, domain, decision, and reason.
- **Corporate override**: `/etc/capsem/corp.toml` overrides user domain lists for enterprise lockdown.

### Vsock protocol security

The vsock control channel (port 5000) uses a disjoint type system to prevent message confusion:

- **`HostToGuest`** enum: commands the host sends to the guest (BootConfig, Resize, Exec, Ping, plus reserved variants).
- **`GuestToHost`** enum: messages the guest sends to the host (Ready, BootReady, ExecDone, Pong, plus reserved variants).
- **Disjoint serde tags**: `HostToGuest` and `GuestToHost` use different tag values. Decoding a `GuestToHost` message as `HostToGuest` (or vice versa) fails at the deserialization level.
- **Bounded frames**: 256KB max frame size, 4-byte big-endian length prefix. Oversized frames are rejected before reading the payload.
- **Type-safe API**: `encode_host_msg()`/`decode_host_msg()` for host-side, `encode_guest_msg()`/`decode_guest_msg()` for guest-side. Impossible to accidentally use the wrong encoder/decoder.

### Zero-trust guest binaries

The guest agent (`capsem-pty-agent`, `capsem-net-proxy`, and any future guest binaries) is treated as untrusted code. All security enforcement, state tracking, and protocol validation live exclusively on the host side. The guest has no state machine, no message validation, and no policy enforcement of its own.

**Rationale**: The agent runs inside the VM where a compromised or rogue AI has full root access. Any validation logic in the guest can be bypassed by an attacker who replaces or patches the binary, modifies its memory at runtime, or intercepts its vsock communication. Guest-side checks would provide a false sense of security while adding complexity.

**Design rules**:

- **No state machine on the guest**: The host tracks its own lifecycle state (`HostState`) and validates all inbound guest messages against it. The guest agent follows a simple sequential boot procedure with no enforcement.
- **No message validation on the guest**: The guest decodes and acts on whatever the host sends. The host is the trusted party; if the host sends a valid message, the guest should obey it. If a compromised guest sends an invalid message, the host drops it.
- **All policy enforcement on the host**: Domain allow/block lists, HTTP rules, file monitoring, and future MCP/AI gateway policies are evaluated on the host. The guest sees only the effect (connection allowed/denied), never the policy itself.
- **Guest binaries are deliberately simple**: Minimal logic, no configuration, no policy files. This reduces the attack surface inside the VM and makes the binaries easier to audit.
- **State machine lives in `capsem-core`**: The `HostState` enum, `StateMachine`, and message validation functions are in `capsem-core` (host-only crate), not in `capsem-proto` (shared crate). This enforces the architectural boundary at the dependency level -- the guest agent physically cannot import host state machine code.

### Boot handshake hardening

The boot handshake (vsock:5000) sends env vars and files to the guest. Multiple layers of validation prevent injection, crashes, and resource exhaustion:

- **Env var sanitization**: Keys containing `=` or NUL bytes are rejected (prevents `std::env::set_var` panics that would crash PID 1 and kernel panic). Values containing NUL bytes are also rejected.
- **Blocked env var list**: `LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`, `IFS`, `BASH_ENV`, `ENV`, `CDPATH`, `GLOBIGNORE`, `SHELLOPTS`, `BASHOPTS`, `PROMPT_COMMAND`, `PS4`, all `LD_*` prefixed vars, and `BASH_FUNC_*` exports are blocked. Case-sensitive (Linux env vars are case-sensitive).
- **Allocation caps**: Max 128 env vars, 64 files, 10MB total file data during boot handshake. Prevents unbounded memory allocation from a compromised or buggy config.
- **File path traversal protection**: Paths containing `..` are rejected, preventing writes outside intended directories.
- **Defense-in-depth**: The guest agent validates all env vars and file paths independently of the host. Even if a compromised host sends invalid data, the agent rejects it.

### AI agent permission bypass (yolo mode)

AI agents inside the VM (Claude Code, Gemini CLI) run with their built-in safety prompts disabled. Claude Code gets `{"defaultMode":"bypassPermissions"}` in `~/.claude/settings.json`. Gemini CLI gets `"approvalMode":"yolo"` in `~/.gemini/settings.json`. Both configs are injected as boot files via the settings registry.

**Why this is safe**: Capsem's entire purpose is to be the security boundary. The VM provides hardware-enforced isolation -- no real network interface, read-only rootfs, air-gapped HTTPS proxy with domain allow-lists, and per-session ephemeral storage. Every action the agent takes is contained within the sandbox. Adding a second layer of "are you sure?" prompts inside the VM would serve no purpose: the agent has full root access anyway, and the prompts would only slow down legitimate work without preventing anything that the VM sandbox doesn't already prevent.

**Why double-prompting is actively harmful**: AI agents that stop to ask permission for every file write and shell command are unusable for autonomous work -- the whole reason Capsem exists. Users would either disable the prompts manually (defeating the purpose) or avoid using the sandbox entirely. By defaulting to yolo mode, Capsem delivers on its value proposition: run AI agents at full speed with real security, not security theater.

**Corporate override**: Organizations can override these defaults via `/etc/capsem/corp.toml` (MDM-distributed). Setting `ai.anthropic.claude.settings_json` or `ai.google.gemini.settings_json` to custom values replaces the yolo configs with stricter policies if needed.

### Clock synchronization

The guest VM boots with epoch-0 clock. Without correct time, TLS cert validation, git, and other tools break. The host sends the current time in `BootConfig { epoch_secs }` during the vsock boot handshake, and the guest agent sets the system clock via `clock_settime(CLOCK_REALTIME)` before forking bash.

- Clock is set **before** any user-facing process starts.
- Requires `CAP_SYS_TIME` (satisfied: agent runs as root, launched by capsem-init PID 1).
- If `clock_settime` fails (EPERM), a warning is logged but boot continues with incorrect time.

### Read-only rootfs

The rootfs is mounted read-only. Writable areas are tmpfs overlays wiped on every reboot:

- `/root` -- tmpfs
- `/tmp` -- tmpfs
- `/run` -- tmpfs
- `/var/log` -- tmpfs
- `/var/tmp` -- tmpfs

System binaries, libraries, and configuration are immutable.

### Boot asset integrity (BLAKE3)

When the application is compiled, `build.rs` reads `B3SUMS` and embeds the expected BLAKE3 hashes of `vmlinuz`, `initrd.img`, and `rootfs.img` as compile-time constants:

```
B3SUMS -> build.rs -> VMLINUZ_HASH, INITRD_HASH, ROOTFS_HASH
```

At runtime, `capsem-core` computes the BLAKE3 hash of each file before loading it into the VM. If any hash does not match, the VM refuses to boot. BLAKE3 is used instead of SHA-256 for performance: hashing the 2GB rootfs takes ~40ms with BLAKE3 vs ~80s with unoptimized SHA-256.

This ensures:
- Tampered assets are detected before execution.
- The hashes are baked into the binary at compile time, not read from a file at runtime.
- An attacker cannot replace both the asset and its hash without modifying the binary itself.

### Guest binary hardening

All host-controlled binaries deployed inside the VM follow strict hardening rules:

**Read-only permissions (chmod 555)**: Every guest binary (`capsem-pty-agent`, `capsem-net-proxy`, and future binaries like `capsem-fswatch`) is deployed with `r-xr-xr-x` permissions. No write bit. This applies to both deployment paths:

- **Rootfs path** (`/usr/local/bin/`): Set to `chmod 555` in `Dockerfile.rootfs`. The rootfs itself is mounted read-only, providing a second layer of protection.
- **Initrd override path** (`/run/`): When `just repack` bundles a binary into the initrd, `capsem-init` copies it to `/newroot/run/` (a tmpfs) at boot with `chmod 555`. The tmpfs is writable by root, but the 555 permissions prevent casual overwrites. Future SELinux policy (M12) will enforce this at the MAC level.

**Read-only rootfs backing**: The rootfs ext4 image is mounted with `mount -o ro`. Even if a binary had write permissions, the filesystem would reject modifications. The combination of read-only mount + read-only permissions provides defense in depth.

**Integrity verification**: Boot asset hashes (BLAKE3) are compiled into the host binary. The initrd (which contains the agent when repacked) is hashed along with the kernel and rootfs. Tampering with any asset causes a hash mismatch and the VM refuses to boot.

**Smoke test verification**: The in-VM `capsem-test` script (`images/test-vm.sh`) verifies at runtime that:
- Guest binaries are not writable (`! test -w`)
- The rootfs is read-only (writes to `/usr` fail)
- Writable tmpfs areas are correctly scoped

**Guidelines for adding new guest binaries**:

1. Set `chmod 555` in `Dockerfile.rootfs` (rootfs path)
2. Add the binary to `just repack` with cross-compile + copy + `chmod 555`
3. Add initrd-override logic in `capsem-init` (check `/binary` before rootfs path, copy to `/newroot/run/` with `chmod 555`)
4. Add a `! test -w` check in `images/test-vm.sh` for the binary path
5. Update the "Currently repacked binaries" list in `CLAUDE.md`
6. Document the binary's purpose in this section

### Policy override security

Capsem uses a two-tier settings system where corporate policy always overrides user preferences:

- **User settings** (`~/.capsem/user.toml`): editable by the user. Only stores overrides from registry defaults.
- **Corporate settings** (`/etc/capsem/corp.toml`): read-only, MDM-distributed. Any setting specified here cannot be changed by the user.

**Enforcement rules:**

1. **Per-key override**: Corp settings override user settings per setting ID, not per category. If corp sets `ai.anthropic.allow = false`, the user cannot enable it, but other AI providers are unaffected unless also locked.
2. **Write isolation**: `can_write_corp_settings()` always returns false. The application never writes to `/etc/capsem/corp.toml`. Only user.toml is writable.
3. **Corp-locked indicator**: Each resolved setting carries a `corp_locked` flag. The UI shows locked settings as read-only with a lock icon.
4. **No expansion**: A user cannot expand permissions beyond what corp allows. If corp blocks a domain toggle, the user cannot enable it. If corp sets `network.default_action = "deny"`, the user cannot change it to `"allow"`.
5. **Dynamic settings**: Corp can also lock dynamic settings like `guest.env.*` to enforce specific environment variable values.

**Audit trail**: Each setting entry includes a `modified` timestamp. The `source` field (`default`, `user`, `corp`) in resolved settings makes it clear which tier set each value.

### Telemetry header sanitization

The MITM proxy logs every HTTP request/response to per-session SQLite (`web.db`). HTTP headers are sanitized before storage to prevent credential leakage:

- **Allowlisted headers** are stored verbatim: `accept`, `content-encoding`, `content-length`, `content-type`, `date`, `host`, `server`, `transfer-encoding`, `user-agent`.
- **All other headers** (including `authorization`, `x-api-key`, `x-goog-api-key`, `cookie`, `set-cookie`, and any custom headers) keep their name but the value is replaced with `hash:<12-char-hex>` -- the first 6 bytes of the BLAKE3 digest of the raw value.

This design:

1. **Prevents credential leakage**: API keys, bearer tokens, and session cookies never reach the database. Even if the SQLite file is exfiltrated or shared for debugging, no secrets are exposed.
2. **Preserves header presence**: Analysts can see which headers were sent (e.g., "this request included an `x-api-key` header") without seeing the value.
3. **Enables correlation**: The hash is deterministic -- the same API key always produces the same hash. This allows grouping requests by key identity without exposing the key itself.
4. **Minimal allowlist approach**: The allowlist is deliberately small and contains only structural/metadata headers that carry no authentication material. New headers default to hashed, not verbatim. Adding a header to the allowlist requires a code change and review.

Implementation: `format_headers()` in `crates/capsem-core/src/net/mitm_proxy.rs`. Tests verify that allowlisted headers pass through, sensitive headers are hashed, hashing is deterministic, and different values produce different hashes.

### No systemd, no services

The VM runs capsem-init as PID 1. There is no systemd, no cron, no sshd, no service manager. The only processes are those explicitly started by capsem-init (bash, and whatever the user/agent runs). No background services, no listening ports, no scheduled tasks.

### Hardened custom kernel

The VM runs a custom-compiled Linux 6.6 LTS kernel (~7MB vs ~30MB stock Debian) built from source with a hardened defconfig (`images/defconfig`). The kernel is compiled with `CONFIG_MODULES=n` -- there is no module loading infrastructure at all.

**Only the following drivers are compiled in (built-in, not modules):**

| Driver | Purpose |
|--------|---------|
| `virtio_pci` | PCI transport for virtio devices |
| `virtio_console` | Serial console (hvc0) |
| `virtio_blk` | Block device (rootfs on /dev/vda) |
| `hw_random_virtio` | Hardware entropy from host |
| `pl011` | ARM UART (early console) |
| `ext4` | Root filesystem |

**Exploit mitigations enabled:**

| Config | Protection |
|--------|-----------|
| `CONFIG_RANDOMIZE_BASE=y` | KASLR -- randomizes kernel base address each boot |
| `CONFIG_STACKPROTECTOR_STRONG=y` | Stack canaries on all functions with arrays or address-taken locals |
| `CONFIG_FORTIFY_SOURCE=y` | Compile-time and runtime buffer overflow detection |
| `CONFIG_STRICT_KERNEL_RWX=y` | Kernel code pages are not writable; data pages are not executable |
| `CONFIG_VMAP_STACK=y` | Guard pages around kernel stacks detect overflow |
| `CONFIG_HARDEN_BRANCH_PREDICTOR=y` | Mitigates Spectre-v2 on ARM64 |
| `CONFIG_SECURITY_DMESG_RESTRICT=y` | Unprivileged users cannot read kernel log |

**Attack surface eliminated:**

| Config | What it removes |
|--------|----------------|
| `CONFIG_MODULES=n` | No loadable kernel modules -- root cannot install rootkits via `.ko` files |
| `CONFIG_DEVMEM=n` | No `/dev/mem` -- root cannot read/write physical memory |
| `CONFIG_DEVPORT=n` | No `/dev/port` -- no I/O port access |
| `CONFIG_COMPAT=n` | No 32-bit syscall layer -- eliminates legacy compat attack surface |
| `CONFIG_INET=y` | IP stack enabled for synthetic networking (dummy0 + iptables REDIRECT to vsock-backed SNI proxy). No real NIC drivers (`CONFIG_NETDEVICES=n`). |
| `CONFIG_IO_URING=n` | No io_uring -- eliminates a historically high-CVE subsystem |
| `CONFIG_BPF_SYSCALL=n` | No eBPF -- eliminates JIT-based attack surface |
| `CONFIG_USERFAULTFD=n` | No userfaultfd -- commonly used in race condition exploits |
| `CONFIG_KEXEC=n` | No kexec -- cannot hot-swap the running kernel |
| `CONFIG_MAGIC_SYSRQ=n` | No SysRq -- cannot trigger kernel debug actions |
| `CONFIG_USB_SUPPORT=n` | No USB stack |
| `CONFIG_DRM=n` | No GPU/display |
| `CONFIG_SOUND=n` | No audio |
| `CONFIG_BT=n` | No Bluetooth |
| `CONFIG_WLAN=n` | No WiFi |
| `CONFIG_SCSI=n` | No SCSI |
| `CONFIG_KALLSYMS=n` | No kernel symbol table exposed to userspace |
| `CONFIG_DEBUG_FS=n` | No debugfs mount |

Unix domain sockets (`CONFIG_UNIX=y`) are kept because node, python, and git depend on them. IP networking is enabled (`CONFIG_INET=y`) for the synthetic network stack (dummy0 + iptables REDIRECT to vsock), but `CONFIG_NETDEVICES=n` ensures no real NIC drivers exist. The kernel can route IP packets through the dummy interface to vsock bridges, but has no driver to attach to any real network.

The kernel source is pinned to a specific LTS version, downloaded from kernel.org, and built reproducibly in a container. Trade-off: we own kernel patching. CVE response requires a version bump in `images/Dockerfile.kernel` and a rebuild.

### Application signing

The .app bundle is codesigned with the virtualization entitlement. In release builds, codesigned with a Developer ID certificate and notarized with Apple. macOS verifies the signature before allowing the app to run.

### Entitlements (minimal)

| Entitlement | Purpose |
|-------------|---------|
| `com.apple.security.virtualization` | Required for Virtualization.framework |
| `com.apple.security.network.client` | Required for auto-updater to check GitHub Releases |

No camera, microphone, contacts, location, or other sensitive resource access.

### Auto-update signature verification

Updates are signed with minisign. The public key is embedded in the binary at compile time. An attacker who compromises a GitHub Release but not the signing key cannot deliver a malicious update.

### Supply chain protections

- All Rust dependencies pinned in `Cargo.lock`.
- Frontend dependencies pinned in `pnpm-lock.yaml` with `--frozen-lockfile` in CI.
- SLSA Build Provenance (Level 2) attestation per release.
- SPDX SBOM generated and attested alongside each release.
- `cargo auditable build` embeds dependency manifest for post-hoc vulnerability scanning.

---

## Planned Security Controls

### Milestone 4: VirtioFS workspace sharing

- VirtioFS shares scoped to specific directories (workspace, caches).
- Cache shares mounted read-only with ephemeral overlayfs to catch writes.
- No share for ~/.ssh, ~/.aws, ~/.config, or any path outside the workspace.

### Milestone 5: Network boundaries & real-time telemetry (partially implemented)

**Implemented:**
- Kernel IP stack enabled (`CONFIG_INET=y`) but no real NIC drivers (`CONFIG_NETDEVICES=n`).
- Synthetic network: `dummy0` + fake DNS (dnsmasq `--address=/#/10.0.0.1`) + iptables REDIRECT.
- Host-side SNI proxy extracts TLS SNI and enforces domain allow-list (default-deny).
- AI provider domains blocked at SNI proxy.
- Configurable domain policy via `~/.capsem/user.toml` and `/etc/capsem/corp.toml`.
- Per-session `web.db` logging all HTTPS connection decisions.
- Zero DNS leaks: fake DNS inside VM, real resolution on host only.
- vsock port 5002 for SNI proxy connections.

**Remaining:**
- `capsem-fswatch` daemon for `/workspace` fanotify monitoring (vsock:5005).
- vsock ports 5003 (MCP gateway), 5004 (AI gateway), 5005 (file telemetry).

### Milestone 6: Active AI audit gateway

- Active Layer 7 gateway (Axum on vsock:5004) -- not a passive proxy.
- 9-stage event lifecycle: `on_agent_launch`, `on_file_create`, `on_file_edit`, `on_file_delete`, `on_model_call`, `on_model_response`, `on_tool_call`, `on_tool_response`, `on_agent_end`.
- PII engine scrubs sensitive data (emails, API keys, phone numbers) from prompts before forwarding upstream (`on_model_call`), rehydrates on response (`on_model_response`).
- Secret detection scans tool output before returning to LLM (`on_tool_response`).
- `on_tool_call` pauses the LLM SSE stream, presents tool name + arguments for user/policy approval. Denied calls inject synthetic error response.
- API keys in macOS Keychain, never inside the VM. Dummy keys in agent env vars.
- Real keys injected per-provider: `x-api-key` (Anthropic), `?key=` (Gemini), `Authorization: Bearer` (OpenAI).
- Policy engine evaluates `policy.toml` (local) or remote corporate webhook (gRPC/HTTPS).
- Full audit trail: every request/response logged to per-session audit.db with zstd-compressed payloads.

### Milestone 7: Hybrid MCP architecture

- Local MCP tools (bash, filesystem, git) run inside the VM -- the VM IS the sandbox.
- Remote/enterprise MCP tools route through host MCP gateway (vsock:5003) with credential injection from macOS Keychain.
- Agent MCP configs rewritten at boot: local tools stay as `stdio`, remote tools rewritten to `streamableHttp` URLs via host gateway.
- Host controls local tools via AI gateway's `on_tool_call` lifecycle stage (stream pause + approval).
- Remote tool credentials (GITHUB_TOKEN, etc.) injected by host gateway, never present in VM.
- Full audit log of all MCP calls (both local via AI gateway, remote via MCP gateway).

### Milestone 8: State, audit, and observability

- Per-session SQLite databases (`~/.capsem/sessions/sess_<id>/audit.db`), not a monolithic DB.
- Raw telemetry and LLM payloads zstd-compressed before SQLite BLOB insertion.
- Each session self-contained and independently deletable.
- OverlayFS config write-back: agent config changes captured in tmpfs upperdir, presented in Tauri UI for selective save-back on session end.
- Per-session overlay disk for workspace persistence across resume.
- Graceful shutdown: sync + unmount + ACPI poweroff prevents corruption.
- Prometheus metrics (`localhost:9090/metrics`): tool executions, model calls, policy denials, latency histograms.
- OpenTelemetry (OTLP): sanitized + compressed session export to SIEM when mandated by corporate policy.
- Corporate policy (`/etc/capsem/policy.toml`): distributable via MDM, controls domain lists, gateway enforcement, model restrictions, MCP tool policies, session limits, audit export.

### Milestone 12: SELinux, filesystem stripping

Custom kernel is implemented (see "Hardened custom kernel" above).

SELinux mandatory access control:
- SELinux in enforcing mode. Policy baked into read-only rootfs; agent cannot modify it.
- Agent confined to `capsem_agent_t` domain: can read/write workspace and tmpfs, cannot write to system dirs, cannot access raw devices, cannot disable SELinux.
- Even root inside the VM is constrained by MAC policy. Root != omnipotent.

Rootfs binary stripping:
- Remove all setuid/setgid bits from every binary.
- Remove dangerous tools: `su`, `sudo`, `chroot`, `mount`, `dd`, `nc`, `nsenter`, `unshare`, etc.
- Keep tools agents need: `gcc`, `make`, `pip`, `npm`, `git`, `node`, `python3`, `curl`, `strace`, `gdb`.
- Remove docs, man pages, locales, headers, static libraries.
- Rootfs size target: <200MB (vs ~500MB+ unstripped).

---

## Corporate Security Profile

For enterprise deployments, Capsem supports a system-wide policy file distributable via MDM (Mobile Device Management). This enables IT/security teams to enforce organizational controls without modifying the application.

**Policy file**: `/etc/capsem/policy.toml` (system-wide, read-only to non-root)

**Configurable controls**:

| Setting | Description | Example |
|---------|-------------|---------|
| `network.allowed_domains` | Extend the default domain allow-list | `["*.internal.corp.com"]` |
| `network.blocked_domains` | Block additional domains | `["*.competitor.com"]` |
| `gateway.enforce` | Prevent users from disabling the audit gateway | `true` |
| `gateway.approved_models` | Restrict to specific model IDs | `["claude-opus-4-6", "gemini-2.5-pro"]` |
| `mcp.blocked_tools` | Globally block specific MCP tools | `["shell_exec", "file_delete"]` |
| `mcp.approval_required` | Tools that always require user approval | `["git_push", "npm_publish"]` |
| `session.max_duration_hours` | Auto-terminate sessions after N hours | `8` |
| `session.max_cost_usd` | Cost ceiling per session | `50.0` |
| `session.max_concurrent` | Max concurrent sessions per user | `3` |
| `audit.otlp_endpoint` | SIEM export endpoint (OTLP/gRPC) | `"https://siem.corp.com:4317"` |
| `audit.retention_days` | Minimum local audit retention | `90` |
| `audit.export_on_end` | Push session audit on `on_agent_end` | `true` |
| `pii.custom_patterns` | Additional regex patterns for PII scrubbing | `["CORP-\\d{6}"]` |

**Enforcement**: When `/etc/capsem/policy.toml` exists, its settings override user preferences. The gateway cannot be disabled, domain lists are merged (corporate additions cannot be removed by user), and audit export is mandatory. The policy file is read at application startup and cached; changes require app restart.

**MDM distribution**: Deploy via any MDM that supports custom configuration profiles (Jamf, Kandji, Mosyle, etc.). The policy file is a standard TOML file placed at a fixed path.

---

## Verification

### For users

```sh
# Verify macOS code signature
codesign --verify --deep --strict /Applications/Capsem.app

# Verify Gatekeeper approval (notarized builds only)
spctl --assess --type execute /Applications/Capsem.app

# Verify GitHub build attestation
gh attestation verify Capsem.dmg --repo google/capsem

# Scan for known vulnerabilities in dependencies
cargo audit bin /Applications/Capsem.app/Contents/MacOS/capsem
```

### For developers

```sh
# Run all tests
cargo test --workspace

# Check for lint issues
cargo clippy --workspace -- -D warnings

# Verify asset hashes match
b3sum --check assets/B3SUMS
```

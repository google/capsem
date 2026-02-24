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
| Future vsock protocol bugs | Length-prefixed framing with bounded message sizes. All deserialization uses serde with deny_unknown_fields. | Planned (M2) |

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
| Direct network access | No network interface exists in the VM. No NIC, no IP stack, no default route. `curl`, `ping`, `wget` all fail with ENETUNREACH. | Implemented |
| DNS exfiltration | No real DNS. Future fake DNS (M5) resolves everything to a single IP; no queries leave the VM. | Implemented (no DNS at all currently) |
| Covert channel via serial console | Serial goes to xterm.js in the UI. An agent could encode data in terminal output, but the user sees everything. No automated forwarding to external services. | Accepted risk (user-visible) |
| Future network (M5): HTTPS to unauthorized domain | Host-side SNI proxy enforces domain allow-list. Only explicitly allowed domains (e.g., api.anthropic.com) are reachable. Everything else is dropped. | Planned (M5) |
| Future network (M5): DNS tunneling | Fake DNS inside VM never contacts real DNS servers. Host does real resolution only for allowed domains. | Planned (M5) |
| Future network (M5): plain HTTP to bypass SNI | No SNI in plain HTTP = connection rejected by host proxy. HTTPS-only enforcement. | Planned (M5) |
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
| Read API keys from environment | API keys are not in the VM environment. Currently not needed (no network). Future: host-side gateway injects keys into outbound requests (M6). Keys never enter VM. | Planned (M6) |
| Read API keys from gateway traffic | Future gateway uses plain HTTP inside VM (no TLS to inspect). Keys injected on the host side after the request leaves the VM. Agent sees requests without keys. | Planned (M6) |
| Keylog or ptrace other processes | Only the agent's own processes run in the VM. No other user sessions, no other agents (per-VM isolation). | Implemented |
| Future MCP (M7): MCP server reads host secrets | MCP servers run in macOS Seatbelt sandbox. Cannot read outside workspace directory. ~/.ssh, ~/.aws explicitly denied. | Planned (M7) |

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
| Shared network | No network currently. Future: per-session vsock CIDs and per-session network policies (M5, M8). | Planned |
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
| `pip install evil-package` (with post-install script) | No network interface. `pip install` fails with connection error. Package cannot be downloaded. | Implemented |
| Package pre-cached in rootfs image | Rootfs is built from a controlled Dockerfile with explicit package lists. No arbitrary packages. Build is reproducible from source. | Implemented |
| Future network (M5): download from pypi.org | pypi.org is on the domain allow-list (needed for legitimate installs). A malicious package's post-install script could run, but it cannot exfiltrate because only allow-listed domains are reachable. | Planned (M5) |
| Future network (M5): malicious package opens reverse shell | No inbound connections possible (no real NIC). Outbound only to allow-listed domains via SNI proxy. Reverse shell to attacker's server fails unless attacker controls an allowed domain. | Planned (M5) |
| Future network (M5): package exfiltrates via allowed domain (e.g., npmjs.org PUT) | Host-side gateway could inspect and log all traffic. For registry domains, restrict to GET only (read-only package downloads). | Planned (M5, policy refinement) |
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

### T11: Data Exfiltration via Network (Future M5+)

**Threat**: Once network access is added (M5), an agent attempts to exfiltrate workspace data to an attacker-controlled server through the allowed network channels.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| HTTPS POST to attacker's server | Domain allow-list. Only explicitly allowed domains are reachable. Attacker's domain is not on the list. | Planned (M5) |
| Exfiltrate via allowed domain (e.g., github.com API) | Agent could theoretically push data to a public GitHub repo. Mitigation: API key isolation means no GitHub auth token in VM. Without auth, public writes are limited. | Planned (M5, M6) |
| Exfiltrate via DNS TXT records | Fake DNS inside VM. No real DNS queries ever leave the VM. DNS tunneling is impossible. | Planned (M5) |
| Encode data in TLS SNI field | SNI is the domain name only (max 253 bytes). Not a practical exfiltration channel. Host validates SNI against allow-list; random SNI values are rejected. | Planned (M5) |
| Exfiltrate via HTTP headers to allowed API | Host-side API gateway (M6) logs full request/response bodies. Anomalous requests visible in audit log. Gateway could enforce request schemas. | Planned (M6) |
| Steganography in legitimate API requests | Agent embeds data in prompt text sent to AI provider. Mitigation: all API traffic is logged and inspectable. User can review. Not practically preventable without breaking functionality. | Accepted risk (logged) |
| Exfiltrate via timing / packet size side channels | Impractical over vsock bridge. All traffic is proxied; timing is noisy. | Low risk |

### T12: Social Engineering the User

**Threat**: Agent manipulates terminal output or MCP approval dialogs to trick the user into taking dangerous actions on the host.

| Vector | Mitigation | Status |
|--------|-----------|--------|
| Fake system prompts ("run this command on host") | Terminal output is clearly scoped to the VM. No mechanism for the VM to execute host commands. | Implemented |
| Unicode / ANSI tricks to hide malicious output | xterm.js renders standard terminal escapes. User sees what the agent produces. Education, not technology, is the mitigation here. | Accepted risk |
| Future MCP (M7): trick user into approving dangerous tool call | MCP approval UI shows tool name, full arguments, and target. User must explicitly approve. Dangerous patterns (rm -rf, git push --force) flagged. | Planned (M7) |

---

## Security Controls (Implemented)

### Hardware-backed VM isolation

Apple's Virtualization.framework provides hardware-enforced isolation via Apple Silicon's hypervisor:

- Each VM runs in its own address space, enforced by the CPU's Stage 2 page tables.
- The VM cannot access host memory, host filesystems, or host processes.
- The only communication channels are those explicitly configured: serial console (and future vsock).

### No network interface

The VM has no `VZNetworkDeviceAttachment`. There is physically no network interface inside the guest:

- No IP connectivity of any kind.
- No DNS resolution.
- `ping`, `curl`, `wget` all fail with "Network is unreachable".
- There is no software configuration that can bypass this -- the network device does not exist.

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
| `CONFIG_INET=n` | No IP networking stack -- even if a NIC were injected, no IP |
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

Unix domain sockets (`CONFIG_UNIX=y`) are kept because node, python, and git depend on them. IP networking (`CONFIG_INET=n`) is disabled at the kernel level -- this is a deeper defense than just not attaching a NIC, because the kernel has no IP stack code at all.

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

### Milestone 5: Air-gapped network with SNI filtering

- Still no real NIC. Synthetic network via dummy0 + fake DNS + vsock bridge.
- Host-side proxy extracts TLS SNI and enforces domain allow-list.
- HTTPS-only: plain HTTP rejected (no SNI to validate).
- Zero DNS leaks: fake DNS inside VM, real resolution on host only.
- No UDP forwarding, no ICMP, no raw sockets.

### Milestone 6: API key isolation

- API keys in macOS Keychain, never inside the VM.
- Host-side Axum gateway injects keys into outbound API requests.
- Agent sees plain HTTP requests without keys; host adds keys and opens HTTPS upstream.

### Milestone 7: MCP sandboxing

- Host-side MCP servers in macOS Seatbelt (`sandbox-exec`) profiles.
- Confined to workspace directory. Cannot read ~/.ssh, ~/.aws, ~/.config.
- Policy engine: allow, block, or require user approval per tool call.
- Full audit log of all MCP calls.

### Milestone 8: Session isolation

- Per-session overlay disk, vsock CID, and VirtioFS mounts.
- No cross-session data leakage.
- Session deletion wipes overlay disk.
- Graceful shutdown: sync + unmount + ACPI poweroff prevents corruption.

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

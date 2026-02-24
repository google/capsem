# Security

Capsem's primary purpose is to provide a secure sandbox for running AI agents. This document describes the security model, isolation guarantees, and supply chain protections.

## Threat Model

### What we defend against

- **AI agent escape**: An AI agent running inside the VM attempts to access the host filesystem, network, or other system resources outside the sandbox.
- **Malicious code execution**: Code generated or downloaded by an AI agent that attempts to exfiltrate data, install malware, or attack other systems.
- **Supply chain attacks**: Tampered VM images, compromised dependencies, or modified application binaries.
- **Network exfiltration**: An agent attempts to send data to unauthorized endpoints.

### What is out of scope

- Physical access attacks on the host machine.
- Attacks against Apple's Virtualization.framework itself (we rely on its correctness).
- Side-channel attacks between VMs (mitigated by Apple Silicon hardware, not by us).

## VM Isolation

### Hardware-backed isolation

Capsem uses Apple's Virtualization.framework, which provides hardware-enforced isolation via Apple Silicon's hypervisor:

- Each VM runs in its own address space, enforced by the CPU's Stage 2 page tables.
- The VM cannot access host memory, host filesystems, or host processes.
- The only communication channels are those explicitly configured: serial console and (in future) vsock.

### No network interface

The current VM has no `VZNetworkDeviceAttachment`. There is physically no network interface inside the guest. This means:

- No IP connectivity of any kind.
- No DNS resolution.
- `ping`, `curl`, `wget` all fail with "Network is unreachable".
- There is no software configuration that can bypass this -- the network device simply does not exist.

### Planned network architecture (Milestone 5)

When network access is added, it will use an air-gapped architecture:

- Still no real NIC in the VM.
- A `dummy0` interface with a fake IP provides a synthetic network stack.
- A fake DNS server resolves all domains to a single IP.
- All TCP is redirected via iptables to a vsock bridge.
- The host-side proxy extracts the TLS SNI field and enforces a domain allow-list.
- Only HTTPS traffic is forwarded. Plain HTTP has no SNI and is rejected.
- Zero DNS leaks: DNS never leaves the VM.

### Filesystem isolation

- The VM boots from a read-only kernel and initrd embedded in the .app bundle.
- The rootfs is a small ext4 image, also embedded in the bundle.
- The VM has no access to the host filesystem (no VirtioFS shares in Milestone 1).
- Future VirtioFS shares (Milestone 4) will be scoped to specific workspace directories with read-only caches protected by overlayfs.

## Boot Asset Integrity

### Build-time hash embedding

When the application is compiled, `build.rs` reads `SHA256SUMS` and embeds the expected hashes of `vmlinuz`, `initrd.img`, and `rootfs.img` as compile-time constants:

```
SHA256SUMS -> build.rs -> VMLINUZ_HASH, INITRD_HASH, ROOTFS_HASH
```

At runtime, `capsem-core` computes the SHA-256 hash of each file before loading it into the VM. If any hash does not match, the VM refuses to boot.

This ensures that:
- Tampered assets are detected before execution.
- The hashes are baked into the binary at compile time, not read from a file at runtime.
- An attacker cannot replace both the asset and its hash without modifying the binary itself.

### Asset bundling

In release builds, all VM assets are bundled inside `Capsem.app/Contents/Resources/`. The .app bundle is codesigned, so macOS verifies the bundle's integrity on launch. Modifying any file inside the .app invalidates the code signature.

## Application Signing

### Development builds

`make run` signs the debug binary with an ad-hoc signature and the virtualization entitlement:

```
codesign --sign - --entitlements entitlements.plist --force target/debug/capsem
```

### Release builds

`make release-sign` signs the entire .app bundle:

```
codesign --sign - --entitlements entitlements.plist --force --deep Capsem.app
```

### CI builds (planned)

CI builds use a Developer ID certificate for distribution. The app is notarized with Apple, which means:

- macOS verifies the signature before allowing the app to run.
- Apple's notarization service scans the binary for known malware.
- `spctl --assess` validates the app passes Gatekeeper.

### Entitlements

The app requests two entitlements:

| Entitlement | Purpose |
|-------------|---------|
| `com.apple.security.virtualization` | Required to use Virtualization.framework |
| `com.apple.security.network.client` | Required for the auto-updater to check GitHub Releases |

No other entitlements are requested. The app does not request access to the camera, microphone, contacts, location, or any other sensitive resource.

## Auto-Update Security

### Signature verification

The Tauri updater uses minisign for update signature verification:

- A keypair is generated with `cargo tauri signer generate`.
- The public key is embedded in `tauri.conf.json` and compiled into the binary.
- During CI release builds, the private key (stored as a GitHub Actions secret) signs the update artifact.
- Before installing an update, the app verifies the signature against the embedded public key.
- An attacker who compromises the GitHub Release (but not the signing key) cannot deliver a malicious update.

### Update flow

1. On launch, the app checks `https://github.com/google/capsem/releases/latest/download/latest.json`.
2. If a newer version exists, the JSON contains the download URL and signature.
3. The app downloads the update artifact (`.app.tar.gz`).
4. The signature is verified against the embedded public key.
5. If valid, the update is installed and the app restarts.
6. If invalid, the update is rejected and the current version continues running.

### HTTPS enforcement

All update checks and downloads use HTTPS. The updater rejects HTTP endpoints in release builds.

## Supply Chain

### Binary transparency (CI)

Release builds include multiple supply chain protections:

**SLSA Build Provenance (Level 2)**

```yaml
- uses: actions/attest-build-provenance@v3
  with:
    subject-path: target/release/bundle/dmg/*.dmg
```

This generates a signed attestation proving the binary was built by a specific GitHub Actions workflow from a specific commit. Users can verify:

```sh
gh attestation verify Capsem.dmg --repo google/capsem
```

**Software Bill of Materials (SBOM)**

An SPDX SBOM is generated and attested alongside each release:

```sh
cargo sbom --output-format spdx_json_2_3 > capsem-sbom.spdx.json
```

This documents every dependency compiled into the binary, enabling vulnerability scanning against known CVE databases.

**cargo-auditable**

CI builds use `cargo auditable build`, which embeds a compact dependency manifest (~4KB) directly in the binary. This allows post-hoc vulnerability scanning:

```sh
cargo audit bin /Applications/Capsem.app/Contents/MacOS/capsem
```

### Dependency policy

- All Rust dependencies are locked in `Cargo.lock`.
- Frontend dependencies are locked in `pnpm-lock.yaml` with `--frozen-lockfile` in CI.
- The VM image is built from Debian bookworm packages with pinned versions.

## Planned Security Features

### Milestone 5: Network filtering

- Domain allow/block lists enforced at the host level via TLS SNI inspection.
- HTTPS-only: no plain HTTP, no UDP, no ICMP from the VM.
- Zero DNS leaks: fake DNS inside VM, real DNS resolution on host only.

### Milestone 6: API key isolation

- API keys stored in macOS Keychain, never inside the VM.
- Host-side gateway injects keys into outbound API requests.
- Keys are not in VM environment variables or VM memory.

### Milestone 7: MCP sandboxing

- Host-side MCP servers run inside macOS Seatbelt (`sandbox-exec`) profiles.
- Each server is confined to the workspace directory.
- Cannot read `~/.ssh`, `~/.aws`, `~/.config`, or any path outside the workspace.
- MCP tool calls go through a policy engine: allow, block, or require user approval.

### Milestone 8: Session isolation

- Each session gets a separate overlay disk, separate vsock CID, and separate VirtioFS mounts.
- No cross-session data leakage.
- Session deletion wipes the overlay disk.

### Milestone 11: Graceful shutdown

- `Cmd+Q` triggers graceful shutdown: sync filesystems, unmount overlays, ACPI poweroff.
- Prevents ext4 corruption on persistent overlay disks.
- The app only exits after all VMs reach the stopped state.

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
sha256sum -c assets/SHA256SUMS
```

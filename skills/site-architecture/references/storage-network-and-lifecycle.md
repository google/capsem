# Storage, Network, and Lifecycle Architecture

Read this reference before changing VM storage or forks, network interception,
security or logger ownership, the ephemeral-session model, installation,
service registration, companion launch, or self-update behavior.

## Storage modes

Selected by kernel cmdline `capsem.storage=virtiofs` (default) or absence (block mode).

**VirtioFS mode** (default):
```
~/.capsem/sessions/{id}/
  system/rootfs.img    # ext4 loopback (2GB sparse) -- overlayfs upper
  workspace/           # VirtioFS files for /root (host-visible)
  auto_snapshots/      # Rolling ring buffer (12 APFS clones, 5min interval)
```

Boot sequence: profile-selected read-only rootfs asset -> VirtioFS mount -> loopback ext4 -> overlayfs -> bind-mount workspace.

Why ext4 loopback: Apple VZ's VirtioFS doesn't support `mknod` (whiteout creation), so overlayfs can't use VirtioFS directly as upper.

**Block mode** (legacy): tmpfs overlay + scratch disk. No host file visibility, no snapshots.

**Fork images** (user-created templates):
```
~/.capsem/images/
  image_registry.json       # Image metadata index (JSON)
  {name}/
    system/                  # APFS clone of source VM's rootfs overlay
    workspace/               # APFS clone of workspace files
    session.db               # Telemetry from source VM (checkpointed)
```

## Network architecture

The guest is air-gapped. No real NIC, no real DNS, no direct internet access.

1. `capsem-init` creates a dummy0 NIC with fake DNS (dnsmasq)
2. iptables redirects all port 443 traffic to `capsem-net-proxy` on localhost:10443
3. `capsem-net-proxy` bridges each TCP connection to host vsock port 5002
4. Host network intercept terminates TLS using per-domain minted certs (signed by static Capsem CA)
5. Host parses HTTP/model facts into a `SecurityEvent` and calls the shared security engine
6. Runtime materialization forwards allowed bytes to upstream
7. Logging plugins produce ledger-safe event output for the logger DB

### Network/security policy

- Corp config owns enterprise constraints, reporting endpoints, and locked
  rule/plugin policy.
- Profile config owns VM assets, MCP config, rules, detections, plugins, and
  defaults for sessions created from that profile.
- Settings config owns UI/app preferences only.
- All enforcement and detection compiles into one `SecurityRuleSet` over
  `SecurityEvent`; there is no domain-policy, HTTP-policy, or MCP-policy
  decision provider.
- Credential capture/injection belongs to the credential broker plugin.
  Durable ledger materialization belongs to the logger DB boundary after
  logging plugins such as `log_sanitizer` produce ledger-safe events. Network
  formatters, service routes, frontend transforms, and debug harnesses must not
  implement credential handling or logged-data caches.

### Logger DB boundary

`capsem-logger` owns SQLite connections and storage mechanics. Routes,
service code, MCP helpers, UI handlers, and benchmarks must not call
`rusqlite::Connection::open` or `DbReader::open` directly and must not maintain
their own telemetry/security projection caches. They call a logger DB object to
run queries and writes.

The DB layer owns connection threads, `mem`/disk table layout, batching, flush,
rehydration, WAL tuning, and future FTS5/search. It does not own product route
semantics by hardcoding route-specific helper methods in `DbWriter`; callers may
own query intent while the DB object owns execution. Missing ledger tables or
columns are schema-contract failures, not empty data.

### MITM CA

- Static CA: `crates/capsem-core/resources/ca/capsem-ca.key` + `crates/capsem-core/resources/ca/capsem-ca.crt` (ECDSA P-256)
- Baked into rootfs via `update-ca-certificates` + certifi patch
- Guest trusts it via system store + env vars (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`)

## Ephemeral VM model (invariants)

**VirtioFS mode**: fresh workspace + sparse rootfs.img per session. Host creates empty dirs, guest formats on first boot.

**Block mode**: `mke2fs` runs unconditionally at boot. Overlay upper is always tmpfs.

**Sessions run profiles.** Session workspace and overlay state are session
state; image contents come from the profile asset contract. Never make the
overlay upper layer a hidden image-authoring rail. To add packages, edit the
profile-owned package files under `config/profiles/<id>/` and rebuild through
the profile-derived asset rail.

**Fork images** extend the session model with reusable templates. `capsem fork
<session> <image-name>` snapshots a session via APFS clonefile. Forks stay tied
to their profile asset contract. Deleting any image is always safe; asset
cleanup protects referenced profile assets.

## Installation and service lifecycle

Release packages are the primary install entry point. Local development uses
the same package rail as CI: build the package, pass a manifest override, and
let the package install service files plus manifest URL provenance.

Package install handles service registration, records manifest metadata metadata,
and hydrates the live manifest through `capsem update --assets --manifest
<URL>`. Profile configuration handles security rules, plugins, MCP, assets, and
packaged root content; credentials are brokered at runtime.

**Install layout** (`~/.capsem/`):
- `bin/` -- capsem, capsem-service, capsem-process, capsem-mcp, capsem-gateway, capsem-tray
- `assets/` -- manifest.json, manifest-metadata.json, and profile-selected VM
  assets such as `vmlinuz`, `initrd.img`, and EROFS rootfs images
- `run/` -- service.sock, service.pid, gateway.token, gateway.port, gateway.pid, instances/{id}.sock

**Service registration**: LaunchAgent `com.capsem.service` (macOS) or systemd user unit `capsem.service` (Linux). KeepAlive/Restart=always. Service auto-launches gateway and tray as companion processes, passing `--parent-pid` so companions self-exit when the service dies (see capsem-guard, `/dev-rust-patterns` lesson 18).

**Auto-launch cascade**: capsem-service starts -> spawns capsem-gateway (port 19222) + capsem-tray. All three are separate processes.

**Self-update**: `capsem update` checks the release-channel health index,
downloads verified binary installers, prints the package-manager apply command
for audit, executes it with `--yes`, materializes VM assets from URL-shaped
manifest sources, and reports manifest metadata/hash plus update availability
through the canonical `/system/status` service endpoint. Background update state
is merged into `~/.capsem/assets/manifest-metadata.json` and refreshes on ordinary CLI commands.

Key source files: `crates/capsem/src/paths.rs`,
`crates/capsem/src/service_install.rs`, `crates/capsem/src/update.rs`, and
`crates/capsem/src/uninstall.rs`.

# Storage, Network, and Lifecycle Architecture

Read this reference before changing VM storage or forks, network interception,
security or logger ownership, the ephemeral-session model, installation,
service registration, companion launch, or self-update behavior.

## Storage modes

Selected by kernel cmdline `capsem.storage=virtiofs` (default) or absence (block mode).

**VirtioFS mode** (default):
```
~/.capsem/sessions/{id}/
  guest/                     # the only VirtioFS share
    system/rootfs.img        # ext4 loopback (2GB sparse) -- overlayfs upper
    workspace/               # VirtioFS files for /root (host-visible)
  system -> guest/system     # compat symlinks
  workspace -> guest/workspace
  session.db                 # host-only, outside the share
```

Boot sequence: profile-selected read-only rootfs asset -> VirtioFS mount -> loopback ext4 -> overlayfs -> bind-mount workspace.

Why ext4 loopback: Apple VZ's VirtioFS doesn't support `mknod` (whiteout creation), so overlayfs can't use VirtioFS directly as upper.

**Block mode** (legacy): tmpfs overlay + scratch disk. No host file visibility.

**Forks** (`capsem fork`) become new persistent sandboxes:
```
~/.capsem/run/
  persistent_registry.json  # Persistent sandbox metadata
  persistent/{vm_id}/
    guest/system/            # CoW clone of source VM's rootfs overlay
    guest/workspace/         # CoW clone of workspace files
    session.db               # SQLite-consistent copy of the source ledger
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

### Private networks between VMs: cables and switches

The model is physical. Every VM starts **unplugged**: it cannot reach any
other VM. A named network is **one switch**: one confined
`capsem-router --network` process behaving like an ordinary layer-2 switch
with no uplink. `capsem network connect <network> <vm>` **plugs** a cable from
the VM into that switch; `disconnect` **unplugs** it. Everything plugged into
the same switch can talk, over any protocol. That is the user's choice: the
switch is a network system, not a security boundary, and it has no route to
the internet, the host, or another switch.

Do not reintroduce per-flow machinery. There is no private TCP relay, no
per-connection admission, no tokens per flow, no TCP/UDP parsing, and no
per-flow policy in the data plane. Those were built once (guest REDIRECT to
10128, vsock 5010, `/networks/private/connect`, `PrivateAccept`, the router's
private class) and removed: they added privileged attack surface and a second
lifecycle while doing nothing a plugged cable does not already do. Traffic
inspection, when wanted, comes from a future mirror port on the switch, not
from logic inside it.

**Cable.** One cable per attachment; a VM on ten networks has ten cables.

- Guest end: one tap device per cable, created and removed by the agent on the
  owner's instruction, with MAC `mac_of(address)`, MTU `LINK_MTU`, a connected
  route for that network's subnet only, and a declared link of 10 Gb/s full
  duplex with carrier up (`ETHTOOL_SSET` on the tap, which the core turns
  into link settings; the tap default is 10 Mb/s, which makes Linux tooling
  and schedulers treat the link as slow). `/sys/class/net/<tap>/speed` reads `10000`. The guest kernel is
  tuned to match: the defconfig carries the options a 10 Gb/s stack needs,
  `capsem-init` writes socket buffer and backlog limits sized for 10 Gb/s
  with 64 KiB frames through `/proc/sys`, and each tap gets a transmit queue
  long enough not to drop under a burst. `capsem-tun` pumps each
  tap's ethernet frames as `[u16 len][frame]` records over its own vsock 5009
  connection, opening with the cable id the owner assigned. The guest never
  names a network and never forwards between cables: the container launcher,
  which turns forwarding on for the container, drops `cable+ -> cable+` first,
  and `capsem-init` sets `arp_announce=2`/`arp_ignore=1` so each cable speaks
  ARP only as its own address (port security below requires it).
- Owner (`capsem-process`) keeps the VM's cable list, `cable id -> (network,
  generation, stream)`. It never reads frames. `plug()` has the agent create
  the tap, waits for that cable's stream, and hands the service a duplicate
  descriptor; `unplug()` drops the stream and removes the tap.

**Addressing.** The service allocates each network a subnet from
`10.128.0.0/9` when the network is created (stored in its `network.db`), and
each attachment an address inside it. Subnets never overlap, so addresses and
their derived MACs are unique host-wide. The hypervisor has no part in it:
neither Apple VZ nor KVM gives the VM a NIC; vsock is the only wire.

**Switch** (`capsem-router --network`, one per network, started on first plug).

- Forwards on MAC only. A service-programmed table `MAC -> port`; no learning,
  no aging. Unicast goes to the owning port; an unknown destination is dropped
  and counted (the table is complete, so nothing floods). Broadcast
  (`ff:ff:ff:ff:ff:ff`) and multicast flood to every other port, exactly like a
  normal switch: that is how guests resolve each other with ordinary ARP
  requests. The switch never answers ARP itself. A per-port broadcast cap
  (1024 floods/s) exists only against storms, sized so normal ARP never hits
  it; drops above it are counted.
- Port security, as a managed switch has it: each port is a `Station` (MAC and
  leased address). A frame whose source MAC is not its port's is dropped
  (`source_mac`); an IPv4 packet or ARP message whose sender address is not
  its port's is dropped (`source_address`), so no member speaks as another or
  poisons ARP. These are fixed-offset comparisons on the header, not parsing:
  other ethertypes cross unchecked, and nothing above the IPv4 source is read.
- `Switch::plug(port, mac)` and `Switch::unplug(port)` are the only control
  operations. A port id carries its attachment generation; a stale one is
  refused.
- Built for throughput: one read fills a `BytesMut` with many records,
  `split_to().freeze()` hands each frame on as `Bytes` (flood = refcount, not
  copy), writers drain their queue into `write_vectored`, the table is an
  atomically swapped snapshot read without locks, and a full per-port queue
  drops with `try_send` so a slow member never stalls the others.
- A port is one owned job: reader and writer under one cancellation token.
  `unplug` cancels and joins both halves, closes the descriptor, then reports
  `Closed{counters}` and frees the quota slot. The service holds an unplugged
  port until that report arrives, so the leave's close row carries the
  counters.
- Confined before it accepts a grant: cleared environment, inherited
  descriptors closed, parent watch, Seatbelt `(deny default)` on macOS,
  seccomp allowlist without open/connect/accept/clone on Linux, no
  virtualization entitlement. Startup fails closed. It holds only its own
  ports' descriptors and opens nothing.

**Lifecycle** (service owns it; `plug()`/`unplug()` in the service's switch
registry drive owner and switch).

- Membership (and its durable state `declared | attaching | ready | failed`)
  lives in `NetworkRegistry`; each attachment's generation lives in the
  service's switch registry, because it only orders plugs within one switch
  process's life. Every plug and unplug bumps it.
- `plug`: mark `attaching` with a new generation, start the switch if needed,
  get the cable from the owner, `Switch::plug`. Once the switch has it, check
  under the registry lock that the VM is still a member on that generation:
  if not, unplug at once; otherwise mark `ready`.
- `detach`: revoke the membership first, then `unplug` (which bumps the
  generation and closes the port). A plug that completes in between fails
  its check, so a successful disconnect never leaves a live port.
- Retiring a network cancels the switch host's token, which closes the grant
  channel, ends the event loop, kills and waits the child, and joins every
  task before `retire` returns.
- A dead switch is retired and its ports' members are plugged again into a
  fresh one; each re-plug checks membership, so removed members never return.
  A dead guest pump closes its port: the service marks the member `declared`
  and re-plugs that cable alone once the pump has had time to reconnect.
- Bounded: ports per switch, fixed per-port queues and a broadcast cap; a
  newer plug or unplug makes any older plug under way unplug itself.
- Logging: plug/unplug transitions and the switch's per-port counters (frames
  and bytes each way, drops per reason) go to the network's ledger through
  `NetworkRegistry` and the logger DB boundary. The switch never touches a
  database.

**Containers** run inside the VM. `launch.py` forwards and NATs every protocol
between the container veth and each attached tap (SNAT out the tap, DNAT in).
Private subnets RETURN before the egress port REDIRECTs so member traffic on
443 or 80 never enters the MITM path.

**Names.** `<vm>.<network>.capsem.internal` resolves to that attachment's
address, answered on the host for members of that network only, zero TTL,
never upstream. There is no network-less `<vm>.capsem.internal`: a VM has one
address per network, not one lifetime address.

**Published host ports** are a separate responsibility (the per-VM router's
expose class) and never a path between VMs.

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
WAL tuning, and future FTS5/search. It does not own product route
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
<session> <image-name>` clones a session through
`capsem_core::session::clone_sandbox_state`, which walks the guest-writable
tree with descriptor-relative no-follow operations
(`capsem_foundation::unix::tree_clone`): symlinks are recreated, never followed;
setuid/setgid/sticky bits are dropped; a non-regular `rootfs.img` is refused.
File contents share extents via APFS `clonefile` or Linux `FICLONE`, falling
back to a sparse copy. Forks stay tied
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
- `bin/` -- capsem, capsem-service, capsem-process, capsem-mcp-aggregator, capsem-mcp-builtin, capsem-gateway, capsem-tray
- `@capsem/mcp` -- separately installed npm host MCP package
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

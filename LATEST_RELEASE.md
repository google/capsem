version: 0.6.3
---
Capsem 0.6.3 improves build and test cache reuse, tightens sandbox security,
and reduces service and gateway overhead. The accompanying code and co-work
profiles advance independently to 0.6.2 to deliver the guest fixes.

### Cache and verification

- One typed cache controller owns disk, Docker/Colima, and Tart inventory,
  retention, and cleanup. `just cache` shows usage and policy; cleanup previews
  protect active work and foreign resources, and applied mutations are journaled.
- Linked worktrees share cache authority. Cargo, Python, Node, compiler,
  image, and package caches retain expensive work across isolated builds.
- VM assets and release staging reuse digest-verified objects. Retained VM
  generations are separate from exported runtime views, so exports preserve
  the receipts needed for reuse.
- Exact-source local qualification resumes proven work and reuses successful
  journals. Focused verification avoids unnecessary complete reruns, while
  hosted release lanes independently qualify their package/profile pairings.
- Python and Node dependency audits share a checksum-pinned scanner and cache
  only short-lived clean verdicts. Failures are never cached.

### Security and reliability

- Hardened VirtioFS and file operations against guest symlink escapes, and
  bounded guest-controlled frames, downloads, decompression, and HTTP bodies.
- Built-in HTTP tools check and pin resolved addresses, blocking DNS rebinding
  and unauthorized access to private addresses. Network policy handles legacy
  IPv4 spellings and IPv6 literals consistently.
- Credential-store updates use private, collision-safe atomic writes.
- Guest control and audit channels recover more reliably after reconnects;
  slow MCP calls and oversized file reads no longer strand unrelated work.
- Fixed KVM pause snapshot races, persistent-session lifecycle handling, and
  retrieval of preserved logs after a failed ephemeral VM.
- Debian installation grants restricted KVM/vsock access to the existing
  user service, with access restored after device events.

### Performance and tooling

- Service filesystem work runs off asynchronous workers, persistent-registry
  writes release the reader lock before disk I/O, and the gateway reuses service
  connections.
- The guest benchmark helper is smaller, and snapshot measurements reuse an
  MCP connection to measure operations without repeated client startup.
- Admin image and manifest commands select the locked builder environment.
  Bounded diagnostics correctly enter virtual environments and reap detached
  descendants when commands time out.
- Web sources and Python engineering tooling have consolidated owners under
  `web/` and `build_system/`.

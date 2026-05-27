# Sprint: Linux Release

Get the Linux `.deb` release path working end-to-end. Deferred out of the next-gen → main merge -- macOS arm64 `.pkg` is the shipping artifact for the first post-merge release; Linux can iterate on `main` afterward.

## Context

- Hypervisor backend on Linux is **KVM** (vs macOS Apple VZ). See `/dev-testing-hypervisor` for what KVM needs.
- Release pipeline currently builds `.deb` for arm64 + x86_64 via `cargo tauri build --bundles deb`, then runs `scripts/repack-deb.sh` to inject companion binaries (`capsem-service`, `capsem-gateway`, `capsem-tray`).
- User reports "I don't think it works as is." No concrete repro yet -- this sprint starts with a verification pass.
- Relevant skills to load before any Linux work: `/dev-installation` (install layout, service registration), `/dev-testing-hypervisor` (KVM CI, what the backend needs), `/dev-debugging` (reproduce-first workflow), `/dev-rust-patterns` (cross-compilation gotchas).

## L0: Verify what actually breaks

- [ ] Run `.github/workflows/release.yaml` deb build locally via `act` or on a branch tag (arm64 + x86_64 matrix)
- [ ] Download artifact, `dpkg-deb --info` and `dpkg-deb --contents` — confirm companion binaries present
- [ ] Install on clean Ubuntu 24.04 VM (arm64): `sudo dpkg -i capsem_*.deb`
- [ ] Install on clean Ubuntu 24.04 VM (x86_64)
- [ ] `capsem doctor` on installed system — capture failures
- [ ] `capsem shell` — does a VM boot? (KVM, not VZ)
- [ ] File concrete bug list from the failures below, update checklist

## L1: Known gaps (fill in after L0)

- [ ] TBD -- symptoms go here
- [ ] TBD
- [ ] TBD

## L2: Service registration on Linux

- [ ] systemd unit vs user-mode daemon — which does `capsem setup` register on Linux?
- [ ] `scripts/pkg-scripts/postinstall` is macOS .pkg-only; `.deb` has its own postinst hooks — verify parity (service enable, asset prefetch)
- [ ] `capsem uninstall` cleanly removes the systemd unit
- [ ] Log location matches `/dev-installation` expectations

## L3: KVM hypervisor path

- [ ] `just shell` on Linux boots a VM via KVM
- [ ] vsock works (host <-> guest IPC)
- [ ] VirtioFS mount works
- [ ] MITM proxy terminates TLS correctly
- [ ] Reference: `/dev-testing-hypervisor` for what the KVM backend needs

## L4: CI coverage

- [ ] `ci.yaml` already has a Linux-arm64 KVM job — verify it still passes after merge
- [ ] Add a Linux install-smoke job equivalent to the macOS Docker e2e gate
- [ ] Release workflow: `.deb` repack + dpkg install + `capsem doctor` must pass before publishing release

## L5: Signing / provenance

- [ ] `.deb` is not currently signed (no `dpkg-sig` step in release.yaml) — decide: sign with minisign (already used for manifest), GPG, or ship unsigned
- [ ] SLSA attestation already covers `.deb` (release.yaml line 615-625) — verify

## Out of scope

- macOS packaging (covered by release pipeline today)
- Windows (not a supported platform)
- Snap / Flatpak / AppImage (future)

## Exit criteria

- Linux user can download `.deb`, `sudo dpkg -i`, run `capsem shell`, get a working KVM VM
- CI publishes `.deb` for arm64 + x86_64 on every release tag with the install-smoke gate passing

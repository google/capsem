---
name: build-images
description: Building the Capsem VM runtime (kernel, initrd, rootfs). Use when working on the runtime package set, Dockerfile templates, kernel or rootfs builds, or the capsem-builder backend.
---

# Building the Capsem VM runtime

## Overview

Capsem builds one VM runtime: a kernel, initrd and rootfs per architecture.
Applications come from OCI images (`images/`,
resolved through `images/catalog.toml`), never from the runtime rootfs, so
every build of a commit produces the same asset set.

- `config/docker/image/` is the runtime's source: `build.toml` (package set,
  EROFS settings, size budgets, kernel pin, base images), `manifest.toml`, and
  the `kernel/`, `security/` and `vm/` inputs.
- `guest/artifacts/` holds the core guest payloads: `capsem-init`, doctor,
  diagnostics, bench, the container launcher.
- `capsem-admin image build` copies those two inputs into a generated backend
  workspace under `cache/target/build/image-workspace/<arch|all>/` and runs the
  build there.
- The Python builder backend renders Docker templates and emits assets, build
  ledgers, and OBOMs only when invoked by the admin build rail. Do not add
  product truth directly to the backend image-spec path.

## Source Layout

```
config/
  docker/                 Dockerfile templates (*.j2)
  docker/image/
    build.toml            Runtime package set, EROFS, rootfs budgets, kernel pin
    manifest.toml         Image manifest metadata and changelog
    kernel/               defconfig.<arch> and ordered kernel patches
    security/             Built-in web security inputs
    vm/                   Guest shell environment
guest/artifacts/          Core guest payloads: init, doctor, diagnostics, bench
images/                   OCI application images and their catalog
cache/target/assets/            Generated VM assets
cache/target/packages/          Generated native packages
```

The materialized backend workspace is an implementation detail, not an
authoring surface. It is never a config root.

`capsem-admin` is a tool, not a config authority. It validates, materializes,
builds, and checks contracts; it must not grow scaffolding commands that invent
MCP, AI provider, package, or rule truth. Do not add admin config roots, guest
config roots, settings metadata, provider registries, or backend-owned
catalogs as product truth.

## CLI commands

```bash
just build-assets [arch]                      # The runtime for one arch, or every one
just _build-assets [arch]                     # Same rebuild, CI-facing name
just _build-kernel arm64                      # Kernel slice
just _build-rootfs arm64                      # Rootfs slice
uv run --project build_system --frozen capsem-builder audit                  # Parse trivy/grype vulnerability output
```

Underneath, the gate runs
`capsem-admin image build [--config-root config] [--guest-dir guest] [--output assets] [--arch arm64|x86_64] [--template all|kernel|rootfs] [--clean] [--json]`.

Use admin/just recipes for all runtime image work. `capsem-builder` is a
backend helper only; it must not expose or document public `build`, `validate`,
`inspect`, `mcp`, render-only, or dry-run rails for image authoring.
`capsem-admin image build` may call private Python modules such as
`capsem_builder.image.image_build_backend`; agents must not make those modules
public CLI contracts.

## Per-arch asset layout

```
cache/target/assets/
  manifest.json          Version, checksums, asset list
  B3SUMS                 BLAKE3 checksums
  arm64/
    vmlinuz              Kernel
    rootfs.erofs         Root filesystem
    initrd.img           Initial ramdisk (repacked by just exec)
```

The approved release default is EROFS with `lz4hc` compression level 12
(`config/docker/image/build.toml [build.erofs]`).

`config/docker/image/build.toml [build.rootfs]` also owns two independent
release budgets: the exported tar ceiling and the final EROFS ceiling. The
builder checks the first before compression and the second before writing the
artifact ledger. Never raise a size ceiling to accommodate an unexplained
image jump; inspect the rootfs composition first.

## Build Ledger

Each per-arch build emits `build-ledger.log` JSONL. The
`rootfs.config_inputs` record captures the rendered rootfs package list and
the EROFS config. Installed-package/component truth belongs in the CycloneDX
OBOM, not the build ledger.

## Adding a guest tool

The runtime package set is
`config/docker/image/build.toml [build.rootfs].runtime_apt_packages`. Add a
package there **only** when Capsem's own guest machinery needs it: the
container launcher (runc, umoci, iptables, iproute2), `capsem-init` (auditd,
e2fsprogs), the trust store the Capsem CA lands in, or what capsem-doctor,
capsem-bench and the network diagnostics run with.

Anything a user or an agent works with -- an AI CLI, a language toolchain, a
vendor binary -- belongs in an OCI image under `images/`, not in the runtime.

1. Add the package to `runtime_apt_packages` (no duplicates; the model refuses
   an empty or duplicated list).
2. Rebuild with `just build-assets` (or `just _build-rootfs <arch>`).
3. Verify with `capsem-doctor` inside a booted VM.

Do not edit generated Dockerfiles. Docker templates live under `config/docker/`.

## Dockerfile templates

Templates live in `config/docker/`:
- `Dockerfile.rootfs-dependencies.j2` -- snapshot-selected Debian packages;
- `Dockerfile.kernel-dependencies.j2` -- snapshot-selected kernel toolchain and
  the SHA-256-verified kernel archive;
- `Dockerfile.rootfs.j2` -- network-denied first-party rootfs assembly;
- `Dockerfile.kernel.j2` -- network-denied kernel compilation, initrd assembly,
  and vmlinuz extraction.

`asset-dependencies` is the visible resumable frontier between those pairs.
The gate materializes one input-keyed helper for every selected
architecture/template, validates its platform and identity label, and
passes only its exact image ID to the source build. Source builds always use
BuildKit network `none` and never use the remote CI cache. A carried frontier
must revalidate every helper; it must not silently rebuild inside the sealed
lane. The dependency helpers reuse the one checked-in Debian snapshot
authority rather than the mutable sources inherited from the base image.

Templates use Jinja2 with variables from the admin-materialized image
workspace. Do not add a second preview rail for product truth; if a build input
needs validation, add it to the normal admin validation path.

Every architecture's `base_image` is required to be its immutable
`repository@sha256:<child-manifest>` identity. Do not use a mutable tag or the
multi-platform index. `capsem-gate` materializes a missing exact child through
the Docker daemon before the cross-execution probe and build lanes; keep this
on the ordinary guarded Docker runner, not the host-process egress broker.

---

# Builder Internals (for modifying the builder itself)

## Architecture: config/docker/image -> admin workspace -> Pydantic -> context dict -> Jinja2 -> Dockerfile

1. **Runtime source** (`config/docker/image/` and `guest/artifacts/`).
2. **capsem-admin** copies both into a backend build workspace.
3. **Pydantic models** (`build_system/builder/image/models.py`) parse that workspace.
4. **Context dict** (`build_system/builder/image/docker.py`) feeds Jinja2 templates.
5. **Jinja2 templates** (`config/docker/`) produce Dockerfiles.

### Key files

| File | Role |
|------|------|
| `build_system/builder/image/models.py` | All Pydantic models (enums, configs, top-level `GuestImageConfig`) |
| `build_system/builder/image/config.py` | Backend loader for admin-materialized build workspaces |
| `build_system/builder/image/docker.py` | Context builders (`_rootfs_context`, `_kernel_context`), rendering, build execution |
| `build_system/builder/image/image_build_backend.py` | Private admin-invoked image build backend; not a public CLI |
| `crates/capsem-admin/src/image_build.rs` | `image build` / `image workspace`: workspace materialization and the build plan |
| `config/docker/Dockerfile.rootfs.j2` | Rootfs Dockerfile template |
| `config/docker/Dockerfile.kernel.j2` | Kernel Dockerfile template |
| `build_system/builder/image/validate.py` | Validation rules (E001-E302, W001-W012) |
| `build_system/builder/image/cli.py` | Click CLI entry points |

### Context dict (rootfs template variables)

`_rootfs_context()` in `docker.py` builds the dict passed to `Dockerfile.rootfs.j2`
(plus the Debian snapshot keys):

```python
{
    "arch": ArchConfig,           # Per-arch settings (docker_platform, rust_target, etc.)
    "arch_name": str,             # "arm64" or "x86_64"
    "apt_packages": list[str],    # build.toml [build.rootfs].runtime_apt_packages
    "guest_binaries": list[str],  # GUEST_BINARIES in docker.py
}
```

### Kernel context dict

```python
{
    "arch": ArchConfig,
    "arch_name": str,
    "kernel_version": str,  # exact checked-in release, e.g. "6.18.44"
    "kernel_sha256": str,   # verified before the source archive is extracted
    "kernel_patches": list, # applied in order with --fuzz=0
}
```

## How to: Add a new guest binary

Guest binaries are compiled from `crates/capsem-agent/`. Every architecture on
every host goes through `container_compile_agent()`; native Linux has no second
ambient Cargo/rustup rail. The asset preflight first materializes the host
platform's config-selected exact Rust child image plus the checked-in Rust
toolchain and `Cargo.lock`; a foreign target also materializes its exact
config-pinned C compiler package and both the foreign-target and host-target
Cargo dependency sets. The host set covers target-gated build-script and
proc-macro dependencies that `cargo fetch --target <foreign>` does not select.
The actual container build runs with
`--network none` and Cargo `--locked --offline`.

Materialize only helpers the command can consume: every requested architecture
on either supported host, and none for a kernel-only build. Immutable Debian
guest bases remain architecture-selected separately.

1. Add the binary target in `crates/capsem-agent/Cargo.toml`
2. Add the binary name to `GUEST_BINARIES` list in `docker.py`
3. The template already loops `{% for binary in guest_binaries %}` to COPY + chmod 555

## Verifying Linux builds locally

`just _cross-compile [arch]` builds everything in a container: agent binaries,
frontend, and the full Linux `.deb` package. Useful for catching system
dependency issues before CI.

```bash
just _cross-compile           # Build for host arch (arm64 on Apple Silicon)
just _cross-compile x86_64    # Build x86_64 deb
```

## Build pipeline (what `build_image()` does)

For rootfs:
1. Build guest agent binaries (`cross_compile_agent` -- every target uses the
   pre-materialized, network-denied Rust builder; a foreign target cross-compiles
   on the host CPU)
2. Assemble build context (`prepare_build_context`) -- copies CA cert, shell configs, diagnostics, agent binaries
3. Render Dockerfile from template
4. `docker build`
5. Export container filesystem as tar
6. Create EROFS from tar (`create_erofs` -- runs mkfs.erofs in a container)
7. Clean up container image

For kernel:
1. Read the exact kernel version and SHA-256 from the checked-in build config
2. Assemble build context (defconfig, capsem-init)
3. Render Dockerfile from template
4. `docker build`, verifying the downloaded source archive before extraction
5. Extract vmlinuz + initrd.img from image
6. Clean up

## The guest Rust builder workspace: do not widen the `/src/*` glob

`container_compile_agent` mounts the checkout read-only at `/src` and assembles
a writable workspace at `/build`:

```sh
for f in /src/*; do b=$(basename "$f"); \
  [ "$b" != target ] && [ "$b" != crates ] && ln -s "$f" /build/; done
```

`/src/*` does not match dotfiles. **That exclusion is load-bearing. Widening it
breaks the build.** It reads like an oversight -- `.cargo/config.toml` never
reaches the container, so the checked-in Cargo configuration is never applied --
and it has been "fixed" on exactly that reasoning.

Why it must stay: `.cargo/config.toml` declares

```toml
[target.x86_64-unknown-linux-musl]
linker = "rust-lld"
```

On a developer host, `x86_64-unknown-linux-musl` is a *cross* target and
rust-lld is correct. Inside the Alpine builder that same triple **is the host
target**, so inheriting the file makes every proc-macro crate -- `serde_derive`,
`tokio-macros` -- link its host `.so` with rust-lld:

```
rust-lld: error: unable to find library -lgcc_s
rust-lld: error: unable to find library -lc
error: could not compile `tokio-macros`
```

The rule this generalizes to: **checked-in Cargo configuration is
developer-host configuration.** The builder container owns its own toolchain
settings and receives them as environment on the `docker run`, never by reading
them out of the tree. If a container build needs a linker or a `CC`, pass it
explicitly.

`build_system/tests/gate/test_guest_rust_builder_hermetic.py::test_container_workspace_excludes_dotfiles`
fails if the glob is widened, so this cannot be rediscovered the slow way.

## Cross-compiling guest binaries instead of emulating

Foreign-target guest builds used to run `rustc` under QEMU on the target's own
platform child. Measured on a 16-core Linux host, cold, `--locked --offline`,
for the six aarch64 guest binaries:

| | |
|---|---|
| emulated (`--platform linux/arm64`, qemu-aarch64) | 1194.7s |
| cross-compiled from the amd64 base | 86s |

A release run compiles that graph for every architecture's rootfs and again
for `assets.pack-initrds`, so emulation costs tens of minutes per run.

What a cross image needs, materialized at image-build time on the same
network-open setup edge that `cargo fetch --locked` already uses:

- `apk add --no-cache ${CROSS_PACKAGES}`, where config owns the exact package
  tuple (currently `clang21=21.1.2-r2`) and the helper identity includes it.
  `ring` is the **only** crate in the
  `capsem-agent` + `capsem-bench` graph that compiles C -- nothing else pulls
  `cc`, `cmake` or `bindgen`. Alpine's clang cross-compiles it for a foreign
  musl target with **no external sysroot**. The pinned Rust toolchain already
  supplies `rust-lld`; no separate ambient linker package is installed.
- `rustup target add "${RUST_TARGET}"`, after which the existing
  `rustup target list --installed` assertion proves it landed.
- `CC_<target>=clang` and `CFLAGS_<target>=--target=<target>` on the run.
- `CARGO_TARGET_<TARGET>_LINKER=rust-lld` on the run -- **not** by linking
  `.cargo/config.toml` into the workspace, for the reason in the section above.

The runtime build stays `--network none` with `--locked --offline`. The image
tag is keyed by the base, target, cross-package tuple, Dockerfile and lockfiles,
so a change to any of them is a different image rather than a silent reuse.

## Container runtime requirements

On macOS, Docker runs inside a Colima VM with limited resources.
The rootfs build runs apt and the guest Rust builds concurrently --
the default RAM allocation may cause OOM kills (exit code 137).

**Minimum**: 12GB RAM. **Recommended**: 16GB RAM, 8 CPUs.

```bash
# Colima (macOS)
colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8

# Linux: Docker runs natively, no memory tuning needed
# sudo apt install docker.io
```

`just doctor` owns the product readiness gate. `capsem-builder doctor` is a
backend helper used by the build rail to check container/runtime prerequisites.

The resource check lives in `build_system/builder/image/doctor.py`:
- `check_container_resources()` -- checks docker info
- Thresholds: `DOCKER_MIN_MEMORY_MB = 4096`, `DOCKER_RECOMMENDED_MEMORY_MB = 8192`

## Container image compatibility

Guest cross-build containers use the exact per-platform
`rust:1.97.1-alpine3.23` child manifests in
`config/docker/image/build.toml`, never a mutable Rust tag. Those children
already own the exact toolchain, native musl target, musl headers, and compiler.
At the guarded asset-prefetch boundary,
`build_system/docker/Dockerfile.guest-rust-builder` resolves the Cargo.lock graph and, only
for a foreign target, adds the exact config-pinned C compiler package and Rust
target. Do not add package installation, target installation, index updates, or
downloads to `container_compile_agent()`; its runtime network is deliberately
`none`.

The local helper tag is an input cache key, not an OCI content digest. Two cold
Docker builds may have different image IDs because registry/index and layer
metadata are materialization outputs. Cargo verifies every registry package
against `Cargo.lock`, and the nightly/release qualification boundary is the
specific helper image materialized by that run: after this one guarded fetch
edge, the binary build is locked, offline, and network-denied. Do not claim
byte-for-byte reproducible helper images unless every remaining registry and
layer byte is independently pinned.

The selected image is a minimal Alpine image and has no Bash. Many common
utilities (`file`, `less`, `vim`, etc.) are NOT available. Runtime shell
commands must be POSIX `sh` and use only the BusyBox tools already present in
the materialized image.

**Lesson learned**: using `file /output/binary` to verify compiled binaries failed because `file` is not in slim images. Replaced with `ls -l` which is always available and still confirms the copy succeeded. The real validation (existence + non-zero size) is done in Python after the container exits.

**Rule**: never assume a command exists in a slim container image. Stick to coreutils or install what you need explicitly.

## Clock skew workaround

All asset dependency-helper `apt-get update` calls use
`-o Acquire::Check-Valid-Until=false -o Acquire::Check-Date=false` against the
config-owned HTTPS Debian snapshot to handle container VM clock drift.
Without this, apt rejects Release files whose timestamp is in the future relative to the VM's clock.
This can occur with any container VM backend on macOS.

Files affected:
- `Dockerfile.kernel-dependencies.j2`
- `Dockerfile.rootfs-dependencies.j2`

---
name: build-images
description: Building Capsem VM images from profile-owned inputs. Use when working on profile package files, Dockerfile templates, kernel or rootfs builds, or the capsem-builder backend.
---

# Building VM Images

## Overview

Capsem image builds are profile-led.

- `config/profiles/<profile_id>/profile.toml` is the profile ledger.
- Profile sibling files own packages, MCP declarations, rule files, detection
  files, tips, build-time hooks, and packaged guest root seed files.
- `capsem-admin` validates profile-owned inputs and materializes the generated
  backend build workspace.
- The Python builder backend renders Docker templates and emits assets, build
  ledgers, and OBOMs only when invoked by the admin build rail. Do not add
  product truth directly to the backend image-spec path.

## Source Layout

Read `config/README.md` before changing this layout.

```
config/
  settings/               UI/application preferences and generated UI schema
  corp/                   Corporate source contracts and rule files
  docker/                 Dockerfile/build templates
  profiles/<profile_id>/
    profile.toml          Source ledger; no hash/size pins
    enforcement.toml      Profile enforcement rules
    detection.yaml        Profile Sigma detections
    mcp.json              Profile MCP declarations
    apt-packages.txt      Profile apt package input
    python-requirements.txt / python-requirements.lock
    npm-packages.txt / npm-package-lock.json
    build.sh              Profile image build hook
    tips.txt              Profile guest tips
    root/                 Guest / seed, projected by capsem-init
cache/target/config/            Generated runtime config with asset/file evidence
cache/target/assets/            Generated VM assets
cache/target/packages/          Generated native packages
guest/artifacts/          Core guest payloads: init, doctor, diagnostics, bench
```

The materialized backend workspace may contain generated package-set files and
profile build scripts. Treat those as implementation details, not authoring
surfaces. The workspace is never a config root and never a second profile
catalog.

`capsem-admin` is a tool, not a config authority. It validates, materializes,
builds, and checks the profile/corp/settings contracts; it must not grow
scaffolding commands that invent profile, MCP, AI provider, package, or rule
truth outside `config/profiles`, `config/corp`, and `config/settings`.
Do not add admin config roots, guest config roots, settings metadata, provider
registries, or backend-owned profile catalogs as product truth. `schema`
validates one contract, `catalog` lists materialized profile instances, and UI
metadata only helps render settings.

## CLI commands

```bash
just _build-assets code [arch]                # Profile-derived asset rebuild
just _build-kernel arm64 code                 # Kernel slice
just _build-rootfs arm64 code                 # Rootfs slice
uv run --project build_system --frozen capsem-builder audit                  # Parse trivy/grype vulnerability output
```

Use admin/just recipes for all product image work. `capsem-builder` is a
backend helper only; it must not expose or document public `build`, `validate`,
`inspect`, `mcp`, render-only, or dry-run rails for profile/image authoring.
`capsem-admin image build` may call private Python modules such as
`capsem.builder.image_build_backend`; agents must not make those modules public
CLI contracts.

## Building assets

Full rebuild (kernel + rootfs):
```bash
just _build-assets    # Runs doctor + validate + build for host arch
```

Individual templates:
```bash
just _build-kernel arm64
just _build-rootfs arm64
```

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

Rootfs EROFS settings are profile-derived. The approved release default
is EROFS with `lz4hc` compression level 12.

`config/docker/image/build.toml [build.rootfs]` also owns two independent
release budgets: the exported tar ceiling and the final EROFS ceiling. The
builder checks the first before compression and the second before writing the
artifact ledger. It also scans every exported member against the configured
forbidden prefixes. Keep Ollama accelerator families forbidden under both
`/usr/lib/ollama` and `/usr/local/lib/ollama`: vendor archives have moved the
bundle between those roots, and the guest exposes no accelerator device. A
profile hook should remove the payload for efficiency, while the exported-tree
scan remains the fail-closed proof. Never raise a size ceiling to accommodate
an unexplained image jump; inspect the rootfs composition first.

## Build Ledger

Each per-arch build emits `build-ledger.log` JSONL. The
`rootfs.config_inputs` record captures declared profile package inputs,
rendered rootfs package lists, profile root/build-script inputs, EROFS config,
git revision, and project version. Installed-package/component truth belongs in
the CycloneDX OBOM, not the build ledger.

## Profile Source And Generated Evidence

Profile sibling files are ledgered source inputs, but agents must not add or
hand-edit `hash` or `size` fields in `profile.toml`. If editing
`apt-packages.txt`, `python-requirements.txt`, `npm-packages.txt`, `build.sh`,
rules, MCP declarations, tips, or root seed files makes
`capsem-admin profile check` fail, fix the source contract or the
validation/materialization rail with tests. Do not "just fix the hash" in TOML.

Generated runtime asset URLs/hashes belong in `cache/target/config` after
`capsem-admin profile materialize`, not in checked-in source TOML. Profile
materialization must recopy descriptor files and `root/` payloads from source
on every run; stale generated roots are a release blocker, not a cache.

## Adding packages to the VM

1. Edit the profile-owned package file, for example
   `config/profiles/code/apt-packages.txt`,
   `python-requirements.txt`, or `npm-packages.txt`. Python and npm direct
   selections must be exact, and their checked-in integrity locks must be
   regenerated together; never install from the direct list alone.
2. Run the admin/profile validation path.
3. Run `just _build-assets code` to rebuild the rootfs.
4. Verify with `capsem-doctor` inside a booted VM.

Do not edit generated Dockerfiles. Docker templates live under `config/docker/`.

## Adding a guest CLI/tool

There are no image-owned AI providers. A CLI/tool exists only if the active
profile declares the package/build hook and any required guest root seed files.

1. Add package input to the profile package files, or add build-time shell work
   to profile-owned `build.sh`.
2. Add config files under `config/profiles/<profile_id>/root/` so they project
   into the VM at boot.
3. Add MCP declarations to profile-owned `mcp.json` when relevant.
4. Add network/model/security behavior through profile/corp rules, not builder
   provider config.
5. Let the credential broker plugin capture/materialize credentials at runtime;
   do not add settings-owned boot secrets.
6. Rebuild with `just _build-assets code` and verify with `capsem-doctor`.

`build.sh` is executed only while constructing the rootfs image. It is the
right place for digest-bound official binary archives such as Claude, AGY, or
Ollama when they cannot be represented as apt/npm/Python package inputs. Never
pipe or invoke a mutable installer endpoint. Install verified runtime binaries
under system paths such as `/usr/local/bin`; anything left only under `/root`
can be hidden by the runtime overlay.

## Profile `build.sh` contract

Remember this rail when touching profile image contents:

- `config/profiles/<profile_id>/build.sh` is a profile-owned build hook.
- It runs inside the rootfs Docker build, before the EROFS image is produced.
- It does not run during native glow-up, service startup, VM boot, or user
  session creation.
- It is for image construction work that cannot be cleanly expressed through
  `apt-packages.txt`, `python-requirements.txt`, or `npm-packages.txt`.
- It may install public runtime tools such as Claude, AGY, and Ollama into
  stable system paths.
- It is not a second profile format, provider registry, runtime settings file,
  credential injection path, or local developer repair script.
- It must not bake credentials, per-user state, corp policy, rules, MCP
  decisions, or runtime settings.
- The owning `profile.toml` must reference it through `[files.build]`; the
  descriptor hash/size is refreshed by the profile-derived build rail, never by hand.
- Changing `build.sh` changes future rootfs assets only. Rebuild assets through
  the profile-derived just/admin-tool rail before claiming a VM contains the
  change.
- The same profile materialization path must be used locally and in CI; no
  one-off Docker or installer path is release proof.
- Verification must be black-box: boot the rebuilt profile image, run the tool
  from the VM, and inspect the generated session evidence when the tool should
  produce network, model, MCP, file, process, or credential events.

Decision rule:

- Normal Debian package: use `apt-packages.txt`.
- Normal Python package: use `python-requirements.txt`.
- Normal npm package: use `npm-packages.txt`.
- Vendor shell installer, binary tarball, wrapper creation, or cleanup that must
  happen while baking the immutable rootfs: use `build.sh`.
- Anything that depends on user/corp/runtime state: do not use `build.sh`.

## Dockerfile templates

Templates live in `config/docker/`:
- `Dockerfile.rootfs-dependencies.j2` -- snapshot-selected Debian packages and
  the profile's network-resolved Python/npm/vendor inputs;
- `Dockerfile.kernel-dependencies.j2` -- snapshot-selected kernel toolchain and
  the SHA-256-verified kernel archive;
- `Dockerfile.rootfs.j2` -- network-denied first-party rootfs assembly;
- `Dockerfile.kernel.j2` -- network-denied kernel compilation, initrd assembly,
  and vmlinuz extraction.

`asset-dependencies` is the visible resumable frontier between those pairs.
The gate materializes one input-keyed helper for every selected
profile/architecture/template, validates its platform and identity label, and
passes only its exact image ID to the source build. Source builds always use
BuildKit network `none` and never use the remote CI cache. A carried frontier
must revalidate every helper; it must not silently rebuild inside the sealed
lane. The dependency helpers reuse the one checked-in Debian snapshot
authority rather than the mutable sources inherited from the base image.

Templates use Jinja2 with variables from the admin-materialized profile image
workspace. Do not add a second preview rail for product truth; if a build input
needs validation, add it to the normal profile/admin validation path.

Every architecture's `base_image` is required to be its immutable
`repository@sha256:<child-manifest>` identity. Do not use a mutable tag or the
multi-platform index. `capsem-gate` materializes a missing exact child through
the Docker daemon before the cross-execution probe and build lanes; keep this
on the ordinary guarded Docker runner, not the host-process egress broker.

---

# Builder Internals (for modifying the builder itself)

## Architecture: Profile -> admin materialization -> Pydantic -> context dict -> Jinja2 -> Dockerfile

The data flows through four layers:

1. **Profile ledger** (`config/profiles/<id>/profile.toml`) and profile-owned
   sibling files.
2. **capsem-admin** validates and materializes a backend build workspace.
3. **Pydantic models** (`src/capsem/builder/models.py`) parse that workspace.
4. **Context dict** (`src/capsem/builder/docker.py`) feeds Jinja2 templates.
5. **Jinja2 templates** (`config/docker/`) produce Dockerfiles.

### Key files

| File | Role |
|------|------|
| `src/capsem/builder/models.py` | All Pydantic models (enums, configs, top-level `GuestImageConfig`) |
| `src/capsem/builder/config.py` | Backend loader for admin-materialized build workspaces |
| `src/capsem/builder/docker.py` | Context builders (`_rootfs_context`, `_kernel_context`), rendering, build execution |
| `src/capsem/builder/image_build_backend.py` | Private admin-invoked image build backend; not a public CLI |
| `config/docker/Dockerfile.rootfs.j2` | Rootfs Dockerfile template |
| `config/docker/Dockerfile.kernel.j2` | Kernel Dockerfile template |
| `src/capsem/builder/validate.py` | Validation rules (E001-E302, W001-W012) |
| `src/capsem/builder/cli.py` | Click CLI entry points |

### Context dict (rootfs template variables)

`_rootfs_context()` in `docker.py` builds the dict passed to `Dockerfile.rootfs.j2`:

```python
{
    "arch": ArchConfig,           # Per-arch settings (docker_platform, rust_target, etc.)
    "arch_name": str,             # "arm64" or "x86_64"
    "apt_packages": list[str],    # Materialized from profile apt-packages.txt
    "python_packages": list[str], # Materialized from profile python-requirements.txt
    "python_install_cmd": str,    # e.g. "uv pip install --system --break-system-packages"
    "npm_packages": list[str],    # Materialized from profile npm-packages.txt
    "profile_root_seed": bool,    # Whether profile-root/ is copied into the image
    "profile_build_script": bool, # Whether profile-build.sh is executed
    "npm_prefix": str,            # e.g. "/opt/ai-clis"
    "guest_binaries": list[str],  # ["capsem-pty-agent", "capsem-net-proxy", "capsem-mcp-server"]
}
```

### Kernel context dict

```python
{
    "arch": ArchConfig,
    "arch_name": str,
    "kernel_version": str,  # exact checked-in release, e.g. "6.6.130"
    "kernel_sha256": str,   # verified before the source archive is extracted
}
```

## How to: Change a shipped CLI

1. Prefer a profile package file (`apt-packages.txt`, `npm-packages.txt`, or
   `python-requirements.txt`) when the tool has a normal package manager.
2. Use profile-owned `build.sh` when the vendor ships an official shell
   installer. The build hook runs during rootfs construction only.
3. Make sure binaries end up in stable system paths such as `/usr/local/bin`.
4. Validate and materialize through `capsem-admin`; if the rail cannot express
   the change, implement it with tests first.
5. Add or update capsem-admin materialization tests and Docker context tests.
6. Rebuild: `just _build-assets code` and verify with `capsem-doctor`.

Ollama is intentionally installed by `config/profiles/<id>/build.sh`, not by a
VM one-off command. That keeps Codex, Claude, AGY, and OpenAI-compatible local
testing available in every shipped profile image that declares the hook.

## How to: Add a new package to an existing set

1. Edit `config/profiles/<profile_id>/apt-packages.txt`,
   `python-requirements.txt`, or `npm-packages.txt`.
2. Validate and materialize through `capsem-admin`.
3. Keep the checked-in profile source free of generated hashes or sizes.
4. Rebuild: `just _build-assets <profile_id>`.

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

## Backend Workspace Schema

The backend workspace is generated by `capsem-admin`; do not author it by
hand for product behavior. Its install inputs are package-set TOML files:

```toml
[npm]
name = "Node Packages"
manager = "npm"
install_cmd = "npm install -g --prefix /opt/ai-clis"
packages = ["@scope/package"]
```

Profiles own CLI/tool selection. If an installer cannot be represented as a
package set, put it in `config/profiles/<profile_id>/build.sh`, reference it
from `[files.build]` in `profile.toml`, refresh pins with `capsem-admin`, and
rebuild through the admin/just rail. Do not add a provider registry under
backend-generated image workspaces.

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
7. Extract tool versions (`extract_tool_versions`)
8. Clean up container image

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

A profile release run compiles that graph **three** times -- co-work rootfs,
code rootfs, and `assets.pack-initrds` -- so emulation costs roughly forty
minutes per run.

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
The rootfs build runs apt, npm, and curl-based CLI installers concurrently --
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

The resource check lives in `src/capsem/builder/doctor.py`:
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

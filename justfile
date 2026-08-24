# Capsem Justfile
#
# The public surface is intentionally small and locked by
# config/public-surface.toml. New public recipes require explicit approval.
#
#   dev [ui|frontend|tui]  development surfaces
#   build                  desktop application + embedded frontend
#   build-all              all host binaries, desktop app, and docs
#   build-docs             documentation and marketing sites
#   shell / exec           temporary VM interaction
#   run-service            idempotent local daemon
#   logs [sandbox|failure] service, VM, or failure evidence
#   doctor                 host, Docker/Colima, Tart, and asset readiness
#   fast-test              incomplete source feedback; never qualification
#   focus-test             one named functional group, optionally cold
#   install                build and install the complete local macOS product
#   test-clean             exceptional cold complete diagnostic
#   release-binaries       publish packages for one channel
#   release-profile        publish one channel/profile
#
# Underscore recipes are implementation detail. No workflow may call one:
# `tests/citadel/test_ci_calls_only_public_recipes.py` refuses it.

host_crates := "-p capsem-service -p capsem-process -p capsem -p capsem-tui -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray -p capsem-admin -p capsem-mock-server -p capsem-bench"
# Propagate Cargo.toml's version across the release cohort (capsem.gate.versions).
_stamp-version:
    @uv run capsem-gate stamp-version

# Build one profile's VM assets for one architecture: kernel, then rootfs.
build-assets arch profile="":
    just _build-kernel {{quote(arch)}} {{quote(profile)}}
    just _build-rootfs {{quote(arch)}} {{quote(profile)}}


# Host-crate unit tests against the Linux KVM backend, with coverage.
test-linux-rust:
    just _gate-linux-rust


# Qualify the candidate packages against the manifest-selected profiles.
qualify-binaries workspace_root:
    uv run capsem-gate qualify-binaries {{quote(workspace_root)}}


# Qualify one profile's built assets against the selected binary.
qualify-assets input_dir profile workspace_root activation_ready:
    uv run capsem-gate qualify-assets {{quote(input_dir)}} {{quote(profile)}} {{quote(workspace_root)}} --activation-ready {{quote(activation_ready)}}


# Replay a release qualification lane locally, against a cohort built here.
replay-release-lane lane="binaries":
    uv run python scripts/replay-release-lane.py --lane {{quote(lane)}}


# Build, test, and publish only Capsem binaries/packages for one channel.
release-binaries channel source_commit force="false":
    uv run capsem-gate release-binaries {{quote(channel)}} {{quote(source_commit)}} --force {{quote(force)}}


# Build, test, and publish exactly one channel/profile through capsem-admin.
release-profile channel profile source_commit force="false":
    uv run capsem-gate release-profile {{quote(channel)}} {{quote(profile)}} {{quote(source_commit)}} --force {{quote(force)}}


# Compile all host binaries
_build-host:
    cargo build {{host_crates}}

# Codesign all host binaries (macOS only, needed for Virtualization.framework)
_sign: _build-host
    uv run capsem-gate sign


# Ensure capsem-service daemon is running with the current binary.
# Kills any existing dev-owned instance (via pidfile -- never pkill-by-name)
# and relaunches fresh. Honors CAPSEM_HOME / CAPSEM_RUN_DIR env vars so
# `just test` and `just vm-smoke` run against an isolated test home
# without ever touching the user's locally installed capsem.
_ensure-service: _sign
    uv run capsem-gate ensure-service


# Start service daemon + Tauri GUI with hot-reloading
_dev-ui: _ensure-dev-ready _pnpm-install run-service
    uv run capsem-gate dev ui


# Frontend-only dev server with mock data (no Tauri/VM needed)
_dev-frontend: _pnpm-install _generate-settings
    cd frontend && pnpm run dev

# Build the Tauri desktop app (capsem-app) with a fresh frontend bundle.
# IMPORTANT: the Tauri binary embeds frontend/dist at cargo compile time via
# tauri::generate_context!(), so rebuilding only the frontend has no effect
# on the running binary. This recipe keeps the two in lockstep.
#   just build          # debug binary at ./target/debug/capsem-app
#   just build release  # release binary at ./target/release/capsem-app
_build-ui profile="debug": _pnpm-install _generate-settings
    uv run capsem-gate build-ui {{quote(profile)}}


# Frontend release gate used by Sprinty and docs.
# Build both public documentation surfaces.
build-docs: _pnpm-install
    bash scripts/check-web-surface.sh docs
    bash scripts/check-web-surface.sh site

# Select one deliberate development surface.
dev surface="ui": _ensure-dev-ready _pnpm-install
    uv run capsem-gate dev {{quote(surface)}}


# Build the desktop application with its embedded frontend.
build profile="debug":
    just _build-ui {{quote(profile)}}

# Build every host binary plus the desktop and documentation surfaces.
# VM/release assets remain profile-owned and are built by the canonical test
# and release workflows, not hidden inside a routine source build.
build-all profile="debug":
    just build {{quote(profile)}}
    just _build-host
    just build-docs

# Start service daemon + boot temporary VM + shell (~10s after first build)
shell: _prepared-runtime _ensure-service
    uv run capsem-gate shell


# Start capsem-service daemon (builds, signs, launches or reuses running instance)
run-service: _prepared-runtime _ensure-service

# Execute a command in a fresh temporary VM (auto-provisioned and destroyed)
# Usage: just exec "echo hello"   or   just exec "ls -la"
exec +CMD: run-service
    uv run capsem-gate exec -- {{quote(CMD)}}



# Build kernel only for one profile/arch (CI-facing primitive).
_build-kernel arch profile="":
    uv run capsem-gate build-assets {{quote(profile)}} {{quote(arch)}} --template kernel


# Build rootfs only for one profile/arch (CI-facing primitive).
_build-rootfs arch profile="":
    uv run capsem-gate build-assets {{quote(profile)}} {{quote(arch)}} --template rootfs


# VM asset rebuild (kernel + rootfs). Profile is mandatory. Optional second arg
# restricts to one arch.
_build-assets profile="" arch="":
    uv run capsem-gate build-assets {{quote(profile)}} {{quote(arch)}}


# Ironbank VM asset gate. This is the superset owner for the image-build work
# performed by release-assets.yaml: every checked-in profile, both published
# architectures, the exact CI-facing build primitives, generated-manifest
# validation, and a real shell marker from each profile-owned host-arch image.
# Outputs stay under target/ so the gate never mutates the developer's assets/.
_gate-assets: _bootstrap _install-tools _generate-settings _sign
    @uv run capsem-gate assets

# Run ALL tests: Rust + frontend + Python + injection + integration + bench + cross-compile + install e2e. No shortcuts.
#
# Runs against an isolated CAPSEM_HOME under target/test-home/ so the suite
# never kills or mutates the user's locally installed capsem. The flock is
# still honored for multi-agent coordination but now lives inside the test
# home, not the shared ~/.capsem/run.
_bootstrap:
    sh {{quote(justfile_directory() / "bootstrap.sh")}} -y

# Build output is reused between runs by default, which is what makes a second
# commit cost minutes rather than an hour; `buildcache` explains how. This is
# the escape hatch, for when a local pass has to mean a pass on a cold runner:
# diagnose with nothing reused, compiling every artifact from nothing. Release
# qualification belongs to the release rails; agents must not run this by habit.
test-clean source_commit="":
    @uv run capsem-gate candidate {{quote(source_commit)}} --clean-build

# After the source-only fast gate passes, local composition constructs every
# artifact family before running the remaining modules used by release CI.
_test-candidate:
    uv run capsem-gate test-candidate


# Parser errors, source contracts, dependency vulnerabilities, lint, Clippy,
# and every JavaScript/web check run before Colima, bootstrap, artifacts, or
# VMs. This is private composition, not a public release shortcut.
_test-source-checks:
    uv run capsem-gate test-fast
    just _check-generated-settings
    just _test-release-contracts

_test-compiled-checks: _clean-stale _check-generated-settings
    just _bound-docker-test-storage
    uv run capsem-gate test-static

_test-artifacts:
    uv run capsem-gate test-artifacts

_test-profile-artifacts input_dir profile:
    uv run capsem-gate test-profile-artifacts {{quote(input_dir)}} {{quote(profile)}}

_test-functional: _generate-settings
    uv run capsem-gate test-functional

_test-glowup:
    uv run capsem-gate test-glowup

_test-release-contracts: _release-site-pnpm-install
    uv run capsem-gate test-release-contracts

# Require Docker headroom without discarding content-addressed compiler caches.
# Cargo validates cached artifacts against the current source inputs; bounded
# reuse speeds forward fixes without weakening the before/after tree invariant.

_test-recipes:
    uv run python -m pytest tests/capsem-recipes/ -v --tb=short -m recipe

# Build the capsem-host-builder Docker image (cached, only rebuilds changed layers).

# Execute the portable Linux host-crate suite through one checked-in runner.
# Linux CI calls this recipe natively. Mac-local `just test` calls it through
# capsem-host-builder so cfg(target_os = "linux") tests are not CI-only.
_gate-linux-rust:
    uv run capsem-gate linux-rust


# Build the Linux parity base image, with network, before a sealed run needs it.
# The lane refuses to build this itself: its tag is keyed by Cargo.lock,
# rust-toolchain.toml and frontend/pnpm-lock.yaml, so a dependency bump re-keys
# it, and resolving that inside the run would turn a `--network none` lane into
# a multi-gigabyte network build at minute four. `capsem-gate linux-rust` names
# this recipe when the image is missing.
_warm-linux-rust-base:
    uv run capsem-gate warm-linux-rust-base


# Run the production release SBOM generator over the exact current-version
# packages built by the canonical gate. Mac runs cover one .pkg plus both .deb
# architectures; native Linux qualification covers both .deb architectures.
_gate-host-package-sbom:
    uv run capsem-gate host-sbom


# Remove cross-compilation image and cached volumes.
_clean-host-image:
    @uv run capsem-gate storage clean --scope all

_release-completed-docker-rails:
    @uv run capsem-gate storage release completed-docker-rails

_release-completed-package-rails:
    @uv run capsem-gate storage release completed-package-arm64
    @uv run capsem-gate storage release completed-package-x86_64

_release-deferred-install-target:
    @uv run capsem-gate storage release deferred-install-target

# repack-deb.sh below reads the materialized profile catalog from target/config,
# so this recipe owns filling it rather than leaving each call site to remember.
# Release CI never enters here: it consumes an already-built package with its
# staged profile cohort, so nothing it pulled can be clobbered.
# Build the full Linux release in a container (agent + deb).
# Uses the private cached capsem-host-builder image.
# Supports arm64 and x86_64 via native cross-compilation (no QEMU).
#
# The image runs natively on the host arch and cross-compiles via
# Rust --target + multiarch system libs. Named volumes cache cargo
# registry and build artifacts between runs. CARGO_TARGET_DIR=/cargo-target
# inside the container isolates from host macOS target/ directory.
#
# CI vs local divergences (keep in sync when changing either):
#   - CI runs on bare ubuntu runners; this runs in capsem-host-builder via docker
#   - Tauri signing keys: CI from secrets, local from private/tauri/
#   - See: .github/workflows/release.yaml build-app-linux job
_cross-compile arch="": _clean-stale _check-assets _generate-settings _materialize-config
    @uv run capsem-gate cross-compile {{quote(arch)}}

# Generate settings schema/UI metadata and frontend mock data.
_generate-settings:
    bash scripts/generate-settings.sh


# Generate tracked settings outputs and fail if the generator changed them.
# This is the local equivalent of CI's generate-then-git-diff drift gate, but
# compares before/after content so an intentional already-generated worktree
# change can still be tested before it is committed.
_check-generated-settings:
    bash scripts/check-generated-settings.sh {{quote(justfile_directory())}}


# Incomplete source feedback, and nothing else. The gate command owns the plan;
# this public recipe only makes its scope and the next supported commands clear.
#
# It was called `smoke`, which described neither half of what it did. Focused
# runtime proof now belongs to `focus-test functional`; there is no second
# public VM-smoke spelling for agents to stack beside it.
fast-test:
    @echo "Agent: incomplete feedback only; use 'just focus-test <group>' for targeted proof, or 'just release-profile ...' / 'just release-binaries ...' for qualification."
    uv run capsem-gate test-fast


# One existing gate owner, selected by a closed group name. `clean` discards
# reusable build output for the exceptional stale-cache reproduction.
focus-test group mode="reuse":
    @uv run capsem-gate focus-test {{quote(group)}} {{quote(mode)}}

# Build the complete installable product and install that exact local package
# onto this Mac for hands-on testing. This is not release qualification.
install:
    @uv run capsem-gate local-install


# Measure performance and record it. `just bench` takes every dimension that
# has a collector; `just bench <dim>...` takes the named ones.
#
# Capsem had no such entry point: nine Criterion targets existed and nothing
# ran them, and a release once failed on a gateway CPU figure that no run had
# ever recorded.
bench *dimensions: _prepared-runtime
    @uv run capsem-gate bench {{ quote(dimensions) }}

# The dev loop: only the dimensions that need no guest, bounded so it stays a
# dev loop. Records like any other run; never evidence.
bench-quick *dimensions:
    @uv run capsem-gate bench --quick {{ quote(dimensions) }}

# What every measured subject reads, and how it has moved.
bench-report:
    @uv run capsem-gate bench-report


# Run install e2e tests in Docker (Linux + systemd).
# Depends on _pnpm-install: the install suite builds the release site inside
# the container, and CI's test-install job enables the pnpm cache -- whose
# post-job save step fails on a store that was never created.
_gate-install: _pnpm-install
    @uv run capsem-gate install

# Check dev tools and dependencies. Pass "fix" to auto-fix.
doctor fix="": _pnpm-install
    @uv run capsem-gate doctor
    @scripts/doctor-common.sh {{ if fix == "fix" { "--fix" } else { "" } }}

# View service logs, a sandbox's logs, or the latest preserved test failure.
# `just logs`, `just logs <sandbox-id>`, `just logs failure`.
logs target="":
    uv run capsem-gate logs {{quote(target)}}


# Remove stale rootfs copies, orphan UDS sockets, and trim bloated incremental caches.
# See scripts/clean_stale.py for implementation (tested: tests/capsem-cleanup-script/).
_clean-stale:
    @uv run python3 scripts/clean_stale.py

# Auto-prune Docker after builds: stopped containers, dangling images, build cache >7d.
# Keeps named volumes (cross-compile cargo caches) and recent build cache for fast rebuilds.
_docker-gc:
    @uv run capsem-gate storage gc

# Enforce release-rail headroom while preserving content-addressed Cargo,
# registry, rustup, and recent BuildKit caches that make forward fixes fast.
_bound-docker-test-storage:
    @uv run capsem-gate storage ensure-space default candidate-boundary

# Explicit deep cleanup for a human-requested cold rebuild. The canonical gate
# deliberately does not call this recipe.
_clean-docker-test-targets:
    @uv run capsem-gate storage clean --scope working --rail default

# --- Internal helpers (hidden from `just --list`) ---

# Run doctor automatically on first use (creates .dev-setup sentinel)
_ensure-dev-ready:
    uv run capsem-gate dev-ready


# Auto-install Rust targets, components, and cargo tools
_install-tools:
    uv run capsem-gate install-tools


# Verify VM assets exist (vmlinuz, initrd.img, rootfs)
_check-assets:
    uv run capsem-gate check-assets


_pnpm-install:
    uv run capsem-gate install-node


_release-site-pnpm-install:
    cd release-site && CI=true pnpm install --frozen-lockfile

_frontend: _pnpm-install
    bash scripts/check-web-surface.sh frontend-build

_compile: _frontend _clean-stale
    cargo build -p capsem

_sign-release: _compile
    uv run capsem-gate sign


_pack-initrd:
    uv run capsem-gate pack-initrd


_materialize-config:
    bash scripts/materialize-config.sh


# One bootable local runtime: verified assets, the initrd repacked around the
# current guest binaries, and a materialized profile catalog. `test` and
# `vm-smoke` both need exactly this before they can run anything against a VM,
# so they name it once instead of repeating the sequence.
_prepared-runtime: _check-assets _pack-initrd _materialize-config

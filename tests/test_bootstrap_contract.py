from __future__ import annotations

import subprocess
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _gate_labels(name: str = "candidate") -> tuple[str, ...]:
    """Every step of a command's plan, in graph order. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels(name)


def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text()


def test_bootstrap_always_checks_project_skills_and_site_shape() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "check_bootstrap_shape" in bootstrap
    assert "check_bootstrap_shape\n\n# Ask the developer" in bootstrap
    for link in [
        ".agents/skills",
        ".claude/skills",
        ".codex/skills",
        ".cursor/skills",
        ".gemini/skills",
    ]:
        assert link in bootstrap
        assert "../skills" in bootstrap
    for required_file in [
        "skills/dev-sprint/SKILL.md",
        "skills/dev-testing/SKILL.md",
        "skills/dev-capsem/SKILL.md",
        "skills/ironbank/SKILL.md",
        "skills/frontend-design/SKILL.md",
        "site/package.json",
        "site/astro.config.mjs",
        "site/src/components/FAQ.svelte",
        "site/src/lib/data.ts",
    ]:
        assert required_file in bootstrap
    assert "find skills -mindepth 2 -name SKILL.md" in bootstrap


def test_bootstrap_runs_full_doctor_fix_without_a_parallel_check_mode() -> None:
    bootstrap = _read("bootstrap.sh")

    assert '"$SCRIPT_DIR/scripts/doctor-common.sh" --fix' in bootstrap
    assert "doctor-common.sh --check" not in bootstrap
    assert "dry-run" not in bootstrap.lower()


def test_bootstrap_uses_colima_exit_status_not_running_text() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "colima status >/dev/null 2>&1" in bootstrap
    assert 'colima status 2>&1 | grep -qi "running"' not in bootstrap


def test_bootstrap_repairs_stale_live_rosetta_registration_before_docker_probe() -> None:
    bootstrap = _read("bootstrap.sh")

    registration = "colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta"
    assert registration in bootstrap
    assert "colima restart" in bootstrap
    assert bootstrap.index(registration) < bootstrap.index("docker info >/dev/null")


def test_bootstrap_waits_for_container_dns_after_colima_restart() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "docker run --rm --pull=missing alpine:3.20 getent hosts ghcr.io" in bootstrap
    assert "Docker DNS did not become ready" in bootstrap
    assert "for attempt in $(seq 1 30)" in bootstrap


def test_linux_bootstrap_owns_host_packages_node_docker_and_kvm_access() -> None:
    bootstrap = _read("bootstrap.sh")
    linux = _read("scripts/bootstrap-linux.sh")

    assert '. "$SCRIPT_DIR/scripts/bootstrap-linux.sh"' in bootstrap
    assert 'bootstrap_linux "$SCRIPT_DIR" "$ASSUME_YES"' in bootstrap
    assert "[SKIP] docker (install via your package manager" not in bootstrap
    assert "[SKIP] docker daemon" not in bootstrap

    # Ubuntu/Debian host prerequisites cover the native fast gate as well as
    # the image builder. Buildx is the Ubuntu archive package name; the
    # Docker-CE repository's *-plugin name is only a fallback.
    for package in [
        "build-essential",
        "bubblewrap",
        "cpio",
        "pkg-config",
        "libssl-dev",
        "libgtk-3-dev",
        "libwebkit2gtk-4.1-dev",
        "libayatana-appindicator3-dev",
        "libxdo-dev",
        "librsvg2-dev",
        "musl-tools",
        "docker.io",
        "docker-buildx",
        "acl",
    ]:
        assert package in linux
    assert "apt-get update" in linux
    assert "apt-get install" in linux
    assert "systemctl enable --now docker" in linux

    # Durable group membership helps future shells. vhost-vsock keeps a narrow
    # current-user ACL. KVM follows the mode used in Linux CI because logind
    # removes named KVM ACLs on the first VM lifecycle, before a second VM can
    # start in the same bootstrap session.
    assert 'usermod -aG docker "$CAPSEM_BOOTSTRAP_USER"' in linux
    assert 'usermod -aG kvm "$CAPSEM_BOOTSTRAP_USER"' in linux
    assert 'setfacl -m "u:$CAPSEM_BOOTSTRAP_USER:rw" /var/run/docker.sock' in linux
    assert "chmod 0666 /dev/kvm" in linux
    assert 'setfacl -m "u:$CAPSEM_BOOTSTRAP_USER:rw" /dev/vhost-vsock' in linux
    assert 'KERNEL=="kvm", GROUP="kvm", MODE="0666", TAG-="uaccess"' in linux
    assert 'KERNEL=="vhost-vsock", GROUP="kvm", MODE="0660", TAG-="uaccess"' in linux
    assert "modprobe vhost_vsock" in linux
    assert "udevadm control --reload-rules" in linux
    assert "docker info" in linux
    assert "docker buildx version" in linux
    assert "bwrap --unshare-net" in linux
    assert '[ -r /dev/kvm ] && [ -w /dev/kvm ]' in linux
    assert '[ -r /dev/vhost-vsock ] && [ -w /dev/vhost-vsock ]' in linux

    doctor = _read("scripts/doctor-linux.sh")
    assert 'for device in /dev/kvm /dev/vhost-vsock' in doctor
    assert "--bind / / --dev-bind /dev /dev -- sh -c ': > /dev/null'" in linux
    assert "--bind / / --dev-bind /dev /dev -- sh -c ': > /dev/null'" in doctor
    assert "gate network namespace active (loopback only)" in doctor
    assert "run ./bootstrap.sh" in doctor

    # Linux does not accept whatever Node happens to be in the distribution.
    # The required major comes from the profile image config and the fetched
    # official tarball is checked against Node's SHA256 manifest.
    assert "config/docker/image/build.toml" in bootstrap
    assert "latest-v${CAPSEM_NODE_MAJOR}.x/SHASUMS256.txt" in linux
    assert "sha256sum" in linux
    assert 'CAPSEM_NODE_COMMAND=$(command -v node 2>/dev/null || true)' in linux
    assert '"$HOME/.local/bin/node"' in linux
    assert "refusing to replace unmanaged" in linux
    assert 'npm install --global pnpm@10 --prefix "$HOME/.local"' in bootstrap
    assert 'PNPM_MAJOR=${PNPM_VERSION%%.*}' in bootstrap
    assert '"$PNPM_MAJOR" = 10' in bootstrap
    assert "pnpm 10 is required after bootstrap" in bootstrap
    assert "uv run capsem-gate install-node" in bootstrap
    assert "uv sync --frozen" in bootstrap
    assert "cargo fetch --locked" in bootstrap
    assert "cd frontend && CI=true pnpm install" not in bootstrap

    # Daemon activation and a new socket are asynchronous on a fresh host.
    # Bootstrap waits for both the socket and post-ACL client access instead
    # of racing systemd and failing a valid installation.
    assert "CAPSEM_DOCKER_WAIT" in linux
    assert "CAPSEM_DOCKER_ACCESS_WAIT" in linux


def test_linux_bootstrap_node_major_parser_requires_one_shared_value() -> None:
    command = (
        '. scripts/bootstrap-linux.sh; '
        'capsem_linux_node_major config/docker/image/build.toml'
    )
    completed = subprocess.run(
        ["sh", "-c", command],
        cwd=PROJECT_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )

    assert completed.stdout.strip() == "24"


def test_bootstrap_rust_toolchain_parser_requires_the_checked_in_pin() -> None:
    command = (
        '. scripts/bootstrap-rust.sh; '
        'capsem_rust_toolchain rust-toolchain.toml'
    )
    completed = subprocess.run(
        ["sh", "-c", command],
        cwd=PROJECT_ROOT,
        check=True,
        text=True,
        capture_output=True,
    )

    assert completed.stdout.strip() == "1.97.1"


def test_bootstrap_installs_and_proves_the_exact_checked_in_rust_toolchain() -> None:
    bootstrap = _read("bootstrap.sh")
    rust = _read("scripts/bootstrap-rust.sh")
    doctor = _read("scripts/doctor-common.sh")

    assert '. "$SCRIPT_DIR/scripts/bootstrap-rust.sh"' in bootstrap
    assert 'capsem_rust_toolchain "$SCRIPT_DIR/rust-toolchain.toml"' in bootstrap
    assert '--default-toolchain "$CAPSEM_RUST_TOOLCHAIN" --profile minimal' in bootstrap
    assert 'capsem_ensure_rust_toolchain "$CAPSEM_RUST_TOOLCHAIN"' in bootstrap
    assert "capsem_expose_rustup_tools" in bootstrap
    expose = bootstrap.index("capsem_expose_rustup_tools")
    assert bootstrap.rfind('if [ "$(uname -s)" = "Linux" ]', 0, expose) >= 0
    assert 'rustup toolchain install "$CAPSEM_RUST_TOOLCHAIN" --profile minimal' in rust
    assert 'rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version' in rust
    assert 'for CAPSEM_RUST_TOOL in rustup rustc cargo' in rust
    assert '"$HOME/.local/bin/$CAPSEM_RUST_TOOL"' in rust
    assert "refusing to replace unmanaged" in rust

    # Doctor consumes the same checked-in pin and tests the exact selected
    # compiler. Merely finding a standalone cargo binary is not a Rust setup.
    assert '. "$SCRIPT_DIR/bootstrap-rust.sh"' in doctor
    assert 'capsem_rust_toolchain "$PROJECT_ROOT/rust-toolchain.toml"' in doctor
    assert 'rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version' in doctor
    assert 'target list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed' in doctor
    assert 'component list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed' in doctor
    assert "rustup target add --toolchain $CAPSEM_RUST_TOOLCHAIN" in doctor
    assert "rustup component add --toolchain $CAPSEM_RUST_TOOLCHAIN" in doctor


def test_bootstrap_exposes_rustup_proxies_to_the_agent_path(tmp_path: Path) -> None:
    rust_bin = tmp_path / "cargo-bin"
    local_bin = tmp_path / "home" / ".local" / "bin"
    rust_bin.mkdir()
    for tool in ("rustup", "rustc", "cargo"):
        proxy = rust_bin / tool
        proxy.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        proxy.chmod(0o755)

    completed = subprocess.run(
        [
            "sh",
            "-c",
            '. scripts/bootstrap-rust.sh; capsem_expose_rustup_tools; '
            'readlink "$HOME/.local/bin/rustup"; '
            'readlink "$HOME/.local/bin/rustc"; '
            'readlink "$HOME/.local/bin/cargo"',
        ],
        cwd=PROJECT_ROOT,
        env={"HOME": str(tmp_path / "home"), "PATH": f"{rust_bin}:/usr/bin:/bin"},
        check=True,
        text=True,
        capture_output=True,
    )

    assert completed.stdout.splitlines() == [
        str(rust_bin / "rustup"),
        str(rust_bin / "rustc"),
        str(rust_bin / "cargo"),
    ]
    assert all((local_bin / tool).is_symlink() for tool in ("rustup", "rustc", "cargo"))


def test_bootstrap_refuses_to_replace_an_unmanaged_rust_tool(tmp_path: Path) -> None:
    rust_bin = tmp_path / "cargo-bin"
    local_bin = tmp_path / "home" / ".local" / "bin"
    rust_bin.mkdir(parents=True)
    local_bin.mkdir(parents=True)
    for tool in ("rustup", "rustc", "cargo"):
        proxy = rust_bin / tool
        proxy.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        proxy.chmod(0o755)
    (local_bin / "rustup").write_text("owned elsewhere\n", encoding="utf-8")

    completed = subprocess.run(
        ["sh", "-c", ". scripts/bootstrap-rust.sh; capsem_expose_rustup_tools"],
        cwd=PROJECT_ROOT,
        env={"HOME": str(tmp_path / "home"), "PATH": f"{rust_bin}:/usr/bin:/bin"},
        check=False,
        text=True,
        capture_output=True,
    )

    assert completed.returncode != 0
    assert "refusing to replace unmanaged file" in completed.stderr


def test_linux_doctor_uses_ubuntu_buildx_name_and_enforces_node_major() -> None:
    doctor_linux = _read("scripts/doctor-linux.sh")
    doctor_common = _read("scripts/doctor-common.sh")

    assert 'apt) echo "sudo apt install docker-buildx"' in doctor_linux
    assert "required Node.js major" in doctor_common
    assert "config/docker/image/build.toml" in doctor_common
    assert "for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum flock zstd cpio" in doctor_common


def test_just_test_invokes_bootstrap_and_release_quality_gates() -> None:
    justfile = _read("justfile")
    web_gate = _read("scripts/check-web-surface.sh")

    # Quoted: a checkout under a path with a space split into two arguments
    # otherwise, so the recipe was not portable to `~/My Projects/capsem`.
    assert '_bootstrap:\n    sh {{quote(justfile_directory() / "bootstrap.sh")}} -y' in justfile
    # The gate is one plan now, so these are phases rather than recipe calls.
    labels = _gate_labels()
    assert any(label.startswith("fast.") for label in labels)
    assert "prepare.bootstrap" in labels
    assert "prepare.storage-budget" in labels
    assert "python.ruff" in labels
    assert "python.ty.strict" in labels
    for command in [
        "uv run capsem-builder validate-skills skills",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "bash scripts/check-web-surface.sh docs",
        "bash scripts/check-web-surface.sh site",
        "bash scripts/check-web-surface.sh release-site",
    ]:
        assert command in _gate_issues()
    for command in [
        "pnpm --dir frontend run check",
        "pnpm --dir frontend run test",
        "pnpm --dir frontend run build",
    ]:
        assert command in web_gate


def test_both_release_lanes_reuse_fail_closed_static_module() -> None:
    binary_workflow = _read(".github/workflows/release.yaml")
    profile_workflow = _read(".github/workflows/release-assets.yaml")
    fast_gate = _read(".github/workflows/fast-gate.yaml")

    assert "uses: ./.github/workflows/fast-gate.yaml" in binary_workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in profile_workflow
    assert "run: just _test-fast" in fast_gate
    assert "run: just _test-static" in fast_gate
    assert "run: just test" not in binary_workflow
    assert "run: just test" not in profile_workflow
    assert "cargo clippy --workspace --all-targets -- -D warnings" in _gate_issues()


def test_frontend_release_gate_is_owned_by_the_canonical_test() -> None:
    justfile = _read("justfile")
    web_gate = _read("scripts/check-web-surface.sh")

    assert "\ntest-frontend:" not in justfile
    block = justfile.split("\n_test-candidate:", 1)[1].split("\n_build-host-image:", 1)[0]
    assert "bash scripts/check-web-surface.sh frontend" in block
    assert "pnpm --dir frontend run check" in web_gate
    assert "pnpm --dir frontend run test" in web_gate
    assert "pnpm --dir frontend run build" in web_gate

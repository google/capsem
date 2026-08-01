"""Install package asset-payload contract tests."""

import contextlib
import errno
import functools
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest
from blake3 import blake3

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _skill_text(skill_path: Path) -> str:
    """Read a skill plus the reference files it explicitly links."""
    skill_dir = skill_path.parent
    main = skill_path.read_text(encoding="utf-8")
    parts = [main]
    for relative in dict.fromkeys(re.findall(r"`(references/[A-Za-z0-9_./-]+\.md)`", main)):
        reference = (skill_dir / relative).resolve()
        assert reference.is_relative_to(skill_dir.resolve())
        assert reference.is_file(), f"missing linked skill reference: {relative}"
        parts.append(reference.read_text(encoding="utf-8"))
    return "\n".join(parts)


#: Recipes whose behaviour moved into the gate, and the command that now owns
#: it. These contracts are about what the gate *does*; when the doing moved
#: from a shell body into a plan, the place to read it moved with it.
DISPATCHED = {
    "test:": ("candidate", {}),
    "_test-candidate:": ("test-candidate", {}),
    "_gate-assets:": ("assets", {}),
    "_gate-install:": ("install", {}),
    "_cross-compile": ("cross-compile", {"arch": "arm64"}),
    "_prove-linux-deb:": ("prove-deb", {}),
    "_test-install-harness-preflight:": ("install-image", {}),
    "_docker-gc:": ("storage", {"action": "gc", "rail": None}),
    "_ensure-service:": ("ensure-service", {}),
    "_pack-initrd:": ("pack-initrd", {}),
    "_stamp-version:": ("stamp-version", {}),
    "_gate-host-package-sbom:": ("host-sbom", {}),
    "_gate-linux-rust:": ("linux-rust", {}),
    "_build-assets": ("build-assets", {"profile": "code", "arch": "arm64", "template": "all"}),
    "_check-assets:": ("check-assets", {}),
}


def _planned(command: str, **args) -> str:
    return _planned_cached(command, tuple(sorted(args.items())))


@functools.cache
def _planned_cached(command: str, args: tuple) -> str:
    """Every command a gate command actually issues, with real argv.

    The plan is *run* against a recording runner rather than merely described.
    Much of this work is still behind `Call`, which renders as prose -- so a
    description would answer "build the install-test image" where the contract
    is about the docker arguments underneath. Running it records those without
    executing anything.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate import config as gate_config
    from capsem.gate.command import GateCommand
    from capsem.gate.context import Context

    runner = RecordingRunner(PROJECT_ROOT)
    try:
        plan = (
            GateCommand.registry[command](
                runner,
                argparse.Namespace(
                    dry_run=False, graph=False, timing=False, **dict(args)
                ),
            )
            ._describe()
        )
        rendered = plan.describe()
        # A step that needs a machine fails here; what it issued before failing
        # is still the evidence these contracts are about.
        with contextlib.suppress(Exception):
            plan.run(Context(runner, gate_config.load(PROJECT_ROOT)))
        return rendered + "\n" + "\n".join(runner.rendered) + "\n" + "\n".join(runner.notes)
    except Exception as exc:
        return f"<plan for {command} unavailable: {exc}>"


def _just_recipe_block(name: str) -> str:
    """The recipe, and the plan it dispatches to.

    Both, because these contracts predate the extraction and each one is about
    the behaviour rather than about where it is written. A recipe is a
    dispatch now, so reading only its body would answer a question nobody was
    asking; reading only the plan would miss the just-level wiring some of
    these are genuinely about.
    """
    block = _recipe_body(name)
    if name in DISPATCHED:
        command, args = DISPATCHED[name]
        block = block + "\n" + _planned(command, **args)
    return block


def _recipe_body(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_job_blocks(workflow: str) -> dict[str, str]:
    lines = workflow.splitlines()
    starts: list[tuple[str, int]] = []
    for index, line in enumerate(lines):
        if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
            starts.append((line.strip()[:-1], index))

    blocks: dict[str, str] = {}
    for offset, (name, start) in enumerate(starts):
        end = starts[offset + 1][1] if offset + 1 < len(starts) else len(lines)
        blocks[name] = "\n".join(lines[start:end])
    return blocks


def _load_local_release_glowup() -> ModuleType:
    path = PROJECT_ROOT / "scripts" / "local-release-glowup.py"
    spec = importlib.util.spec_from_file_location("local_release_glowup", path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    scripts_path = str(PROJECT_ROOT / "scripts")
    sys.path.insert(0, scripts_path)
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(scripts_path)
        sys.modules.pop(spec.name, None)
    return module


def _run_docker_space_gate(
    tmp_path: Path,
    *,
    before_kib: int,
    after_kib: int,
    after_trim_kib: int | None = None,
    volumes: str = "",
    rail: str = "assets",
) -> subprocess.CompletedProcess[str]:
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir(parents=True)
    state = tmp_path / "pruned"
    commands = tmp_path / "docker-commands"
    docker = fake_bin / "docker"
    docker.write_text(
        """#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FAKE_DOCKER_COMMANDS"
if [ "$1" = "run" ]; then
    case "$*" in
        *debian:bookworm-slim*)
            phase=$(cat "$FAKE_DOCKER_STATE" 2>/dev/null || true)
            if [ "$phase" = "pruned" ]; then
                free="$FAKE_DOCKER_AFTER_KIB"
            else
                free="$FAKE_DOCKER_BEFORE_KIB"
            fi
            total=$((100 * 1024 * 1024))
            used=$((total - free))
            printf '%s %s %s\\n' "$total" "$used" "$free"
            ;;
        *alpine:3.20*)
            printf 'trimmed\\n' > "$FAKE_DOCKER_STATE"
            ;;
        *)
            printf 'unexpected fake docker run: %s\\n' "$*" >&2
            exit 97
            ;;
    esac
elif [ "$1" = "builder" ] && [ "$2" = "prune" ]; then
    printf 'pruned\\n' > "$FAKE_DOCKER_STATE"
elif [ "$1" = "volume" ] && [ "$2" = "inspect" ]; then
    case " $FAKE_DOCKER_VOLUMES " in
        *" $3 "*) printf '{}\\n' ;;
        *) exit 1 ;;
    esac
elif [ "$1" = "volume" ] && [ "$2" = "rm" ]; then
    :
elif [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
    exit 1
elif [ "$1" = "ps" ]; then
    :
elif [ "$1" = "system" ] && [ "$2" = "df" ]; then
    if [ "$3" = "--format" ]; then
        printf '%s\\n' \
          '{"Active":"0","Reclaimable":"1GB (50%)","Size":"2GB","TotalCount":"2","Type":"Images"}' \
          '{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Containers"}' \
          '{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Local Volumes"}' \
          '{"Active":"0","Reclaimable":"8GB","Size":"10GB","TotalCount":"10","Type":"Build Cache"}'
    else
        printf 'Local Volumes space usage:\\n\\nVOLUME NAME LINKS SIZE\\n'
        for volume in $FAKE_DOCKER_VOLUMES; do
            printf '%s 0 1GB\\n' "$volume"
        done
        printf '\\nBuild cache usage:\\n'
    fi
else
    printf 'unexpected fake docker command: %s\\n' "$*" >&2
    exit 97
fi
"""
    )
    docker.chmod(0o755)
    colima = fake_bin / "colima"
    colima.write_text(
        """#!/bin/sh
set -eu
if [ "$1" = "status" ]; then
    printf 'running\\n'
elif [ "$1" = "ssh" ]; then
    printf '/mnt/lima-colima: 1 GiB (1073741824 bytes) trimmed\\n'
else
    exit 97
fi
"""
    )
    colima.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{fake_bin}:{env['PATH']}",
            "FAKE_DOCKER_STATE": str(state),
            "FAKE_DOCKER_BEFORE_KIB": str(before_kib),
            "FAKE_DOCKER_AFTER_KIB": str(after_kib),
            "FAKE_DOCKER_VOLUMES": volumes,
            "FAKE_DOCKER_COMMANDS": str(commands),
            "CAPSEM_STORAGE_REPORT_PATH": str(tmp_path / "storage.jsonl"),
        }
    )
    return subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / "scripts" / "ensure-docker-space.sh"),
            rail,
        ],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )


def _storage_rail(rail: str) -> dict:
    """The storage policy's limits for `rail`.

    Read rather than restated. These fixtures simulate a daemon sitting above
    or below the free-space floor, so every literal here is only meaningful
    relative to that floor: hardcoding "30 GiB is plenty" silently became
    "30 GiB is not enough" the moment the floor moved to 40, and the failure
    surfaced as a release gate refusing to build assets.
    """
    import tomllib

    policy = tomllib.loads(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )
    return policy["rails"][rail]


def test_asset_gate_owns_docker_capacity_preflight(tmp_path: Path) -> None:
    recipe = _just_recipe_block("_gate-assets:")

    preflight = '"$ROOT/scripts/ensure-docker-space.sh" assets'
    assert preflight in recipe
    assert recipe.index(preflight) < recipe.index("build_arch_lane arm64")

    assets = _storage_rail("assets")
    floor_gib = assets["minimum_free_gib"]
    keep_gib = assets["buildkit_keep_gib"]
    # Comfortably clear of the floor, and clearly under it, whatever it is.
    ample_gib = floor_gib + 10
    starved_gib = max(floor_gib // 4, 1)
    ample_kib = ample_gib * 1024 * 1024
    starved_kib = starved_gib * 1024 * 1024

    enough = _run_docker_space_gate(
        tmp_path / "enough", before_kib=ample_kib, after_kib=0
    )
    assert enough.returncode == 0, enough.stderr
    assert "Docker storage control [enforce/preflight]" in enough.stdout

    reclaimed = _run_docker_space_gate(
        tmp_path / "reclaimed",
        before_kib=starved_kib,
        after_kib=ample_kib,
    )
    assert reclaimed.returncode == 0, reclaimed.stderr
    assert "buildkit-pressure-prune" in reclaimed.stdout
    assert f"{starved_gib}.0 GiB -> {ample_gib}.0 GiB" in reclaimed.stdout
    reclaimed_commands = (tmp_path / "reclaimed" / "docker-commands").read_text()
    assert f"builder prune --force --keep-storage {keep_gib}GB" in reclaimed_commands
    assert "builder prune -af" not in reclaimed_commands

    package = _storage_rail("package")
    package_reclaimed = _run_docker_space_gate(
        tmp_path / "package-reclaimed",
        before_kib=starved_kib,
        after_kib=ample_kib,
        rail="package",
    )
    assert package_reclaimed.returncode == 0, package_reclaimed.stderr
    assert f"retain {package['buildkit_keep_gib']} GiB" in package_reclaimed.stdout
    package_commands = (tmp_path / "package-reclaimed" / "docker-commands").read_text()
    assert (
        f"builder prune --force --keep-storage {package['buildkit_keep_gib']}GB"
        in package_commands
    )

    exhausted = _run_docker_space_gate(
        tmp_path / "exhausted",
        before_kib=starved_kib,
        after_kib=starved_kib,
    )
    assert exhausted.returncode != 0
    assert f"requires {floor_gib}.0 GiB free" in exhausted.stderr

    storage_script = (PROJECT_ROOT / "scripts" / "ensure-docker-space.sh").read_text()
    controller = (PROJECT_ROOT / "scripts" / "docker-storage-policy.py").read_text()
    assert "docker " not in storage_script
    assert '"retained-active"' in controller
    assert '"buildkit-pressure-prune"' in controller


def test_native_install_is_owned_by_glowup_not_a_forked_just_recipe() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    macos_glowup = (PROJECT_ROOT / "scripts" / "macos_release_glowup.py").read_text()

    assert "\ninstall:" not in justfile
    assert "build-test-macos-package.sh" in macos_glowup
    assert "macos_tart_glowup.py" in macos_glowup
    assert "prove-macos-package-boot.sh" in macos_glowup


def test_cross_compile_repacks_deb_before_exact_systemd_install_proof() -> None:
    block = _just_recipe_block("_cross-compile")

    companion_pos = block.find("--- Build companion host binaries ---")
    tauri_pos = block.find("cargo tauri build --target")
    repack_pos = block.find("scripts/repack-deb.sh")
    validate_pos = block.find("dpkg-deb --contents")
    copy_pos = block.find('cp \\"\\$DEB\\" /src/dist/')
    proof_pos = block.find("just _prove-linux-deb")

    assert companion_pos != -1
    assert tauri_pos != -1
    assert repack_pos != -1
    assert validate_pos != -1
    assert copy_pos != -1
    assert proof_pos != -1
    assert companion_pos < tauri_pos < repack_pos < validate_pos < copy_pos < proof_pos
    assert 'dpkg -i \\"\\$DEB\\"' not in block
    assert "CAPSEM_REQUIRE_LINUX_DEB_PROOF" in block
    assert "scripts/select-linux-deb-proof.sh" in block
    assert 'if [ "$PROOF_DECISION" = "prove" ]' in block
    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in block
    )
    assert 'MANIFEST_CHANNEL="${CAPSEM_INSTALL_CHANNEL:-stable}"' in block
    assert '-e "CAPSEM_INSTALL_MANIFEST_URL=$MANIFEST_URL"' in block
    assert 'scripts/repack-deb.sh --manifest \\"\\$CAPSEM_INSTALL_MANIFEST_URL\\"' in block
    assert "file://\\$PWD/assets/manifest.json" not in block
    assert 'CAPSEM_PROOF_MANIFEST_URL="$MANIFEST_URL"' in block
    assert 'CAPSEM_PROOF_MANIFEST_CHANNEL="$MANIFEST_CHANNEL"' in block
    assert 'CAPSEM_PROOF_DEB="$DEB"' in block
    assert "capsem-bench-rs)\\$'" in block
    assert '-e "HOST_UID=$HOST_UID"' in block
    assert '-e "HOST_GID=$HOST_GID"' in block
    assert 'trap \'chown -R \\"\\$HOST_UID:\\$HOST_GID\\"' in block
    assert "/src/frontend/node_modules /src/frontend/dist" in block
    assert "dpkg -i /cargo-target/$RUST_TARGET/release/bundle/deb/*.deb" not in block


def test_exact_linux_deb_proof_uses_systemd_and_proves_guest_shell() -> None:
    block = _just_recipe_block("_prove-linux-deb")

    assert "capsem-install-test" in block
    assert "/usr/lib/systemd/systemd" in block
    assert "--privileged --cgroupns=host" in block
    assert "--security-opt seccomp=unconfined" in block
    assert "--device /dev/kvm" in block
    assert "--device /dev/vhost-vsock" in block
    assert '-v "$ROOT:/src:ro"' in block
    assert 'dpkg -i "$CONTAINER_DEB"' in block
    assert "apt-get install -f -y" in block
    assert "dpkg-query -W" in block
    for binary in (
        "capsem",
        "capsem-admin",
        "capsem-app",
        "capsem-gateway",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-process",
        "capsem-service",
        "capsem-tray",
        "capsem-tui",
        "capsem-mock-server",
    ):
        assert binary in block
    assert 'test -x "/usr/bin/$bin"' in block
    assert '"/usr/bin/$bin" --version | grep -F "$EXPECTED_VERSION"' in block
    assert 'grep -F "Installed: true"' in block
    assert 'grep -F "Running:   true"' in block
    assert 'grep -F "Service:   ok"' in block
    assert 'grep -F "Gateway:   ok"' in block
    assert "Profiles:" in block
    assert "scripts/prove-installed-shell.py" in block
    assert "CAPSEM_QUALIFIED_DEB_SHELL_OK" in block
    assert "scripts/verify-installed-release.py" in block
    assert 'MANIFEST_URL="${CAPSEM_PROOF_MANIFEST_URL:?exact package proof requires' in block
    assert (
        'MANIFEST_CHANNEL="${CAPSEM_PROOF_MANIFEST_CHANNEL:?exact package proof requires' in block
    )
    assert 'DEB_INPUT="${CAPSEM_PROOF_DEB:?exact package proof requires' in block
    assert "{{deb}}" not in block
    assert '--manifest-url "$MANIFEST_URL"' in block
    assert '--channel "$MANIFEST_CHANNEL"' in block
    assert '--package-version "$EXPECTED_VERSION"' in block
    assert "trap cleanup EXIT" in block
    assert 'dpkg -i "$CONTAINER_DEB" 2>/dev/null || true' not in block


def test_systemd_install_image_cannot_flush_host_binfmt_registrations() -> None:
    dockerfile = (PROJECT_ROOT / "docker/Dockerfile.install-test").read_text()
    install_gate = _just_recipe_block("_gate-install:")

    assert "/etc/systemd/system/systemd-binfmt.service" in dockerfile
    assert "ln -s /dev/null" in dockerfile
    assert "HOST_ROSETTA_REGISTRATION=required" in install_gate
    assert install_gate.count("/proc/sys/fs/binfmt_misc/rosetta") >= 2
    assert "systemd install container removed Colima's Rosetta binfmt registration" in install_gate


def test_binary_release_requires_exact_linux_deb_proof() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    native = _workflow_job_blocks(workflow)["test-native-linux-package"]

    assert "build-app-linux:" in workflow
    assert "runs-on: ${{ matrix.runner }}" in workflow
    assert 'python3 scripts/install-deb-runtime-dependencies.py "$package"' in native
    assert 'sudo dpkg -i "$package"' in native
    assert "sudo apt-get install -f -y" not in native
    assert native.index("install-deb-runtime-dependencies.py") < native.index(
        "install-manifest-request.sh write"
    )
    assert "scripts/verify-installed-release.py" in native
    assert "scripts/prove-installed-shell.py" in native
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow


def test_linux_deb_proof_selector_requires_only_the_native_package() -> None:
    selector = PROJECT_ROOT / "scripts" / "select-linux-deb-proof.sh"

    cases = (
        ("Linux", "x86_64", "x86_64", "1", "1", "prove"),
        ("Linux", "x86_64", "arm64", "0", "1", "skip"),
        ("Linux", "arm64", "arm64", "1", "1", "prove"),
        ("Linux", "arm64", "x86_64", "0", "1", "skip"),
        ("Darwin", "arm64", "arm64", "0", "1", "skip"),
    )
    for host_os, host_arch, target_arch, kvm_ready, required, expected in cases:
        result = subprocess.run(
            [
                "bash",
                str(selector),
                host_os,
                host_arch,
                target_arch,
                kvm_ready,
                required,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == expected


def test_linux_deb_proof_selector_fails_closed_for_native_package_without_kvm() -> None:
    selector = PROJECT_ROOT / "scripts" / "select-linux-deb-proof.sh"

    result = subprocess.run(
        ["bash", str(selector), "Linux", "arm64", "arm64", "0", "1"],
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "native Linux package proof requires KVM and vhost-vsock" in result.stderr


def test_release_matrix_installs_both_architectures_and_keeps_kvm_proof_mandatory() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    linux = _workflow_job_blocks(workflow)["test-native-linux-package"]

    assert "runner: ubuntu-24.04-arm" in linux
    assert "runner: ubuntu-24.04" in linux
    assert linux.count("if: matrix.arch == 'x86_64'") == 2
    assert "Enable KVM for exact-package VM proof" in linux
    assert "Prove exact-package guest shell execution" in linux
    assert "CAPSEM_EXACT_PACKAGE_SHELL_OK" in linux


def test_install_test_restores_host_workspace_ownership() -> None:
    block = _just_recipe_block("_gate-install")

    assert "HOST_UID=$(id -u)" in block
    assert "HOST_GID=$(id -g)" in block
    assert "chown -R $HOST_UID:$HOST_GID /src 2>" not in block
    assert "INSTALL_OWNED_PATHS=(" in block
    assert 'chown -R "$HOST_UID:$HOST_GID" "${INSTALL_OWNED_PATHS[@]}"' in block
    assert "trap cleanup EXIT" in block
    assert 'docker rm -f "$CONTAINER"' in block
    cleanup = block.split("cleanup() {", maxsplit=1)[1].split(
        "\n    }", maxsplit=1
    )[0]
    assert 'docker-storage-policy.py" release' in cleanup
    assert "--boundary after-install" in cleanup


def test_install_test_cleanup_preserves_the_original_gate_failure() -> None:
    block = _just_recipe_block("_gate-install")
    cleanup = block.split("cleanup() {", maxsplit=1)[1].split(
        "\n    }", maxsplit=1
    )[0]

    capture = cleanup.index("install_gate_exit=$?")
    disable_trap = cleanup.index("trap - EXIT")
    remove_container = cleanup.index('docker rm -f "$CONTAINER"')
    restore_status = cleanup.index('exit "$install_gate_exit"')

    assert capture < disable_trap < remove_container < restore_status


def test_install_test_does_not_rebuild_frontend_and_owns_release_site_scratch() -> None:
    block = _just_recipe_block("_gate-install")

    assert "-v capsem-install-frontend-node-modules:/src/frontend/node_modules" not in block
    assert "-v capsem-install-frontend-dist:/src/frontend/dist" not in block
    assert "pnpm build" not in block
    assert (
        "-v capsem-install-release-site-node-modules:/src/release-site/node_modules"
        in block
    )
    assert "-v capsem-install-release-site-dist:/src/release-site/dist" in block
    assert '"/src/release-site/node_modules"' in block
    assert '"/src/release-site/dist"' in block
    install_release_site = block.index(
        "cd /src/release-site && pnpm install --frozen-lockfile"
    )
    build_release_site = block.index("scripts/check-web-surface.sh release-site-build")
    assert install_release_site < build_release_site
    release_site_exec = block.rfind("docker exec", install_release_site, build_release_site)
    assert 'docker exec "$CONTAINER"' in block[release_site_exec:build_release_site]
    assert "docker exec -u capsem" not in block[release_site_exec:build_release_site]


def test_install_test_removes_stale_container_before_controller_preflight() -> None:
    block = _just_recipe_block("_gate-install").replace(r"\"", '"')

    remove_stale = block.index('docker rm -f "$CONTAINER"')
    release_working = block.index("just _release-completed-package-rails")
    capacity = block.index('scripts/ensure-docker-space.sh" install', release_working)
    remove_partial_channel = block.index('rm -rf "$INSTALL_CHANNEL_DIR"')
    start_container = block.index('echo "Starting systemd container..."')

    assert remove_stale < release_working < capacity
    assert remove_partial_channel < start_container
    assert 'INSTALL_CHANNEL_DIR="target/install-test-channel"' in block
    assert "docker system df -v" not in block
    assert "docker volume rm" not in block


def test_install_test_runs_local_release_glowup_from_real_package() -> None:
    block = _just_recipe_block("_gate-install").replace(r"\"", '"').replace(r"\$", "$")

    assert "Running Linux native release glow-up" in block
    assert "scripts/local-release-glowup.py" in block
    assert '--input-deb "$CONTAINER_DEB"' in block
    assert "--bin-dir /usr/bin" in block
    assert "--package-ready" in block
    assert '--assets-dir "$INSTALL_ASSETS_DIR"' in block
    assert '--config-root "$INSTALL_CONFIG_DIR"' in block
    assert "just _gate-install" in _just_recipe_block("test:")


def test_install_test_stages_real_profile_assets_for_mandatory_vm_proofs() -> None:
    block = _just_recipe_block("_gate-install").replace(r"\"", '"').replace(r"\$", "$")
    update_tests = (PROJECT_ROOT / "tests/capsem-install/test_update.py").read_text()
    layout_tests = (PROJECT_ROOT / "tests/capsem-install/test_installed_layout.py").read_text()

    assert 'INSTALL_ASSETS_DIR="target/install-test-assets"' in block
    assert 'INSTALL_CONFIG_DIR="target/install-test-config"' in block
    assert 'rm -rf "$INSTALL_ASSETS_DIR" "$INSTALL_CONFIG_DIR"' in block
    assert "scripts/prepare-install-test-assets.sh" not in block
    assert 'INSTALL_PROFILE_INPUTS="${CAPSEM_INSTALL_PROFILE_INPUTS:-}"' in block
    assert "scripts/stage-release-test-inputs.py" in block
    assert 'cp -R assets/. "$INSTALL_ASSETS_DIR/"' in block
    assert "requires rebuilt local assets or verified pulled profile inputs" in block
    assert "bash scripts/materialize-config.sh" not in block
    assert 'cp -R target/config/. "$INSTALL_CONFIG_DIR/"' in block
    assert 'INSTALL_SOURCE_MANIFEST="$INSTALL_CHANNEL_DIR/assets/local/manifest.json"' in block
    assert "scripts/serve-release-test-root.py" in block
    assert "capsem-admin assets channel build" in block
    assert "capsem-admin assets channel check" in block
    build_graph = block.index("capsem-admin assets channel build")
    build_site = block.index("scripts/check-web-surface.sh release-site-build")
    check_graph = block.index("capsem-admin assets channel check")
    assert build_graph < build_site < check_graph
    assert "CAPSEM_RELEASE_CHANNEL_DIST=" in block
    assert "/src/$INSTALL_CHANNEL_DIR" in block
    assert (
        "CAPSEM_TEST_ASSET_MANIFEST=/home/capsem/.capsem/assets/manifest.json"
        in block
    )
    assert '--assets-dir "$INSTALL_ASSETS_DIR"' in block
    assert '--config-root "$INSTALL_CONFIG_DIR"' in block
    assert 'TEST_ASSET_MANIFEST = os.environ.get("CAPSEM_TEST_ASSET_MANIFEST")' in update_tests
    assert "def _default_release_graph() -> dict:" in update_tests
    assert "install tests require an authoritative release graph" in update_tests
    assert 'REPO_ROOT / "assets" / "manifest.json"' not in update_tests
    assert "native package installed a legacy/runtime projection" in layout_tests
    assert "installed.read_bytes() == SOURCE_MANIFEST.read_bytes()" in layout_tests
    assert "assets downloaded on first use, not bundled in .deb" not in layout_tests


def test_install_test_consumes_exact_publishable_package_without_rebuild() -> None:
    block = _just_recipe_block("_gate-install").replace(r"\"", '"').replace(r"\$", "$")

    select = block.index('DEB="$ROOT/dist/Capsem_${SOURCE_VERSION}_${DEB_ARCH}.deb"')
    install = block.index('dpkg -i "$CONTAINER_DEB"', select)
    assert select < install
    assert 'if [ ! -s "$DEB" ]; then' in block
    assert "missing exact release-mode Debian package" in block
    assert 'VERSION="$SOURCE_VERSION"' in block
    assert (
        'PACKAGE_VERSION=$(docker exec "$CONTAINER" dpkg-deb -f '
        '"$CONTAINER_DEB" Version)'
    ) in block
    host_selection = block[: block.index("DOCKER_RUNTIME_ARGS")]
    assert "dpkg-deb" not in host_selection
    assert 'CONTAINER_DEB="/src/${DEB#$ROOT/}"' in block
    for forbidden in (
        "cargo build",
        "cargo tauri build",
        "scripts/repack-deb.sh",
        "pnpm build",
        "/cargo-target/debug/bundle/deb",
    ):
        assert forbidden not in block


def test_local_release_glowup_uses_real_release_pipeline_not_fake_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "scripts/repack-deb.sh" in script
    assert "scripts/generate-host-binary-sbom.py" in script
    assert "record-binary" in script
    assert "assets" in script and "channel" in script and "build" in script
    assert "json.dumps({" not in script or "capsem.local_release_glowup.v1" in script
    assert "stable-assets-manifest.json" in script
    assert "nightly-assets-manifest.json" in script
    assert "clone_manifest_for_channel(" in script
    assert 'args.assets_dir / "manifest.json",' in script
    assert 'stable_manifest,\n            "stable",' in script
    assert 'clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")' in script
    assert "CAPSEM_RELEASE_URL" in script
    assert "CAPSEM_RELEASE_CHANNELS_URL=" in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert script.count("update --assets --channel nightly") == 1
    assert "corp-escape.log" in script
    assert "update --assets --channel stable" not in script
    transition_gate = (PROJECT_ROOT / "scripts" / "check-public-binary-release.py").read_text()
    assert "run_docker_binary_transition_smoke" in transition_gate
    assert "update --yes --channel nightly" in transition_gate
    assert "update --yes --channel stable" in transition_gate
    assert "SimpleHTTPRequestHandler" in script
    assert "--network=host" not in script


def test_local_release_glowup_has_zstd_extraction_support_in_install_image() -> None:
    dockerfile = (PROJECT_ROOT / "docker" / "Dockerfile.install-test").read_text()

    assert "zstd" in dockerfile


def test_install_recipe_invokes_pytest_as_a_module_inside_container() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    recipe = justfile.split("_gate-install:", maxsplit=1)[1].split(
        "\n# Dispatch one serialized release workflow", maxsplit=1
    )[0]

    # /src is bind-mounted and may contain a host .venv whose console-script
    # shebang cannot exist in the Linux container. Launch via Python so uv's
    # selected interpreter owns module resolution instead.
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in recipe
    assert "uv run python -m pytest tests/capsem-install/" in recipe
    assert "uv run pytest tests/capsem-install/" not in recipe


def test_install_recipe_runs_release_glowup_in_clean_project_environment() -> None:
    recipe = _just_recipe_block("_gate-install")

    assert (
        "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test "
        "uv run python scripts/local-release-glowup.py"
    ) in recipe
    assert "python3 scripts/local-release-glowup.py" not in recipe


def test_native_packages_make_full_doctor_mock_server_self_contained() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    pkg_postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    cli = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()
    mock_server = (PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "main.rs").read_text()

    for package_script in (build_pkg, pkg_postinstall, repack_deb, deb_postinst):
        assert "capsem-mock-server" in package_script
    assert "std::env::current_exe()" in cli
    assert cli.index("std::env::current_exe()") < cli.index("std::env::current_dir()")
    assert "#[command(version" in mock_server


def test_native_packages_include_the_release_functional_benchmark() -> None:
    package_paths = (
        "scripts/repack-deb.sh",
        "scripts/deb-postinst.sh",
        "scripts/build-pkg.sh",
        "scripts/pkg-scripts/postinstall",
        "scripts/build-test-macos-package.sh",
        "scripts/macos_tart_guest.sh",
        "scripts/local-release-glowup.py",
        "scripts/simulate-install.sh",
    )
    for path in package_paths:
        assert "capsem-bench-rs" in (PROJECT_ROOT / path).read_text(), path

    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    assert "-p capsem-bench" in workflow
    assert "capsem-mock-server capsem-bench-rs" in workflow

    justfile = (PROJECT_ROOT / "justfile").read_text()
    assert "-p capsem-mock-server -p capsem-bench" in justfile
    assert "capsem-bench-rs; do" in justfile

    benchmark = (
        PROJECT_ROOT / "crates" / "capsem-bench" / "src" / "main.rs"
    ).read_text()
    assert '#[command(version = env!("CARGO_PKG_VERSION")' in benchmark


def test_binary_packages_embed_public_url_but_install_against_serialized_source() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    macos = workflow.split("  build-app-macos:\n", maxsplit=1)[1].split(
        "\n  build-app-linux:\n", maxsplit=1
    )[0]
    linux = workflow.split("  build-app-linux:\n", maxsplit=1)[1].split(
        "\n  author-binary-candidate:\n", maxsplit=1
    )[0]
    native_macos = workflow.split(
        "  test-native-macos-package:\n", maxsplit=1
    )[1].split("\n  test-native-linux-package:\n", maxsplit=1)[0]
    native_linux = workflow.split(
        "  test-native-linux-package:\n", maxsplit=1
    )[1].split("\n  test-binary-pairing:\n", maxsplit=1)[0]

    for job in (macos, linux):
        assert "needs: [preflight, resolve-channel-source]" in job
        assert "name: binary-channel-source" in job
        assert "PREACTIVATION_MANIFEST=file://" in job
        assert 'CAPSEM_ASSET_MANIFEST="$PREACTIVATION_MANIFEST"' in job

    assert macos.count('--manifest "$ASSET_MANIFEST_URL"') == 1
    assert linux.count('--manifest "$ASSET_MANIFEST_URL"') == 1
    for job in (native_macos, native_linux):
        assert "binary-channel-candidate" in job
        assert "PREACTIVATION_MANIFEST=file://" in job
        assert "scripts/install-manifest-request.sh write" in job
        assert '--manifest-url "$PREACTIVATION_MANIFEST"' in job
        assert "scripts/install-manifest-request.sh clear" in job
    assert (
        "needs: [test-native-macos-package, test-native-linux-package, "
        "test-binary-pairing]"
    ) in workflow


def test_full_gate_runs_fast_checks_before_install_harness_preflight() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    full_gate = _just_recipe_block("test:")
    preflight = justfile.split("_test-install-harness-preflight:", maxsplit=1)[1].split(
        "\ntest-install:", maxsplit=1
    )[0]

    assert "just _test-install-harness-preflight" in full_gate
    clippy = full_gate.index("cargo clippy --workspace --all-targets")
    frontend = full_gate.index("bash scripts/check-web-surface.sh frontend")
    preflight_call = full_gate.index("just _test-install-harness-preflight")
    assert clippy < preflight_call
    assert frontend < preflight_call
    assert "docker/Dockerfile.install-test" in preflight
    assert "source /src/scripts/doctor-linux.sh" in preflight
    assert "linux_musl_toolchain_available" in preflight
    assert preflight.index("linux_musl_toolchain_available") < preflight.index(
        "uv run python -m pytest --version"
    )
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in preflight
    assert "CAPSEM_TEST_OUTPUT_ROOT=/tmp/capsem-test-output" in preflight
    assert "uv run python -m pytest --version" in preflight
    assert (
        "uv run python -m pytest -p no:cacheprovider -q tests/test_materialize_config_http.py"
    ) in preflight
    assert "sudo -n true" in preflight
    assert "docker build --no-cache" in preflight


def test_local_linux_preflight_contains_asset_ci_release_tools() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    preflight = justfile.split("_test-install-harness-preflight:", maxsplit=1)[1].split(
        "\ntest-install:", maxsplit=1
    )[0]
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    assert "@cyclonedx/cdxgen@12.7.0" in host_builder
    assert "@cyclonedx/cdxgen@latest" not in host_builder
    assert "just _build-host-image" in preflight
    assert "if ! docker image inspect capsem-host-builder" not in preflight
    assert "cdxgen --version" in preflight
    assert preflight.index("cdxgen --version") < preflight.index(
        "uv run python -m pytest --version"
    )
    verify = "check_install_image"
    release_base = "--boundary after-linux-rust-builder"
    assert release_base in preflight
    assert preflight.rindex(verify) < preflight.index(release_base)


def test_cross_arch_tauri_swap_covers_every_native_dev_package() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()
    native_block = host_builder.split("# ---- Native-arch Tauri dev libraries ----", maxsplit=1)[
        1
    ].split("# ---- Helper script", maxsplit=1)[0]
    swap_block = swap_script.split("DEV_PACKAGES=(", maxsplit=1)[1].split(")", maxsplit=1)[0]

    native_packages = {
        line.strip().removesuffix("\\").strip()
        for line in native_block.splitlines()
        if line.strip().startswith("lib")
    }
    swapped_packages = {
        line.strip()
        for line in swap_block.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }

    assert swapped_packages == native_packages


def test_cross_arch_tauri_swap_excludes_non_crossable_introspection_toolchain() -> None:
    """Cross builds must not pull foreign executables that require emulation."""
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    # librsvg2-dev depends on gobject-introspection for the target architecture
    # on Ubuntu 24.04. That dependency is a target executable/Python toolchain,
    # is not required to compile Capsem, and cannot run in the native builder.
    assert "librsvg2-dev" not in swap_script
    assert "librsvg2-dev" not in host_builder
    assert "gobject-introspection" not in swap_script
    assert "qemu" not in swap_script.lower()


def test_cross_arch_frontend_build_precedes_foreign_dev_library_swap() -> None:
    """The foreign GTK graph removes native Node; build static UI first."""
    cross_compile = _just_recipe_block("_cross-compile ")

    frontend = "echo '--- Build frontend ---'"
    swap = "swap-dev-libs \\$DPKG_ARCH"
    rust = "echo '--- Build agent binaries ---'"
    assert cross_compile.index(frontend) < cross_compile.index(swap)
    assert cross_compile.index(swap) < cross_compile.index(rust)


def test_cross_compile_reasserts_pinned_rust_target_before_expensive_work() -> None:
    """A persistent rustup volume must not mask the builder's pinned targets."""
    cross_compile = _just_recipe_block("_cross-compile ")

    install = "rustup toolchain install 1.97.1 --profile minimal"
    target = "rustup target add --toolchain 1.97.1 \\$RUST_TARGET"
    verify = "rustup target list --toolchain 1.97.1 --installed"
    frontend = "echo '--- Build frontend ---'"
    swap = "swap-dev-libs \\$DPKG_ARCH"

    assert install in cross_compile
    assert target in cross_compile
    assert verify in cross_compile
    assert cross_compile.index(install) < cross_compile.index(target)
    assert cross_compile.index(target) < cross_compile.index(verify)
    assert cross_compile.index(verify) < cross_compile.index(frontend)
    assert cross_compile.index(verify) < cross_compile.index(swap)


def test_deb_repacker_strips_each_elf_with_its_target_tool_and_fails_closed() -> None:
    repack = (PROJECT_ROOT / "scripts/repack-deb.sh").read_text()

    assert "x86_64-linux-gnu-strip" in repack
    assert "aarch64-linux-gnu-strip" in repack
    assert "CAPSEM_REPACK_STRIP" not in repack
    assert "could not be stripped" not in repack


def test_cross_compile_refreshes_the_cached_host_builder_image() -> None:
    cross_compile = _just_recipe_block("_cross-compile ")
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    assert "just _build-host-image" in cross_compile
    assert "docker image inspect capsem-host-builder:latest" not in cross_compile
    assert host_builder.index("COPY swap-dev-libs.sh") > host_builder.index(
        "cargo install tauri-cli"
    )


def test_cross_compile_preflights_docker_capacity_after_builder_before_package() -> None:
    """Asset lanes must not leave Linux package builds at zero Docker disk."""
    cross_compile = _just_recipe_block("_cross-compile ")

    build_image = cross_compile.index("just _build-host-image")
    release_completed_rails = cross_compile.index("just _release-completed-docker-rails")
    release_install_target = cross_compile.index("just _release-deferred-install-target")
    capacity = '"$ROOT/scripts/ensure-docker-space.sh" package'
    capacities = [
        index for index in range(len(cross_compile)) if cross_compile.startswith(capacity, index)
    ]
    package = cross_compile.index("docker run --rm")

    # The image build itself needs headroom, then its newly materialized layers
    # must not leave the package container without room for apt and Tauri.
    assert len(capacities) == 2
    assert (
        release_completed_rails
        < release_install_target
        < capacities[0]
        < build_image
        < capacities[1]
        < package
    )
    assert cross_compile.count(capacity) == 2
    assert "docker image rm rust:slim-bookworm" not in cross_compile
    post_builder = cross_compile[build_image:package]
    assert capacity in post_builder
    assert 'scripts/ensure-docker-space.sh" 16' not in cross_compile


def test_package_boundary_releases_only_completed_docker_rail_volumes() -> None:
    release = _just_recipe_block("_release-completed-docker-rails:")
    policy = tomllib.loads((PROJECT_ROOT / "config/storage-policy.toml").read_text())

    assert "--boundary after-assets" in release
    resources = policy["resources"]
    assert resources["capsem-agent-target-arm64"]["release_boundary"] == "after-assets"
    assert resources["capsem-agent-target-x86_64"]["release_boundary"] == "after-assets"
    assert resources["capsem-rustup-arm64"]["retention"] == "cache"
    assert resources["capsem-rustup-x86_64"]["retention"] == "cache"
    assert "docker volume rm" not in release


def test_linux_rust_target_is_released_before_asset_capacity_preflight() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    release = _just_recipe_block("_release-completed-linux-rust-target:")

    linux_rust = candidate.index("just _gate-linux-rust")
    release_call = candidate.index("just _release-completed-linux-rust-target")
    release_builder = candidate.index("--boundary after-linux-rust-builder")
    asset_gate = candidate.index("just _gate-assets")

    assert linux_rust < release_call < release_builder < asset_gate
    assert "--boundary after-linux-rust" in release
    assert "docker volume rm" not in release


def test_install_boundary_releases_only_completed_package_targets() -> None:
    release = _just_recipe_block("_release-completed-package-rails:")
    install = _just_recipe_block("_gate-install:")

    assert "--boundary after-package-arm64" in release
    assert "--boundary after-package-x86_64" in release
    assert "docker volume rm" not in release

    cleanup_trap = install.index("trap cleanup EXIT")
    release_call = install.index("just _release-completed-package-rails")
    capacity = install.index('scripts/ensure-docker-space.sh" install', release_call)
    assert cleanup_trap < release_call < capacity


def test_full_gate_releases_deferred_install_target_between_package_arches() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    release = _just_recipe_block("_release-deferred-install-target:")

    arm_package = candidate.index("just _cross-compile arm64")
    release_call = candidate.index("just _release-deferred-install-target")
    x86_package = candidate.index("just _cross-compile x86_64")

    assert arm_package < release_call < x86_package
    assert "--boundary before-packages" in release
    assert "docker volume rm" not in release


def test_full_gate_releases_completed_buildkit_graph_after_packages() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    release = _just_recipe_block("_release-completed-buildkit-graph:")

    arm_package = candidate.index("just _cross-compile arm64")
    x86_package = candidate.index("just _cross-compile x86_64")
    release_call = candidate.index("just _release-completed-buildkit-graph")

    assert arm_package < x86_package < release_call
    assert "--boundary after-packages" in release
    assert "docker buildx prune" not in release
    assert "docker volume rm" not in release


def test_full_gate_bounds_docker_storage_without_flushing_rebuild_caches() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    bound = _just_recipe_block("_bound-docker-test-storage:")

    assert "just _bound-docker-test-storage" in candidate
    assert candidate.index("just _gate-install") < candidate.rindex(
        "just _bound-docker-test-storage"
    )
    capacity = bound.index("scripts/ensure-docker-space.sh")
    release_install = bound.index("--boundary candidate-boundary")
    assert release_install < capacity
    assert "--boundary after-linux-rust-builder" not in bound
    assert "docker image rm -f" not in bound
    assert "docker volume rm" not in bound


def test_full_gate_releases_stage_final_images_and_bounds_completed_cache() -> None:
    candidate = _just_recipe_block("_test-candidate:")

    install_preflight = candidate.index("just _test-install-harness-preflight")
    release_install = candidate.index("--boundary after-install-preflight")
    linux_parity = candidate.index("just _gate-linux-rust")
    asset_gate = candidate.index("just _gate-assets")
    release_buildkit = candidate.index("just _release-completed-buildkit-graph")
    arm_package = candidate.index("just _cross-compile arm64")
    x86_package = candidate.index("just _cross-compile x86_64")
    install_tail = candidate.rindex("just _gate-install")

    assert install_preflight < release_install < arm_package
    assert linux_parity < asset_gate
    assert asset_gate < arm_package < x86_package < release_buildkit < install_tail
    assert "CAPSEM_KEEP_HOST_BUILDER=1" not in candidate


def test_docker_gc_reclaims_old_created_debug_containers() -> None:
    cleanup = _just_recipe_block("_docker-gc:")
    controller = (PROJECT_ROOT / "scripts/docker-storage-policy.py").read_text()

    assert "docker-storage-policy.py gc" in cleanup
    assert '"container",\n                    "prune"' in controller
    assert 'f"until={container_age}h"' in controller
    assert "--filter status=exited" not in controller


def test_install_gate_has_no_disposable_compiler_state_before_pytest() -> None:
    block = _just_recipe_block("_gate-install")

    package_install = block.index("Installing exact release package via dpkg")
    ledger_handoff = block.index(
        "chown -R $HOST_UID:$HOST_GID /src/target/storage",
        package_install,
    )
    final_capacity = block.index('scripts/ensure-docker-space.sh" install', package_install)
    pytest_launch = block.index("Running install e2e tests")

    assert package_install < ledger_handoff < final_capacity < pytest_launch
    assert "/cargo-target" not in block


def test_cross_compile_does_not_bypass_apt_date_validation() -> None:
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    assert "Acquire::Check-Valid-Until=false" not in swap_script
    assert "Acquire::Check-Date=false" not in swap_script


def test_cross_compile_apt_sources_are_encrypted_retried_and_fail_closed() -> None:
    sources = (PROJECT_ROOT / "docker/sources-multiarch.sh").read_text()
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    mirror_assignments = [
        line.strip()
        for line in sources.splitlines()
        if line.strip().startswith(
            ("NATIVE_MIRROR=", "NATIVE_SECURITY=", "FOREIGN_MIRROR=", "FOREIGN_SECURITY=")
        )
    ]
    assert mirror_assignments
    assert all('="https://' in line for line in mirror_assignments)
    assert 'Acquire::Retries "5";' in sources
    assert 'Acquire::https::Timeout "30";' in sources
    assert 'APT::Update::Error-Mode "any";' in sources

    trust_stage = (
        "FROM alpine:3.22@sha256:"
        "14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce "
        "AS truststore"
    )
    trust_copy = (
        "COPY --from=truststore /etc/ssl/certs/ca-certificates.crt "
        "/etc/ssl/certs/ca-certificates.crt"
    )
    sources_copy = "COPY sources-multiarch.sh /tmp/"
    first_update = "RUN apt-get update && apt-get install"
    assert trust_stage in host_builder
    assert host_builder.index(trust_stage) < host_builder.index("FROM ubuntu:24.04")
    assert (
        host_builder.index("FROM ubuntu:24.04")
        < host_builder.index(trust_copy)
        < host_builder.index(sources_copy)
        < host_builder.index(first_update)
    )


def test_cross_arch_tauri_swap_refreshes_indexes_before_removing_native_libs() -> None:
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    update = swap_script.index("apt-get update -qq")
    remove = swap_script.index('apt-get remove -y "${DEV_PACKAGES[@]}"')
    install = swap_script.index("apt-get install -y --no-install-recommends")
    assert update < remove < install
    assert swap_script.count("apt-get update -qq") == 1


def test_host_builder_does_not_refetch_multiarch_indexes_for_python() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()
    native_tools = host_builder.split(
        "# ---- Native build tools + cross-compilation toolchains ----", maxsplit=1
    )[1].split("# ---- Node.js 24 + pnpm 10 ----", maxsplit=1)[0]
    python = host_builder.split("# ---- Python 3 + uv", maxsplit=1)[1].split(
        "# ---- Helper script", maxsplit=1
    )[0]

    assert "python3 \\" in native_tools
    assert "python3-venv \\" in native_tools
    assert native_tools.count("apt-get update") == 1
    assert host_builder.count("apt-get update") == 1
    assert "apt-get update" not in python
    assert "astral.sh/uv/install.sh" in python


def test_host_builder_uses_digest_pinned_prebuilt_node_runtime() -> None:
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    node_stage = (
        "FROM node:24-bookworm-slim@sha256:"
        "6f7b03f7c2c8e2e784dcf9295400527b9b1270fd37b7e9a7285cf83b6951452d "
        "AS node-runtime"
    )
    assert node_stage in host_builder
    assert "COPY --from=node-runtime /usr/local/ /usr/local/" in host_builder
    assert "deb.nodesource.com" not in host_builder


def test_standalone_install_gate_preflights_privileged_helper() -> None:
    block = _just_recipe_block("_gate-install")

    capability = block.index("installed doctor requires KVM")
    release_install_target = block.index("just _release-deferred-install-target")
    capacity = block.index('"$ROOT/scripts/ensure-docker-space.sh" install-preflight')
    preflight = block.index("just _test-install-harness-preflight")
    start_container = block.index('echo "Starting systemd container..."')

    assert capability < release_install_target < capacity < preflight < start_container


def test_install_gate_passes_vm_devices_to_full_installed_proofs() -> None:
    block = _just_recipe_block("_gate-install")

    assert 'if [ "$(uname -s)" = "Linux" ]; then' in block
    assert "DOCKER_RUNTIME_ARGS=(" in block
    assert "--security-opt seccomp=unconfined" in block
    assert ('DOCKER_RUNTIME_ARGS+=("--device" "/dev/kvm" "--device" "/dev/vhost-vsock")') in block
    assert 'DOCKER_RUNTIME_ARGS+=("--device" "/dev/vsock")' in block
    assert '"${DOCKER_RUNTIME_ARGS[@]}"' in block
    assert 'docker exec "$CONTAINER" test -r /dev/kvm -a -w /dev/kvm' in block
    assert ('docker exec "$CONTAINER" test -r /dev/vhost-vsock -a -w /dev/vhost-vsock') in block
    assert "CAPSEM_SKIP_KVM_CHECK" not in block
    assert "colima ssh -- test -r /dev/kvm" not in block


def test_macos_install_gate_consumes_native_full_probe_evidence() -> None:
    runner = _just_recipe_block("_test-candidate-run")
    install = _just_recipe_block("_gate-install")

    macos = runner.index("python3 scripts/macos_release_glowup.py")
    export = runner.index("CAPSEM_MACOS_NATIVE_GLOWUP_REPORT")
    install_call = runner.index("just _gate-install")
    assert macos < export < install_call
    assert "scripts/check-macos-native-glowup.py" in install
    assert "--skip-install" in install
    assert "Linux native release glow-up" in install
    assert "unsupported nested ARM VM boot" in install


def test_macos_install_gate_missing_native_report_fails_before_cleanup() -> None:
    install = _just_recipe_block("_gate-install").replace(r"\"", '"').replace(r"\$", "$")

    assert '${CAPSEM_MACOS_NATIVE_GLOWUP_REPORT:?' not in install
    guard = install.index('if [ -z "${CAPSEM_MACOS_NATIVE_GLOWUP_REPORT:-}" ]; then')
    diagnostic = install.index("macOS install rail requires the native glow-up report", guard)
    failure = install.index("exit 1", diagnostic)
    assignment = install.index('MACOS_REPORT="$CAPSEM_MACOS_NATIVE_GLOWUP_REPORT"', failure)

    assert guard < diagnostic < failure < assignment


def test_binary_release_sbom_jobs_install_zstd_for_deb_payloads() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    jobs = _workflow_job_blocks(workflow)
    author = jobs["author-binary-candidate"]

    assert "Install host SBOM archive deps" in author
    assert "zstd" in author
    assert author.index("Install host SBOM archive deps") < author.index(
        "Generate packaged host SBOM once"
    )
    assert "name: binary-host-sbom" in author
    assert "Generate packaged host SBOM" not in jobs["create-release"]
    assert "Generate packaged host SBOM" not in jobs["assemble-release-channel"]


def test_local_release_glowup_channel_build_uses_local_release_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    build_channel = script.split("def build_channel(", maxsplit=1)[1].split(
        "\ndef copy_artifact_tree", maxsplit=1
    )[0]

    assert "CAPSEM_RELEASE_URL" in build_channel
    assert 'f"{base_url}/releases/download/{channel}"' in build_channel
    assert "--asset-source-base" in build_channel
    assert 'f"{base_url}/assets/releases/{{asset_version}}"' in build_channel
    assert (
        "stage_manifest_artifacts("
        in script
    )


def test_local_release_glowup_uses_preserved_admin_binary_without_rebuild() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    record_binary = script.split("def record_binary(", maxsplit=1)[1].split(
        "\ndef build_channel", maxsplit=1
    )[0]
    build_channel = script.split("def build_channel(", maxsplit=1)[1].split(
        "\ndef copy_artifact_tree", maxsplit=1
    )[0]

    assert 'admin = args.bin_dir / "capsem-admin"' in script
    assert "os.access(admin, os.X_OK)" in script
    assert "str(admin)" in record_binary
    assert "str(admin)" in build_channel
    assert '"cargo"' not in record_binary
    assert '"cargo"' not in build_channel


def test_local_release_glowup_repack_uses_selected_asset_fixture(
    tmp_path: Path,
    monkeypatch,
) -> None:
    glowup = _load_local_release_glowup()
    commands: list[list[str]] = []
    monkeypatch.setattr(glowup, "run", lambda command, **_kwargs: commands.append(command))

    assets_dir = tmp_path / "isolated-assets"
    glowup.repack_deb(
        tmp_path / "input.deb",
        tmp_path / "output.deb",
        tmp_path / "bin",
        tmp_path / "config",
        assets_dir,
        "https://release.invalid/assets/stable/manifest.json",
    )

    assert commands == [
        [
            "bash",
            "scripts/repack-deb.sh",
            "--manifest",
            "https://release.invalid/assets/stable/manifest.json",
            str(tmp_path / "input.deb"),
            str(tmp_path / "bin"),
            str(tmp_path / "config"),
            str(assets_dir),
            str(tmp_path / "output.deb"),
        ]
    ]


def test_local_release_glowup_hardlinks_same_filesystem_immutable_blobs(
    tmp_path: Path,
    monkeypatch,
) -> None:
    """Late release staging must not allocate a second multi-GB asset cohort."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"immutable-rootfs-fixture")

    def reject_duplicate_copy(*_args, **_kwargs) -> None:
        raise OSError(errno.ENOSPC, "constrained release runner")

    monkeypatch.setattr(glowup.shutil, "copy2", reject_duplicate_copy)

    glowup.copy_artifact_tree(source, target)

    assert target.read_bytes() == source.read_bytes()
    assert os.path.samefile(source, target)


def test_local_release_glowup_falls_back_to_copy_across_filesystems(
    tmp_path: Path,
    monkeypatch,
) -> None:
    """Hardlink optimization must remain correct when source and dist differ."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"cross-filesystem-rootfs-fixture")

    def reject_cross_device_link(*_args, **_kwargs) -> None:
        raise OSError(errno.EXDEV, "cross-device link")

    monkeypatch.setattr(glowup.os, "link", reject_cross_device_link)

    glowup.copy_artifact_tree(source, target)

    assert target.read_bytes() == source.read_bytes()
    assert not os.path.samefile(source, target)


def test_local_release_glowup_stages_package_ready_artifact_into_fresh_tree(
    tmp_path: Path,
) -> None:
    """Reusing a release package must not rely on the repacker creating dirs."""
    glowup = _load_local_release_glowup()
    source = tmp_path / "dist" / "Capsem_1.2.3_arm64.deb"
    target = tmp_path / "work" / "artifacts" / "stable" / "v1.2.3" / source.name
    source.parent.mkdir()
    source.write_bytes(b"exact-publishable-package")

    glowup.stage_package_ready_artifact(source, target)

    assert target.read_bytes() == source.read_bytes()


def test_local_release_glowup_does_not_copy_after_real_disk_exhaustion(
    tmp_path: Path,
    monkeypatch,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "assets" / "rootfs.erofs"
    target = tmp_path / "dist" / "x86_64-rootfs.erofs"
    source.parent.mkdir()
    source.write_bytes(b"disk-exhaustion-rootfs-fixture")

    def reject_full_filesystem(*_args, **_kwargs) -> None:
        raise OSError(errno.ENOSPC, "no free inode or data block")

    copy_attempted = False

    def record_copy_attempt(*_args, **_kwargs) -> None:
        nonlocal copy_attempted
        copy_attempted = True

    monkeypatch.setattr(glowup.os, "link", reject_full_filesystem)
    monkeypatch.setattr(glowup.shutil, "copy2", record_copy_attempt)

    with pytest.raises(OSError, match="no free inode or data block"):
        glowup.copy_artifact_tree(source, target)

    assert not copy_attempted


def test_local_release_glowup_stages_graph_bytes_by_manifest_digest(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\nrevision = "2030.0101.1"\n'
    source.write_bytes(payload)
    record = {
        "kind": "profile",
        "path": "profiles/code/profile.toml",
        "url": source.resolve().as_uri(),
        "bytes": len(payload),
        "digest": {
            "sha256": hashlib.sha256(payload).hexdigest(),
            "blake3": blake3(payload).hexdigest(),
        },
        "status": "current",
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [record],
                                "images": [],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    dist = tmp_path / "dist"
    base_url = "http://127.0.0.1:43123"

    glowup.stage_manifest_artifacts(manifest_path, tmp_path / "unused", dist, base_url)

    staged = json.loads(manifest_path.read_text(encoding="utf-8"))
    staged_record = staged["profiles"]["code"]["architectures"][0]["config"][0]
    expected_relative = (
        Path("artifacts")
        / "sha256"
        / hashlib.sha256(payload).hexdigest()
        / "profile.toml"
    )
    assert staged_record["url"] == f"{base_url}/{expected_relative.as_posix()}"
    assert (dist / expected_relative).read_bytes() == payload
    assert os.path.samefile(source, dist / expected_relative)


def test_local_release_glowup_projects_only_fully_staged_architectures(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\nrevision = "2030.0101.1"\n'
    source.write_bytes(payload)

    def record(url: str) -> dict[str, object]:
        return {
            "kind": "profile",
            "path": "profiles/code/profile.toml",
            "url": url,
            "bytes": len(payload),
            "digest": {
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3(payload).hexdigest(),
            },
            "status": "current",
        }

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [
                                    record(
                                        "/profiles/releases/stable/code/"
                                        "2030.0101.1/arm64/profile.toml"
                                    )
                                ],
                                "images": [],
                                "evidence": [],
                            },
                            {
                                "architecture": "x86_64",
                                "config": [record(source.resolve().as_uri())],
                                "images": [],
                                "evidence": [],
                            },
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    dist = tmp_path / "dist"
    base_url = "http://127.0.0.1:43123"

    glowup.stage_manifest_artifacts(manifest_path, tmp_path / "unused", dist, base_url)

    staged = json.loads(manifest_path.read_text(encoding="utf-8"))
    architectures = staged["profiles"]["code"]["architectures"]
    assert [row["architecture"] for row in architectures] == ["x86_64"]
    staged_record = architectures[0]["config"][0]
    expected_relative = (
        Path("artifacts")
        / "sha256"
        / hashlib.sha256(payload).hexdigest()
        / "profile.toml"
    )
    assert staged_record["url"] == f"{base_url}/{expected_relative.as_posix()}"
    assert (dist / expected_relative).read_bytes() == payload


def test_local_release_glowup_clones_graph_with_only_channel_identity_changed(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "stable-manifest.json"
    destination = tmp_path / "nightly-manifest.json"
    source_manifest = {
        "version": "1.0.143",
        "channel": "stable",
        "packages": [{"name": "Capsem_stable_amd64.deb", "status": "current"}],
        "profiles": {
            "code": {
                "status": "current",
                "revision": "1.0.0",
                "architectures": [{"architecture": "x86_64"}],
            }
        },
    }
    source.write_text(
        json.dumps(source_manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    original = source.read_bytes()

    glowup.clone_manifest_for_channel(source, destination, "nightly")

    assert source.read_bytes() == original
    cloned = json.loads(destination.read_text(encoding="utf-8"))
    expected = dict(source_manifest)
    expected["channel"] = "nightly"
    assert cloned == expected


def test_local_release_glowup_projects_both_switch_channels_from_any_candidate() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    setup = script.split(
        'stable_manifest = manifests / "stable-assets-manifest.json"', maxsplit=1
    )[1].split("record_binary(", maxsplit=1)[0]

    assert 'args.assets_dir / "manifest.json"' in setup
    assert "stable_manifest," in setup
    assert '"stable"' in setup
    assert "clone_manifest_for_channel(stable_manifest, nightly_manifest, \"nightly\")" in setup
    assert "shutil.copy2(args.assets_dir / \"manifest.json\"" not in setup

    # Nightly must be projected from the *staged* stable manifest. Staging drops
    # architectures whose blobs are absent locally -- ordinary CI pulls one
    # architecture's profile inputs -- so cloning first left nightly describing
    # an unstaged architecture whose URLs still pointed at GitHub, which the
    # hermetic channel then rejected as "not local".
    assert setup.index("stage_manifest_artifacts(stable_manifest") < setup.index(
        'clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")'
    )


def test_local_release_glowup_rejects_partially_staged_architecture(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "profile.toml"
    source.parent.mkdir()
    payload = b'id = "code"\n'
    source.write_bytes(payload)

    def record(kind: str, url: str) -> dict[str, object]:
        return {
            "kind": kind,
            "path": f"profiles/code/{kind}.toml",
            "url": url,
            "bytes": len(payload),
            "digest": {
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3(payload).hexdigest(),
            },
            "status": "current",
        }

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "packages": [{"status": "current"}],
                "profiles": {
                    "code": {
                        "status": "current",
                        "architectures": [
                            {
                                "architecture": "x86_64",
                                "config": [
                                    record("profile", source.resolve().as_uri())
                                ],
                                "images": [
                                    record(
                                        "rootfs",
                                        "/profiles/releases/stable/code/"
                                        "2030.0101.1/x86_64/rootfs.erofs",
                                    )
                                ],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(SystemExit, match="mixes staged and unstaged artifacts"):
        glowup.stage_manifest_artifacts(
            manifest_path,
            tmp_path / "unused",
            tmp_path / "dist",
            "http://127.0.0.1:43123",
        )


def test_local_release_glowup_rejects_graph_bytes_not_matching_manifest(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    source = tmp_path / "inputs" / "rootfs.erofs"
    source.parent.mkdir()
    payload = b"corrupt-rootfs"
    source.write_bytes(payload)
    record = {
        "kind": "rootfs",
        "name": "rootfs.erofs",
        "url": source.resolve().as_uri(),
        "bytes": len(payload),
        "digest": {
            "sha256": "0" * 64,
            "blake3": blake3(payload).hexdigest(),
        },
        "status": "current",
    }
    manifest_path = tmp_path / "manifest.json"
    original = json.dumps(
        {
            "packages": [{"status": "current"}],
            "profiles": {
                "code": {
                    "status": "current",
                    "architectures": [
                        {
                            "architecture": "arm64",
                            "config": [],
                            "images": [record],
                            "evidence": [],
                        }
                    ],
                }
            },
        }
    )
    manifest_path.write_text(original, encoding="utf-8")

    with pytest.raises(SystemExit, match="SHA-256 mismatch"):
        glowup.stage_manifest_artifacts(
            manifest_path,
            tmp_path / "unused",
            tmp_path / "dist",
            "http://127.0.0.1:43123",
        )

    assert manifest_path.read_text(encoding="utf-8") == original


def test_local_release_glowup_reports_capacity_before_late_asset_staging(
    tmp_path: Path,
    monkeypatch,
    capsys,
) -> None:
    glowup = _load_local_release_glowup()
    gib = 1024**3
    monkeypatch.setattr(
        glowup.shutil,
        "disk_usage",
        lambda _path: SimpleNamespace(total=20 * gib, used=8 * gib, free=12 * gib),
    )

    glowup.report_disk_capacity(tmp_path, "before immutable VM blob staging")

    assert capsys.readouterr().out == (
        "Disk capacity (before immutable VM blob staging): 12.0 GiB free of 20.0 GiB\n"
    )


def test_release_site_overlay_replaces_partial_files_without_clobbering_artifacts(
    tmp_path: Path,
) -> None:
    site = tmp_path / "release-site"
    source = site / "dist"
    source.joinpath("profiles", "code").mkdir(parents=True)
    source.joinpath("index.html").write_text("complete-index", encoding="utf-8")
    source.joinpath("profiles", "code", "index.html").write_text(
        "complete-profile",
        encoding="utf-8",
    )
    target = tmp_path / "release-channel"
    target.joinpath("profiles", "releases").mkdir(parents=True)
    immutable = target / "profiles" / "releases" / "profile.toml"
    immutable.write_text("immutable-profile-artifact", encoding="utf-8")
    stale = target / "index.html"
    stale.write_text("", encoding="utf-8")
    stale.chmod(0o200)

    result = subprocess.run(
        [
            "node",
            str(PROJECT_ROOT / "release-site" / "scripts" / "overlay-dist.mjs"),
        ],
        cwd=site,
        env={**os.environ, "CAPSEM_RELEASE_CHANNEL_DIST": str(target)},
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert stale.read_text(encoding="utf-8") == "complete-index"
    assert target.joinpath("profiles", "code", "index.html").read_text(
        encoding="utf-8"
    ) == "complete-profile"
    assert immutable.read_text(encoding="utf-8") == "immutable-profile-artifact"


def test_release_skills_require_space_efficient_immutable_staging() -> None:
    for skill_path in (
        PROJECT_ROOT / "skills" / "dev-testing" / "SKILL.md",
        PROJECT_ROOT / "skills" / "release-process" / "SKILL.md",
    ):
        skill = _skill_text(skill_path)
        assert "hardlink-first" in skill
        assert "same-filesystem" in skill
        assert "cross-filesystem" in skill
        assert "constrained-disk" in skill


def test_local_release_glowup_uses_the_safe_manifest_url_resolver() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    checker = script.split("def check_generated_release(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]
    resolver = script.split("def local_release_artifact_path(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]

    assert "local_release_artifact_path(base_url, url, dist)" in checker
    assert "safe_relative(" in resolver
    assert "unquote(parsed.path)" in resolver


def test_local_release_glowup_validates_vm_asset_blobs_are_served() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "release_asset_urls" in script
    assert "release is missing VM asset blob" in script
    assert '"artifacts") / "sha256"' in script
    assert "verify_payload(" in script


def test_local_release_glowup_preflights_stable_and_nightly_manifests() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "stable_artifact = check_generated_release(" in script
    assert "nightly_artifact = check_generated_release(" in script
    assert "expected_version=stable_version" in script
    assert "expected_version=nightly_version" in script
    assert "assert_manifest_artifact(manifest, artifact)" in script


def test_local_release_glowup_generated_release_checker_rejects_missing_asset_blob(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        manifest_path = dist / "assets" / "stable" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            f"""{{
  "packages": [
    {{
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "{base_url}/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }}
  ],
  "profiles": {{
    "co-work": {{
      "architectures": [
        {{
          "images": [
            {{"url": "{base_url}/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}}
          ],
          "evidence": [
            {{"url": "{base_url}/assets/releases/2026.0709.13/obom.cdx.json"}}
          ]
        }}
      ]
    }}
  }}
}}
""",
            encoding="utf-8",
        )

        try:
            glowup.check_generated_release(
                base_url,
                f"{base_url}/assets/stable/manifest.json",
                deb,
                dist,
                "stable",
            )
        except SystemExit as error:
            assert "generated stable release is missing VM asset blob" in str(error)
            assert "x86_64-rootfs.erofs" in str(error)
        else:
            raise AssertionError("missing VM asset blob was accepted")


def test_local_release_glowup_generated_release_checker_rejects_tampered_blob(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        expected = b"verified-rootfs"
        artifact_path = dist / "artifacts/sha256" / hashlib.sha256(expected).hexdigest() / "rootfs.erofs"
        artifact_path.parent.mkdir(parents=True)
        artifact_path.write_bytes(b"tampered-rootfs")
        manifest_path = dist / "assets" / "stable" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            json.dumps(
                {
                    "packages": [
                        {
                            "name": "Capsem_1.5.1_amd64.deb",
                            "url": (
                                f"{base_url}/releases/download/v1.5.1/"
                                "Capsem_1.5.1_amd64.deb"
                            ),
                        }
                    ],
                    "profiles": {
                        "code": {
                            "architectures": [
                                {
                                    "images": [
                                        {
                                            "kind": "rootfs",
                                            "name": "rootfs.erofs",
                                            "url": f"{base_url}/{artifact_path.relative_to(dist)}",
                                            "bytes": len(expected),
                                            "digest": {
                                                "sha256": hashlib.sha256(expected).hexdigest(),
                                                "blake3": blake3(expected).hexdigest(),
                                            },
                                        }
                                    ],
                                    "config": [],
                                    "evidence": [],
                                }
                            ]
                        }
                    },
                }
            ),
            encoding="utf-8",
        )

        with pytest.raises(SystemExit, match=r"SHA-256 mismatch|byte size mismatch"):
            glowup.check_generated_release(
                base_url,
                f"{base_url}/assets/stable/manifest.json",
                deb,
                dist,
                "stable",
            )


def test_local_release_glowup_generated_release_checker_accepts_manifest_root_relative_assets(
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    dist = tmp_path / "dist"
    dist.mkdir()
    deb = tmp_path / "Capsem_1.5.1_amd64.deb"

    with glowup.local_release_server(dist) as base_url:
        package_path = dist / "releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
        package_path.parent.mkdir(parents=True)
        package_path.write_bytes(b"fixture deb")
        payload = b"fixture"
        for relative in (
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/profile.toml",
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/rootfs.erofs",
            "profiles/releases/nightly/co-work/2026.0709.13/x86_64/obom.cdx.json",
        ):
            target = dist / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(payload)
        manifest_path = dist / "assets" / "nightly" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            json.dumps(
                {
                    "packages": [
                        {
                            "name": "Capsem_1.5.1_amd64.deb",
                            "url": (
                                f"{base_url}/releases/download/v1.5.1/"
                                "Capsem_1.5.1_amd64.deb"
                            ),
                        }
                    ],
                    "profiles": {
                        "co-work": {
                            "architectures": [
                                {
                                    "images": [
                                        {
                                            "kind": "rootfs",
                                            "name": "rootfs.erofs",
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/rootfs.erofs"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                    "config": [
                                        {
                                            "kind": "profile",
                                            "path": "profiles/co-work/profile.toml",
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/profile.toml"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                    "evidence": [
                                        {
                                            "url": (
                                                "/profiles/releases/nightly/co-work/"
                                                "2026.0709.13/x86_64/obom.cdx.json"
                                            ),
                                            "bytes": len(payload),
                                            "digest": {
                                                "sha256": hashlib.sha256(payload).hexdigest(),
                                                "blake3": blake3(payload).hexdigest(),
                                            },
                                        }
                                    ],
                                }
                            ]
                        }
                    },
                }
            ),
            encoding="utf-8",
        )

        glowup.check_generated_release(
            base_url,
            f"{base_url}/assets/nightly/manifest.json",
            deb,
            dist,
            "nightly",
        )


@pytest.mark.parametrize(
    "url",
    [
        "/profiles/releases/%2e%2e/escape",
        "//attacker.invalid/rootfs.erofs",
        "https://attacker.invalid/rootfs.erofs",
        "/profiles/releases/rootfs.erofs?replacement=1",
    ],
)
def test_local_release_glowup_rejects_unsafe_or_nonlocal_manifest_urls(
    tmp_path: Path,
    url: str,
) -> None:
    glowup = _load_local_release_glowup()

    with pytest.raises(SystemExit):
        glowup.local_release_artifact_path(
            "http://127.0.0.1:43123",
            url,
            tmp_path,
        )


def test_local_release_glowup_installed_path_asserts_channel_round_trip_and_provenance(
    monkeypatch,
    tmp_path: Path,
) -> None:
    glowup = _load_local_release_glowup()
    calls: list[list[str]] = []

    monkeypatch.setattr(glowup, "run", lambda cmd, **_kwargs: calls.append(cmd))

    glowup.run_installed_glowup(
        install_script_url="http://127.0.0.1:1234/install.sh",
        release_base_url="http://127.0.0.1:1234",
        stable_manifest_url="http://127.0.0.1:1234/assets/stable/manifest.json",
        nightly_manifest_url="http://127.0.0.1:1234/assets/nightly/manifest.json",
        corp_manifest_url="http://127.0.0.1:1234/corp/manifest.json",
        package_version="1.5.100",
        stable_package=tmp_path / "Capsem_1.5.100_amd64.deb",
        nightly_package=tmp_path / "Capsem_1.5.100_amd64.deb",
        package_architecture="amd64",
    )

    assert len(calls) == 1
    script = calls[0][-1]
    subprocess.run(["bash", "-n"], input=script, text=True, check=True)
    assert 'grep -F \'"package_version": "1.5.100"\'' in script
    assert 'stable_manifest_sha=$(sha256sum "$HOME/.capsem/assets/manifest.json"' in script
    assert 'test "$stable_manifest_sha" = "$stable_manifest_sha_after_switch"' in script
    assert (
        "check_update_log asset_update_complete http://127.0.0.1:1234/assets/nightly/manifest.json"
        in script
    )
    assert 'CAPSEM_RELEASE_CHANNELS_URL="$release_channels_url"' in script
    assert "binary_update_failed" not in script
    assert "binary_update_complete" not in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert script.count("update --assets --channel") == 1
    assert 'update --assets --channel nightly > "$HOME/.capsem/corp-escape.log"' in script
    assert '"package_version": "1.5.101"' not in script
    assert "probe_installed_transition fresh-stable" in script
    assert "probe_installed_transition channel-nightly" in script
    assert "probe_installed_transition channel-stable-return" in script
    assert "probe_installed_transition corporate" in script
    assert "probe_installed_transition final-nightly" in script
    assert '"$CAPSEM_BIN" status' in script
    assert 'grep -Fq "Installed: true"' in script
    assert 'grep -Fq "Running:   true"' in script
    assert 'grep -Fq "Service:   ok"' in script
    assert 'grep -Fq "Gateway:   ok"' in script
    assert "scripts/verify-installed-release.py" in script
    assert '"$CAPSEM_BIN" doctor' in script
    assert "scripts/run-installed-winterfell.py" in script
    assert "service status" not in script
    assert "CAPSEM_CHANNEL=nightly" in script
    assert "http://127.0.0.1:1234/corp/manifest.json" in script
    assert (
        "check_update_log asset_update_complete http://127.0.0.1:1234/corp/manifest.json" in script
    )
    assert "corporate channel is locked" in script
    assert "corp_escape_status" in script


def test_local_release_glowup_asserts_channel_isolation_and_corp_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "stable_channel_sha_before_nightly" in script
    assert "nightly channel build mutated stable manifest" in script
    assert "nightly channel build mutated stable package records" in script
    assert 'corp_manifest_url = f"{base_url}/corp/manifest.json"' in script
    assert 'corp_dir = dist / "corp"' in script
    assert "update --yes --channel nightly" in script
    assert "update --yes --channel stable" in script
    assert "check_origin_channel corp" in script


def test_local_release_glowup_forbids_metadata_only_binary_cohorts() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "rewrite_deb_version" not in script
    assert "next_patch_version" not in script
    assert "without recompiling a second binary cohort" not in script


def test_native_glowup_owns_exact_manifest_and_installed_shell_evidence() -> None:
    macos = (PROJECT_ROOT / "scripts" / "macos_release_glowup.py").read_text()
    linux = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "assert_manifest_artifact" in macos
    assert "assert_manifest_artifact" in linux
    assert "prove-macos-package-boot.sh" in macos
    assert "verify-installed-release.py" in linux


def test_dev_service_does_not_replace_installed_assets_with_worktree_symlink() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    ensure_body = justfile.split("_ensure-service: _sign", 1)[1].split(
        "\n# Start service daemon", 1
    )[0]

    assert "ln -sfn" not in ensure_body
    assert "assets.installed" not in ensure_body
    assert "Symlinked $ASSETS_LINK" not in ensure_body
    assert "sync-dev-assets.sh" in ensure_body
    assert "retired_config_removed" in ensure_body


def test_installers_remove_retired_user_and_service_config_rails() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert 'retired_user_config="user"".toml"' in text
        assert '"$CAPSEM_DIR/service.toml"' in text or '"$CAPSEM_HOME_DIR/service.toml"' in text
        assert "retired_config_removed" in text


def test_installers_remove_retired_python_admin_bundle() -> None:
    scripts = [
        PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall",
        PROJECT_ROOT / "scripts" / "deb-postinst.sh",
        PROJECT_ROOT / "scripts" / "simulate-install.sh",
    ]

    for path in scripts:
        text = path.read_text()
        assert "capsem-admin-python" in text
        assert "retired_python_admin_bundle_removed" in text


def test_native_postinstall_merges_fresh_check_into_manifest_metadata() -> None:
    for relative in ("scripts/pkg-scripts/postinstall", "scripts/deb-postinst.sh"):
        script = (PROJECT_ROOT / relative).read_text()
        metadata = script.index("manifest-metadata.json")
        hydrate = script.index('update --assets --manifest \\"$MANIFEST_SOURCE\\"')
        refresh = script.index("update --check", hydrate)

        assert metadata < hydrate < refresh, relative
        assert "CAPSEM_RELEASE_MANIFEST_URL" not in script[refresh - 240 : refresh], relative
        assert "update-check.json" not in script, relative
        assert "update-checks" not in script, relative
        assert "update_status_refreshed" in script[refresh:], relative


def test_manifest_generation_public_path_is_capsem_admin() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    public_docs = [
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "asset-pipeline.md",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "security" / "build-verification.md",
        PROJECT_ROOT / "skills" / "asset-pipeline" / "SKILL.md",
        PROJECT_ROOT / "skills" / "release-process" / "SKILL.md",
    ]

    assert "capsem-admin -- manifest generate" in justfile
    assert "scripts/gen_manifest.py" not in justfile
    assert '(cd "$ASSETS" && b3sum' not in justfile
    for path in public_docs:
        text = path.read_text()
        assert "capsem-admin manifest generate" in text
        assert "scripts/gen_manifest.py" not in text


def test_package_builders_stage_manifest_only_not_vm_asset_payload() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    pkg_preinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "preinstall").read_text()
    pkg_postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    pkg_install_user = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "install-user").read_text()

    assert "CAPSEM_PKG_ASSET_MODE" not in build_pkg
    assert "ASSET_MODE=" not in build_pkg
    assert "export COPYFILE_DISABLE=1" in build_pkg
    assert "--manifest" in build_pkg
    assert 'MANIFEST_PATH="${2:?--manifest requires a URL}"' in build_pkg
    assert "materialize_manifest_input" not in build_pkg
    assert "materialize-package-manifest.py" not in build_pkg
    assert 'parsed.scheme not in ("http", "https", "file")' in build_pkg
    assert "urllib.request.Request(" not in build_pkg
    assert "CapsemReleaseValidator/1.0" not in build_pkg
    assert "urllib.request.urlopen" not in build_pkg
    assert "manifest must be a URL" in build_pkg
    assert "pathlib.Path(source).read_bytes()" not in build_pkg
    assert '--version "$VERSION"' in build_pkg
    assert "PKG_VERSION" not in build_pkg
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in build_pkg
    )
    assert (
        'install -m 0644 "$ASSETS_VIEW/manifest.json" "$SHARE_DIR/assets/manifest.json"'
        not in build_pkg
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in build_pkg
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$VERSION" "$SHARE_DIR/assets/manifest-metadata.json"'
        in build_pkg
    )
    assert "snapshot_sha256" not in build_pkg
    assert "materialize_manifest_assets" not in build_pkg
    assert "Added asset:" not in build_pkg
    assert "rootfs-" not in build_pkg
    assert "initrd-" not in build_pkg
    assert "vmlinuz-" not in build_pkg
    assert "obom-" not in build_pkg
    assert "sync-dev-assets.sh" not in build_pkg
    assert 'CONFIG_ROOT="${POSITIONAL[3]}"' in build_pkg
    assert 'ditto --norsrc --noextattr "$src" "$dst"' in build_pkg
    assert 'copy_tree_clean "$CONFIG_ROOT/profiles" "$SHARE_DIR/profiles"' in build_pkg
    assert 'install -m 0755 "$SCRIPT_DIR/pkg-scripts/preinstall"' in build_pkg
    assert 'install -m 0755 "$SCRIPT_DIR/pkg-scripts/install-user"' in build_pkg
    assert 'xattr -rc "$WORK_DIR/payload" "$PKG_SCRIPTS"' in build_pkg
    assert 'find "$WORK_DIR/payload" "$PKG_SCRIPTS" -name' in build_pkg
    assert '--scripts "$PKG_SCRIPTS"' in build_pkg
    assert "--filter '/\\._[^/]*$'" in build_pkg
    assert "capsem-admin" in build_pkg
    assert "capsem-tui" in build_pkg
    assert "rm -rf /Applications/Capsem.app" in pkg_preinstall
    assert "event=remove_user_app_payload" in pkg_preinstall
    assert 'rm -rf "$USER_HOME/Applications/Capsem.app"' in pkg_preinstall
    assert "rm -rf /usr/local/share/capsem" in pkg_preinstall
    assert "pkill -9 -x capsem-app" in pkg_preinstall
    assert "capsem stop" not in pkg_preinstall
    assert "$CAPSEM_DIR/bin/capsem" not in pkg_preinstall
    assert "event=stop_existing_service" not in pkg_preinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in pkg_preinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in pkg_preinstall
    assert "install-current-run" in pkg_preinstall
    assert "install-latest.log" in pkg_preinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in pkg_preinstall
    assert 'source "$(dirname "$0")/install-user"' in pkg_preinstall
    assert "capsem_resolve_install_user" in pkg_preinstall

    assert "CAPSEM_DEB_ASSET_MODE" not in repack_deb
    assert "ASSET_MODE=" not in repack_deb
    assert "export COPYFILE_DISABLE=1" in repack_deb
    assert "strip_packaged_binaries" in repack_deb
    assert "CAPSEM_REPACK_STRIP" not in repack_deb
    assert '"$strip_tool" --strip-unneeded "$path"' in repack_deb
    assert 'CONFIG_ROOT="${POSITIONAL[2]}"' in repack_deb
    assert "--manifest" in repack_deb
    assert "materialize_manifest_input" not in repack_deb
    assert "materialize-package-manifest.py" not in repack_deb
    assert 'parsed.scheme not in ("http", "https", "file")' in repack_deb
    assert "urllib.request.Request(" not in repack_deb
    assert "CapsemReleaseValidator/1.0" not in repack_deb
    assert "urllib.request.urlopen" not in repack_deb
    assert "manifest must be a URL" in repack_deb
    assert "pathlib.Path(source).read_bytes()" not in repack_deb
    assert "BUILD_TS=" not in repack_deb
    assert (
        'materialize_manifest_input "$MANIFEST_PATH" "$ASSETS_VIEW/manifest.json"' not in repack_deb
    )
    assert (
        'cp "$ASSETS_VIEW/manifest.json" "$WORK_DIR/deb/usr/share/capsem/assets/manifest.json"'
        not in repack_deb
    )
    assert 'SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"' in repack_deb
    assert 'PACKAGE_VERSION="$(dpkg-deb -f "$INPUT_DEB" Version)"' in repack_deb
    assert (
        'write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$PACKAGE_VERSION" "$WORK_DIR/deb/usr/share/capsem/assets/manifest-metadata.json"'
        in repack_deb
    )
    assert "snapshot_sha256" not in repack_deb
    assert "materialize_manifest_assets" not in repack_deb
    assert "Added asset:" not in repack_deb
    assert "rootfs-" not in repack_deb
    assert "initrd-" not in repack_deb
    assert "vmlinuz-" not in repack_deb
    assert "obom-" not in repack_deb
    assert (
        'cp -R "$CONFIG_ROOT/profiles/." "$WORK_DIR/deb/usr/share/capsem/profiles/"' in repack_deb
    )
    assert "sync-dev-assets.sh" not in repack_deb
    assert "capsem-admin" in repack_deb
    assert "capsem-tui" in repack_deb
    assert "/usr/share/capsem/assets" in deb_postinst
    assert "/usr/share/capsem/profiles" in deb_postinst
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest.json "$CAPSEM_DIR/assets/manifest.json"'
        not in deb_postinst
    )
    assert (
        'install -m 0644 /usr/share/capsem/assets/manifest-metadata.json "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in deb_postinst
    )
    assert "event=manifest_copied" not in deb_postinst
    assert "manifest check" not in deb_postinst
    assert "event=manifest_report" not in deb_postinst
    assert "MANIFEST_METADATA=$(tr" in deb_postinst
    assert "event=manifest_metadata" in deb_postinst
    assert "MANIFEST_SOURCE=$(sed" in deb_postinst
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"'
        not in deb_postinst
    )
    assert "packaged manifest-metadata.json missing" in deb_postinst
    assert "packaged manifest-metadata.json has no manifest_url" in deb_postinst
    assert "event=manifest_source" in deb_postinst
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in deb_postinst
    )
    assert "event=assets_hydrated" in deb_postinst
    assert "event=asset_hydration_failed" in deb_postinst
    assert "event=assets_copied" not in deb_postinst
    assert 'echo "capsem: packaged binary missing: /usr/bin/$bin" >&2' in deb_postinst
    assert "event=binary_missing bin=$bin" in deb_postinst
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in deb_postinst
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in deb_postinst
    assert "install-current-run" in deb_postinst
    assert "install-latest.log" in deb_postinst
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in deb_postinst
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' in deb_postinst
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' in deb_postinst
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' in deb_postinst
    assert "event=profiles_not_ready" in deb_postinst
    assert "capsem-admin" in deb_postinst
    assert "capsem-tui" in deb_postinst

    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest.json" "$CAPSEM_DIR/assets/manifest.json"'
        not in pkg_postinstall
    )
    assert (
        'install -m 0644 "$PKG_SHARE/assets/manifest-metadata.json" "$CAPSEM_DIR/assets/manifest-metadata.json"'
        in pkg_postinstall
    )
    assert "event=manifest_copied" not in pkg_postinstall
    assert "manifest check" not in pkg_postinstall
    assert "event=manifest_report" not in pkg_postinstall
    assert "MANIFEST_METADATA=$(tr" in pkg_postinstall
    assert "event=manifest_metadata" in pkg_postinstall
    assert "MANIFEST_SOURCE=$(sed" in pkg_postinstall
    assert (
        'MANIFEST_SOURCE="https://release.capsem.org/assets/stable/manifest.json"'
        not in pkg_postinstall
    )
    assert "packaged manifest-metadata.json missing" in pkg_postinstall
    assert "packaged manifest-metadata.json has no manifest_url" in pkg_postinstall
    assert "event=manifest_source" in pkg_postinstall
    assert (
        'CAPSEM_HOME=\\"$CAPSEM_DIR\\" CAPSEM_RUN_DIR=\\"$CAPSEM_DIR/run\\" \\"$CAPSEM_DIR/bin/capsem\\" update --assets --manifest \\"$MANIFEST_SOURCE\\"'
        in pkg_postinstall
    )
    assert "event=assets_hydrated" in pkg_postinstall
    assert "event=asset_hydration_failed" in pkg_postinstall
    assert "event=assets_copied" not in pkg_postinstall
    assert 'echo "capsem: packaged binary missing: $src" >&2' in pkg_postinstall
    assert "event=binary_missing bin=$bin" in pkg_postinstall
    assert 'source "$(dirname "$0")/install-user"' in pkg_postinstall
    assert "capsem_resolve_install_user" in pkg_postinstall
    assert "skipping per-user install" not in pkg_postinstall
    assert "secure install-user request" in pkg_install_user
    assert "/var/run/capsem/install-user" in pkg_install_user
    assert 'rm -rf "$CAPSEM_DIR"/bin.backup*' in pkg_postinstall
    assert "event=retired_binary_backups_removed" in pkg_postinstall


def test_macos_postinstall_adds_capsem_bin_to_fish_path() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert ".config/fish/config.fish" in postinstall
    assert "fish_add_path" in postinstall
    assert "grep -qF 'fish_add_path --path \"$HOME/.capsem/bin\"'" in postinstall
    assert 'cp -R "$PKG_SHARE/assets/"* "$CAPSEM_DIR/assets/"' not in postinstall
    assert "pkill -x capsem-app" in postinstall
    assert 'INSTALL_LOG="$CAPSEM_DIR/logs/install.log"' in postinstall
    assert 'INSTALL_RUN_ID=$(cat "$INSTALL_RUN_FILE" 2>/dev/null || date' in postinstall
    assert 'INSTALL_RUN_LOG="$CAPSEM_DIR/logs/install-$INSTALL_RUN_ID.log"' in postinstall
    assert "install-latest.log" in postinstall
    assert 'exec > >(tee -a "$INSTALL_LOG" "$INSTALL_RUN_LOG") 2>&1' in postinstall
    assert "event=readiness_poll" in postinstall
    assert "attempt=$attempt" in postinstall
    assert 'PROFILE_COUNTS=$(echo "$STATUS_OUTPUT" | sed -n' in postinstall
    assert '[ "$READY_PROFILES" = "$TOTAL_PROFILES" ]' in postinstall
    assert '[ "$TOTAL_PROFILES" -gt 0 ]' in postinstall
    assert "event=profiles_not_ready" in postinstall


def test_linux_postinstall_prints_service_journal_on_readiness_failure() -> None:
    postinstall = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()

    assert "event=service_diagnostics" in postinstall
    assert "systemctl --user status capsem.service --no-pager -l" in postinstall
    assert "journalctl --user-unit capsem.service --no-pager -n 100" in postinstall


def test_release_workflow_decouples_vm_assets_and_keeps_full_host_binary_set() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "run: just test" not in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "kind: profiles" in workflow
    assert "output: target/binary-public-before/profiles" in workflow
    assert "output: target/candidate-profile-inputs" in workflow
    assert "--input-dir target/candidate-profile-inputs" in workflow
    assert "just _test-artifacts" in workflow
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow
    assert "just _build-kernel" not in workflow
    assert "just _build-rootfs" not in workflow
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "-p capsem-admin" in workflow


def test_release_workflow_retries_app_cargo_tool_installs() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    build_app_macos = workflow.split("  build-app-macos:", 1)[1].split("\n  build-app-linux:", 1)[0]
    build_app_linux = workflow.split("  build-app-linux:", 1)[1].split("\n  create-release:", 1)[0]

    assert "cargo install tauri-cli cargo-auditable cargo-sbom --locked" not in workflow
    assert "cargo install tauri-cli cargo-auditable --locked" not in workflow

    for block, required_tools in (
        (build_app_macos, ("tauri-cli", "cargo-auditable")),
        (build_app_linux, ("tauri-cli", "cargo-auditable")),
    ):
        assert "CARGO_NET_RETRY: 10" in block
        assert "install_cargo_tool() {" in block
        assert "for attempt in 1 2 3; do" in block
        assert 'cargo install "$tool" --locked' in block
        assert 'echo "cargo install $tool failed on attempt $attempt/3"' in block
        for tool in required_tools:
            assert f"install_cargo_tool {tool}" in block
    assert "cargo install cargo-sbom --locked" in build_app_macos
    assert "cargo install cargo-sbom --locked" not in build_app_linux
    assert "install_cargo_tool cargo-sbom" not in workflow
    assert "-p capsem-tui" in workflow
    assert "-p capsem-mcp-aggregator" in workflow
    assert "-p capsem-mcp-builtin" in workflow
    assert "capsem-admin" in workflow
    assert "capsem-tui" in workflow
    assert "capsem-mcp-aggregator" in workflow
    assert "capsem-mcp-builtin" in workflow


def test_release_workflow_sets_up_uv_before_uv_run_steps() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    jobs_with_uv = {
        name: block for name, block in _workflow_job_blocks(workflow).items() if "uv run" in block
    }

    assert jobs_with_uv
    for name, block in jobs_with_uv.items():
        setup_pos = block.find("astral-sh/setup-uv@")
        uv_run_pos = block.find("uv run")
        assert setup_pos != -1, f"{name} uses uv run without setup-uv"
        assert setup_pos < uv_run_pos, f"{name} sets up uv after first uv run"


def test_ci_install_job_sets_up_uv_before_the_shared_install_gate() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = _workflow_job_blocks(workflow)["test-install"]

    setup_pos = install_job.find("astral-sh/setup-uv@")
    install_pos = install_job.find("just _gate-install")
    assert setup_pos != -1, "test-install invokes uv-backed Just helpers without setup-uv"
    assert setup_pos < install_pos, "test-install sets up uv after the shared install gate"


def test_ci_install_job_pulls_existing_profiles_before_building_packages() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = _workflow_job_blocks(workflow)["test-install"]
    fetch_action = (
        PROJECT_ROOT / ".github" / "actions" / "fetch-release-inputs" / "action.yaml"
    ).read_text()

    fetch_pos = install_job.index("./.github/actions/fetch-release-inputs")
    package_pos = install_job.index("just _cross-compile x86_64")
    gate_pos = install_job.index("just _gate-install")
    assert fetch_pos < package_pos < gate_pos
    assert "kind: profiles" in install_job
    assert "architecture: x86_64" in install_job
    assert "output: target/ci-install-profile-inputs" in install_job
    assert "Build exact native release package" in install_job
    assert "CAPSEM_INSTALL_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json" in (
        install_job
    )
    assert "CAPSEM_INSTALL_CHANNEL: stable" in install_job
    assert "just _build-host-image" not in install_job
    assert "actions/cache/restore@" in fetch_action
    assert "actions/cache/save@" in fetch_action
    assert "--cache-dir target/release-input-cache" in fetch_action
    assert "--prune-cache" not in fetch_action
    assert "steps.fetch.outputs.cache-misses != '0'" in fetch_action
    assert "inputs.manifest-url" not in fetch_action.split("key:", 1)[1].splitlines()[0]
    assert "inputs.channel" not in fetch_action
    assert (
        "CAPSEM_INSTALL_PROFILE_INPUTS=target/ci-install-profile-inputs just _gate-install"
    ) in install_job
    assert "scripts/prepare-install-test-assets.sh" not in install_job


def test_installed_doctor_failure_is_printed_and_preserved() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    probe = script.split("probe_installed_transition() {{", maxsplit=1)[1].split(
        "\n}}\nwait_for_exact_transition()", maxsplit=1
    )[0]

    assert 'doctor_log="$EVIDENCE_DIR/$label-doctor.log"' in probe
    assert 'failed_process_logs="$EVIDENCE_DIR/$label-failed-process-logs.txt"' in probe
    assert 'if ! CAPSEM_HOME="$CAPSEM_HOME_DIR" CAPSEM_RUN_DIR="$CAPSEM_HOME_DIR/run"' in probe
    assert 'find "$CAPSEM_HOME_DIR/run/sessions"' in probe
    assert 'cat "$doctor_log" >&2' in probe
    assert 'cat "$failed_process_logs" >&2' in probe


def test_ci_install_job_uploads_glowup_evidence_on_failure() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = _workflow_job_blocks(workflow)["test-install"]

    assert "Upload install and glow-up evidence on failure" in install_job
    assert "if: failure()" in install_job
    assert "uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a" in install_job
    assert "target/local-release-glowup/" in install_job
    assert "if-no-files-found: warn" in install_job


def test_asset_build_recipes_skip_kvm_only_for_build_prereq_doctor() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    doctor_linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()

    assert "CAPSEM_SKIP_KVM_CHECK" in doctor_linux
    assert 'skip "/dev/kvm (CAPSEM_SKIP_KVM_CHECK set)"' in doctor_linux

    for recipe in ("_build-kernel", "_build-rootfs", "_build-assets"):
        block = justfile.split(f"\n{recipe} ", 1)[1].split("\n# ", 1)[0]
        assert "CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor" in block

    smoke_block = justfile.split("\nsmoke", 1)[1].split("\n# ", 1)[0]
    assert "CAPSEM_SKIP_KVM_CHECK" not in smoke_block


def test_only_systemd_package_proof_receives_kvm_devices() -> None:
    cross_compile = _just_recipe_block("_cross-compile")
    proof = _just_recipe_block("_prove-linux-deb")

    assert "DOCKER_KVM_ARGS" not in cross_compile
    assert "--device /dev/kvm" not in cross_compile
    assert "--device /dev/vhost-vsock" not in cross_compile
    assert "DEVICE_ARGS=(" in proof
    assert "--device /dev/kvm" in proof
    assert "--device /dev/vhost-vsock" in proof
    assert '"${DEVICE_ARGS[@]}"' in proof


def test_cross_compile_clock_sync_uses_bounded_colima_command() -> None:
    cross_compile = _just_recipe_block("_cross-compile")

    assert "python3 scripts/sync-container-clock.py" in cross_compile
    assert "docker run --rm --privileged alpine date" not in cross_compile


def test_security_event_rows_go_through_security_engine_emitter() -> None:
    roots = [
        PROJECT_ROOT / "crates" / "capsem-core" / "src",
        PROJECT_ROOT / "crates" / "capsem-process" / "src",
    ]
    allowed_files = {
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "mod.rs",
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "security_engine" / "tests.rs",
    }
    patterns = [
        "write(WriteOp::",
        "write(capsem_logger::WriteOp::",
        "try_write(WriteOp::",
        "try_write(capsem_logger::WriteOp::",
        "try_emit_security_write(",
    ]

    violations: list[str] = []
    for root in roots:
        for path in root.rglob("*.rs"):
            if path in allowed_files or "/tests/" in path.as_posix():
                continue
            text = path.read_text()
            for lineno, line in enumerate(text.splitlines(), start=1):
                if any(pattern in line for pattern in patterns):
                    rel = path.relative_to(PROJECT_ROOT)
                    violations.append(f"{rel}:{lineno}: {line.strip()}")

    assert not violations, (
        "security/logging rows must be emitted through "
        "capsem_core::security_engine::{emit_security_write,emit_security_write_blocking}; "
        "direct DbWriter WriteOp sends found:\n" + "\n".join(violations)
    )

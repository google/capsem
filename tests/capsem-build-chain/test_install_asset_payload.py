"""Install package asset-payload contract tests."""

import errno
import importlib.util
import os
import re
import subprocess
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest


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


def _just_recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    block = "\n".join(lines[start:end])
    if name == "test:":
        block = f"{block}\n{_just_recipe_block('_test-candidate:')}"
    return block


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
    spec.loader.exec_module(module)
    return module


def _run_docker_space_gate(
    tmp_path: Path,
    *,
    before_kib: int,
    after_kib: int,
    after_trim_kib: int | None = None,
    volumes: str = "",
    minimum_gib: int = 16,
    cache_keep_gib: int | str | None = None,
    linked_keep_gib: int | str | None = None,
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
            if [ "$phase" = "trimmed" ]; then
                free="$FAKE_DOCKER_AFTER_TRIM_KIB"
            elif [ "$phase" = "pruned" ]; then
                free="$FAKE_DOCKER_AFTER_KIB"
            else
                free="$FAKE_DOCKER_BEFORE_KIB"
            fi
            printf '%s\\n' "$free"
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
elif [ "$1" = "volume" ] && [ "$2" = "ls" ]; then
    printf '%s\\n' "$FAKE_DOCKER_VOLUMES"
elif [ "$1" = "ps" ]; then
    :
elif [ "$1" = "system" ] && [ "$2" = "df" ]; then
    printf 'fake docker disk report\\n'
else
    printf 'unexpected fake docker command: %s\\n' "$*" >&2
    exit 97
fi
"""
    )
    docker.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{fake_bin}:{env['PATH']}",
            "FAKE_DOCKER_STATE": str(state),
            "FAKE_DOCKER_BEFORE_KIB": str(before_kib),
            "FAKE_DOCKER_AFTER_KIB": str(after_kib),
            "FAKE_DOCKER_AFTER_TRIM_KIB": str(
                after_kib if after_trim_kib is None else after_trim_kib
            ),
            "FAKE_DOCKER_VOLUMES": volumes,
            "FAKE_DOCKER_COMMANDS": str(commands),
        }
    )
    if cache_keep_gib is not None:
        env["CAPSEM_DOCKER_CACHE_KEEP_GB"] = str(cache_keep_gib)
    if linked_keep_gib is not None:
        env["CAPSEM_DOCKER_LINKED_KEEP_GB"] = str(linked_keep_gib)
    return subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / "scripts" / "ensure-docker-space.sh"),
            str(minimum_gib),
        ],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )


def test_asset_gate_owns_docker_capacity_preflight(tmp_path: Path) -> None:
    recipe = _just_recipe_block("test-assets:")

    preflight = 'CAPSEM_DOCKER_CACHE_KEEP_GB=4 "$ROOT/scripts/ensure-docker-space.sh" 16'
    assert preflight in recipe
    assert recipe.index(preflight) < recipe.index("build_arch_lane arm64")

    enough = _run_docker_space_gate(tmp_path / "enough", before_kib=20 * 1024 * 1024, after_kib=0)
    assert enough.returncode == 0, enough.stderr
    assert "already has" in enough.stdout

    reclaimed = _run_docker_space_gate(
        tmp_path / "reclaimed",
        before_kib=8 * 1024 * 1024,
        after_kib=20 * 1024 * 1024,
    )
    assert reclaimed.returncode == 0, reclaimed.stderr
    assert "pruning unused builder cache" in reclaimed.stdout
    assert "reclaimed enough space" in reclaimed.stdout
    reclaimed_commands = (tmp_path / "reclaimed" / "docker-commands").read_text()
    assert "builder prune -f --keep-storage 8GB" in reclaimed_commands
    assert "builder prune -af" not in reclaimed_commands

    package_reclaimed = _run_docker_space_gate(
        tmp_path / "package-reclaimed",
        before_kib=10 * 1024 * 1024,
        after_kib=16 * 1024 * 1024,
        minimum_gib=14,
        cache_keep_gib=2,
    )
    assert package_reclaimed.returncode == 0, package_reclaimed.stderr
    assert "hottest 2 GiB" in package_reclaimed.stdout
    package_commands = (tmp_path / "package-reclaimed" / "docker-commands").read_text()
    assert "builder prune -f --keep-storage 2GB" in package_commands

    invalid_floor = _run_docker_space_gate(
        tmp_path / "invalid-floor",
        before_kib=20 * 1024 * 1024,
        after_kib=0,
        cache_keep_gib="all",
    )
    assert invalid_floor.returncode == 2
    assert "cache floor must be a positive GiB integer" in invalid_floor.stderr

    invalid_linked_floor = _run_docker_space_gate(
        tmp_path / "invalid-linked-floor",
        before_kib=20 * 1024 * 1024,
        after_kib=0,
        linked_keep_gib="all",
    )
    assert invalid_linked_floor.returncode == 2
    assert "linked-artifact floor must be a positive GiB integer" in invalid_linked_floor.stderr

    exhausted = _run_docker_space_gate(
        tmp_path / "exhausted",
        before_kib=8 * 1024 * 1024,
        after_kib=10 * 1024 * 1024,
    )
    assert exhausted.returncode != 0
    assert "requires at least 16 GiB" in exhausted.stderr
    assert "fake docker disk report" in exhausted.stderr

    trimmed = _run_docker_space_gate(
        tmp_path / "trimmed",
        before_kib=8 * 1024 * 1024,
        after_kib=10 * 1024 * 1024,
        after_trim_kib=20 * 1024 * 1024,
        volumes="capsem-install-target",
    )
    assert trimmed.returncode == 0, trimmed.stderr
    assert "trimming inactive Cargo incremental cache: capsem-install-target" in trimmed.stdout

    storage_script = (PROJECT_ROOT / "scripts" / "ensure-docker-space.sh").read_text()
    assert "trimming inactive Cargo linked artifacts" in storage_script
    assert '[[ "$volume" == capsem-*-target* ]]' in storage_script
    assert 'docker ps -q --filter "volume=$volume"' in storage_script
    assert 'find "$deps" -maxdepth 1 -type f ! -name "*.*"' in storage_script
    assert "Dependency libraries (.rlib/.rmeta/etc.) are deliberately untouched" in storage_script


def test_just_install_does_not_sync_assets_after_installer() -> None:
    install_body = _just_recipe_block("install:")

    assert "Syncing local dev assets" not in install_body
    assert "scripts/sync-dev-assets.sh" not in install_body
    assert "CAPSEM_PKG_ASSET_MODE=current-arch bash scripts/build-pkg.sh" not in install_body
    assert "CAPSEM_DEB_ASSET_MODE=current-arch bash scripts/repack-deb.sh" not in install_body
    assert "bash scripts/build-pkg.sh" in install_body
    assert "bash scripts/repack-deb.sh --manifest" in install_body
    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in install_body
    )
    assert '--manifest "$MANIFEST_URL"' in install_body
    assert "file://$PWD/{{assets_dir}}/manifest.json" not in install_body
    assert '"target/config"' in install_body
    assert (
        "install: _pnpm-install _stamp-version _check-assets _pack-initrd _materialize-config"
        in install_body
    )
    assert "pkill -9 -x capsem-app" in install_body


def test_just_install_invokes_package_without_gui_installer_block() -> None:
    install_body = _just_recipe_block("install:")

    assert 'PKG="packages/Capsem-$VERSION.pkg"' in install_body
    assert 'open -W "$PKG"' not in install_body
    assert 'installer -pkg "$PKG"' in install_body
    assert '"$HOME/.capsem/bin/capsem" status' in install_body
    assert '"$HOME/.capsem/bin/capsem" debug' in install_body


def test_cross_compile_repacks_deb_before_exact_systemd_install_proof() -> None:
    block = _just_recipe_block("cross-compile")

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
    assert "capsem-admin)\\$'" in block
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
    install_gate = _just_recipe_block("test-install:")

    assert "/etc/systemd/system/systemd-binfmt.service" in dockerfile
    assert "ln -s /dev/null" in dockerfile
    assert "HOST_ROSETTA_REGISTRATION=required" in install_gate
    assert install_gate.count("/proc/sys/fs/binfmt_misc/rosetta") >= 2
    assert "systemd install container removed Colima's Rosetta binfmt registration" in install_gate


def test_release_qualification_requires_exact_linux_deb_proof() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-qualification.yaml").read_text()

    assert 'CAPSEM_REQUIRE_LINUX_DEB_PROOF: "1"' in workflow
    assert "runs-on: ubuntu-24.04" in workflow
    assert "platforms: arm64" in workflow
    assert (
        "CAPSEM_INSTALL_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert "CAPSEM_INSTALL_CHANNEL: ${{ inputs.channel }}" in workflow


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
    linux = _workflow_job_blocks(workflow)["build-app-linux"]

    assert "runner: ubuntu-24.04-arm" in linux
    assert "runner: ubuntu-24.04" in linux
    assert linux.count("if: matrix.arch == 'x86_64'") == 2
    assert "Enable KVM for exact-package VM proof" in linux
    assert "Prove exact-package guest shell execution" in linux
    assert "CAPSEM_EXACT_PACKAGE_SHELL_OK" in linux


def test_install_test_restores_host_workspace_ownership() -> None:
    block = _just_recipe_block("test-install")

    assert "HOST_UID=$(id -u)" in block
    assert "HOST_GID=$(id -g)" in block
    assert "chown -R $HOST_UID:$HOST_GID /src" in block
    assert "trap cleanup EXIT" in block
    assert 'docker rm -f "$CONTAINER"' in block
    cleanup = block.split("cleanup() {", maxsplit=1)[1].split("}", maxsplit=1)[0]
    assert 'docker image rm "$IMAGE:latest"' in cleanup


def test_install_test_keeps_frontend_build_outputs_container_owned() -> None:
    block = _just_recipe_block("test-install")

    assert "-v capsem-install-frontend-node-modules:/src/frontend/node_modules" in block
    assert "-v capsem-install-frontend-dist:/src/frontend/dist" in block
    assert "chown -R capsem:capsem /src/frontend/node_modules /src/frontend/dist" in block


def test_install_test_removes_stale_container_before_fail_closed_cache_reset() -> None:
    block = _just_recipe_block("test-install")

    remove_stale = block.index('docker rm -f "$CONTAINER"')
    inspect_cache = block.index("VOLUME_LINE=$(docker system df -v")
    reset_cache = block.index("docker volume rm capsem-install-target")

    assert remove_stale < inspect_cache < reset_cache
    assert "docker volume rm capsem-install-target >/dev/null 2>&1 || true" not in block
    assert "Failed to reset oversized capsem-install-target volume" in block
    assert "docker ps -a --filter volume=capsem-install-target" in block


def test_install_test_runs_local_release_glowup_from_real_package() -> None:
    block = _just_recipe_block("test-install").replace(r"\"", '"').replace(r"\$", "$")

    assert "Running local release glow-up" in block
    assert "scripts/local-release-glowup.py" in block
    assert '--input-deb "$DEB"' in block
    assert "--bin-dir /cargo-target/debug" in block
    assert '--assets-dir "$INSTALL_ASSETS_DIR"' in block
    assert '--config-root "$INSTALL_CONFIG_DIR"' in block
    assert "just test-install" in _just_recipe_block("test:")


def test_install_test_uses_clean_isolated_asset_fixtures() -> None:
    block = _just_recipe_block("test-install").replace(r"\"", '"').replace(r"\$", "$")
    update_tests = (PROJECT_ROOT / "tests/capsem-install/test_update.py").read_text()

    assert 'INSTALL_ASSETS_DIR="target/install-test-assets"' in block
    assert 'INSTALL_CONFIG_DIR="target/install-test-config"' in block
    assert 'rm -rf "$INSTALL_ASSETS_DIR" "$INSTALL_CONFIG_DIR"' in block
    assert (
        'CAPSEM_ASSETS_DIR="$INSTALL_ASSETS_DIR" bash scripts/prepare-install-test-assets.sh'
    ) in block
    assert (
        'CAPSEM_ASSETS_DIR="$INSTALL_ASSETS_DIR" '
        'CAPSEM_CONFIG_OUTPUT_ROOT="/src/$INSTALL_CONFIG_DIR" '
        "bash scripts/materialize-config.sh"
    ) in block
    assert '"$INSTALL_CONFIG_DIR" "$INSTALL_ASSETS_DIR"' in block
    assert 'CAPSEM_TEST_ASSET_MANIFEST="/src/$INSTALL_ASSETS_DIR/manifest.json"' in block
    assert '--assets-dir "$INSTALL_ASSETS_DIR"' in block
    assert '--config-root "$INSTALL_CONFIG_DIR"' in block
    assert '"target" / "install-test-assets" / "manifest.json"' in update_tests
    assert 'REPO_ROOT / "assets" / "manifest.json"' not in update_tests
    assert "run scripts/prepare-install-test-assets.sh before install tests" in update_tests


def test_local_release_glowup_uses_real_release_pipeline_not_fake_manifest() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "scripts/repack-deb.sh" in script
    assert "scripts/generate-host-binary-sbom.py" in script
    assert "record-binary" in script
    assert "assets" in script and "channel" in script and "build" in script
    assert "json.dumps({" not in script or "capsem.local_release_glowup.v1" in script
    assert "stable-assets-manifest.json" in script
    assert "nightly-assets-manifest.json" in script
    assert 'shutil.copy2(args.assets_dir / "manifest.json"' in script
    assert "CAPSEM_RELEASE_URL" in script
    assert "CAPSEM_RELEASE_CHANNELS_URL=" in script
    assert "update --assets --channel nightly" in script
    assert "update --assets --channel stable" in script
    assert "update --yes --channel nightly" not in script
    assert "update --yes --channel stable" not in script
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
    recipe = justfile.split("test-install:", maxsplit=1)[1].split(
        "\n# Dispatch one serialized release workflow", maxsplit=1
    )[0]

    # /src is bind-mounted and may contain a host .venv whose console-script
    # shebang cannot exist in the Linux container. Launch via Python so uv's
    # selected interpreter owns module resolution instead.
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in recipe
    assert "uv run python -m pytest tests/capsem-install/" in recipe
    assert "uv run pytest tests/capsem-install/" not in recipe


def test_full_gate_preflights_clean_install_harness_before_expensive_stages() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    full_gate = _just_recipe_block("test:")
    preflight = justfile.split("_test-install-harness-preflight:", maxsplit=1)[1].split(
        "\ntest-install:", maxsplit=1
    )[0]

    assert "just _test-install-harness-preflight" in full_gate
    assert full_gate.index("just _test-install-harness-preflight") < full_gate.index(
        "cargo clippy --workspace --all-targets"
    )
    assert "docker/Dockerfile.install-test" in preflight
    assert "source /src/scripts/doctor-linux.sh" in preflight
    assert "linux_musl_toolchain_available" in preflight
    assert preflight.index("linux_musl_toolchain_available") < preflight.index(
        "uv run python -m pytest --version"
    )
    assert "UV_PROJECT_ENVIRONMENT=/home/capsem/.venv-install-test" in preflight
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
    assert "just build-host-image" in preflight
    assert "if ! docker image inspect capsem-host-builder" not in preflight
    assert "cdxgen --version" in preflight
    assert preflight.index("cdxgen --version") < preflight.index(
        "uv run python -m pytest --version"
    )
    verify = "check_install_image"
    release_base = "docker image rm capsem-host-builder:latest"
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
    cross_compile = _just_recipe_block("cross-compile ")

    frontend = "echo '--- Build frontend ---'"
    swap = "swap-dev-libs \\$DPKG_ARCH"
    rust = "echo '--- Build agent binaries ---'"
    assert cross_compile.index(frontend) < cross_compile.index(swap)
    assert cross_compile.index(swap) < cross_compile.index(rust)


def test_cross_compile_reasserts_pinned_rust_target_before_expensive_work() -> None:
    """A persistent rustup volume must not mask the builder's pinned targets."""
    cross_compile = _just_recipe_block("cross-compile ")

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
    cross_compile = _just_recipe_block("cross-compile ")
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text()

    assert "just build-host-image" in cross_compile
    assert "docker image inspect capsem-host-builder:latest" not in cross_compile
    assert host_builder.index("COPY swap-dev-libs.sh") > host_builder.index(
        "cargo install tauri-cli"
    )


def test_cross_compile_preflights_docker_capacity_after_builder_before_package() -> None:
    """Asset lanes must not leave Linux package builds at zero Docker disk."""
    cross_compile = _just_recipe_block("cross-compile ")

    build_image = cross_compile.index("just build-host-image")
    release_completed_rails = cross_compile.index("just _release-completed-docker-rails")
    release_install_target = cross_compile.index("just _release-deferred-install-target")
    release_asset_builder = cross_compile.index("docker image rm rust:slim-bookworm")
    capacity = (
        "CAPSEM_DOCKER_CACHE_KEEP_GB=2 CAPSEM_DOCKER_LINKED_KEEP_GB=2 "
        '"$ROOT/scripts/ensure-docker-space.sh" 14'
    )
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
        < release_asset_builder
        < capacities[0]
        < build_image
        < capacities[1]
        < package
    )
    assert cross_compile.count(capacity) == 2
    assert "docker image rm -f rust:slim-bookworm" not in cross_compile
    post_builder = cross_compile[build_image:package]
    assert capacity in post_builder
    assert 'scripts/ensure-docker-space.sh" 16' not in cross_compile


def test_package_boundary_releases_only_completed_docker_rail_volumes() -> None:
    release = _just_recipe_block("_release-completed-docker-rails:")

    assert "capsem-agent-target-arm64" in release
    assert "capsem-agent-target-x86_64" in release
    assert "capsem-rustup-arm64" in release
    assert "capsem-rustup-x86_64" in release
    assert "capsem-linux-rust-target" not in release
    assert "docker ps -aq" in release
    assert 'docker volume rm "$volume"' in release
    assert "docker volume rm -f" not in release
    for retained in (
        "capsem-linux-rust-rustup",
        "capsem-linux-rust-cargo-registry",
        "capsem-host-target-arm64",
        "capsem-host-target-x86_64",
        "capsem-install-target",
        "capsem-install-rustup",
    ):
        assert retained not in release


def test_linux_rust_target_is_released_before_asset_capacity_preflight() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    release = _just_recipe_block("_release-completed-linux-rust-target:")

    linux_rust = candidate.index("just test-linux-rust")
    release_call = candidate.index("just _release-completed-linux-rust-target")
    release_builder = candidate.index("docker image rm capsem-host-builder:latest")
    asset_gate = candidate.index("just test-assets")

    assert linux_rust < release_call < release_builder < asset_gate
    assert "capsem-linux-rust-target" in release
    assert "docker ps -aq" in release
    assert 'docker volume rm "$volume"' in release
    assert "docker volume rm -f" not in release


def test_install_boundary_releases_only_completed_package_targets() -> None:
    release = _just_recipe_block("_release-completed-package-rails:")
    install = _just_recipe_block("test-install:")

    assert "capsem-host-target-arm64" in release
    assert "capsem-host-target-x86_64" in release
    assert "docker ps -aq" in release
    assert 'docker volume rm "$volume"' in release
    assert "docker volume rm -f" not in release
    for retained in (
        "capsem-cargo-registry",
        "capsem-rustup",
        "capsem-install-target",
        "capsem-install-rustup",
    ):
        assert retained not in release

    cleanup_trap = install.index("trap cleanup EXIT")
    release_call = install.index("just _release-completed-package-rails")
    capacity = install.index('scripts/ensure-docker-space.sh" 16')
    assert cleanup_trap < release_call < capacity


def test_full_gate_releases_deferred_install_target_between_package_arches() -> None:
    candidate = _just_recipe_block("_test-candidate:")
    release = _just_recipe_block("_release-deferred-install-target:")

    arm_package = candidate.index("just cross-compile arm64")
    release_call = candidate.index("just _release-deferred-install-target")
    x86_package = candidate.index("just cross-compile x86_64")

    assert arm_package < release_call < x86_package
    assert "capsem-install-target" in release
    assert "docker ps -aq" in release
    assert 'docker volume rm "$volume"' in release
    assert "docker volume rm -f" not in release
    for retained in (
        "capsem-cargo-registry",
        "capsem-rustup",
        "capsem-host-target-arm64",
        "capsem-host-target-x86_64",
        "capsem-install-rustup",
    ):
        assert retained not in release


def test_full_gate_bounds_docker_storage_without_flushing_rebuild_caches() -> None:
    full_gate = _just_recipe_block("test:")
    bound = _just_recipe_block("_bound-docker-test-storage:")

    assert "_test-candidate: _bound-docker-test-storage " in full_gate
    assert full_gate.index("just test-install") < full_gate.index("just _bound-docker-test-storage")
    capacity = bound.index("scripts/ensure-docker-space.sh")
    release_host = bound.index("docker image rm capsem-host-builder:latest")
    release_install = bound.index("docker image rm capsem-install-test:latest")
    assert release_host < release_install < capacity
    assert "docker image rm -f" not in bound
    assert "docker volume rm" not in bound


def test_full_gate_releases_stage_final_images_without_flushing_hot_cache() -> None:
    candidate = _just_recipe_block("_test-candidate:")

    install_preflight = candidate.index("just _test-install-harness-preflight")
    release_install = candidate.index("docker image rm capsem-install-test:latest")
    linux_parity = candidate.index("just test-linux-rust")
    release_host_builder = candidate.index("docker image rm capsem-host-builder:latest")
    asset_gate = candidate.index("just test-assets")
    arm_package = candidate.index("just cross-compile arm64")
    x86_package = candidate.index("just cross-compile x86_64")
    install_tail = candidate.rindex("just test-install")

    assert install_preflight < release_install < arm_package
    assert linux_parity < release_host_builder < asset_gate
    assert arm_package < x86_package < install_tail
    assert "CAPSEM_KEEP_HOST_BUILDER=1" not in candidate
    assert "docker builder prune -af" not in candidate


def test_docker_gc_reclaims_old_created_debug_containers() -> None:
    cleanup = _just_recipe_block("_docker-gc:")

    assert "docker container prune -f --filter until=24h" in cleanup
    assert "--filter status=exited" not in cleanup


def test_install_gate_releases_disposable_build_state_before_pytest() -> None:
    block = _just_recipe_block("test-install")

    package_install = block.index("Installing .deb via dpkg")
    trim_build_state = block.index(
        "rm -rf /cargo-target/debug/incremental /cargo-target/debug/deps "
        "/cargo-target/debug/build /cargo-target/debug/.fingerprint "
        "/cargo-target/debug/examples",
        package_install,
    )
    final_capacity = block.index('scripts/ensure-docker-space.sh" 12', package_install)
    pytest_launch = block.index("Running install e2e tests")

    assert package_install < trim_build_state < final_capacity < pytest_launch
    cleanup = block[trim_build_state:final_capacity]
    assert "/cargo-target/debug/bundle" not in cleanup
    assert "rm -rf /cargo-target/debug/*" not in cleanup


def test_cross_compile_does_not_bypass_apt_date_validation() -> None:
    swap_script = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text()

    assert "Acquire::Check-Valid-Until=false" not in swap_script
    assert "Acquire::Check-Date=false" not in swap_script


def test_standalone_install_gate_preflights_privileged_helper() -> None:
    block = _just_recipe_block("test-install")

    release_install_target = block.index("just _release-deferred-install-target")
    capacity = block.index(
        "CAPSEM_DOCKER_CACHE_KEEP_GB=2 CAPSEM_DOCKER_LINKED_KEEP_GB=2 "
        '"$ROOT/scripts/ensure-docker-space.sh" 22'
    )
    preflight = block.index("just _test-install-harness-preflight")
    start_container = block.index('echo "Starting systemd container..."')

    assert release_install_target < capacity < preflight < start_container


def test_binary_release_sbom_jobs_install_zstd_for_deb_payloads() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()

    for job_name in ("create-release", "assemble-release-channel"):
        job = _workflow_job_blocks(workflow)[job_name]
        assert "Install host SBOM archive deps" in job
        assert "zstd" in job
        assert job.index("Install host SBOM archive deps") < job.index(
            "Generate packaged host SBOM"
        )


def test_local_release_glowup_channel_build_uses_local_release_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    build_channel = script.split("def build_channel(", maxsplit=1)[1].split(
        "\ndef copy_artifact_tree", maxsplit=1
    )[0]

    assert "CAPSEM_RELEASE_URL" in build_channel
    assert 'f"{base_url}/releases/download/{channel}"' in build_channel
    assert "--asset-source-base" in build_channel
    assert 'f"{base_url}/assets/releases/{{asset_version}}"' in build_channel
    assert "stage_vm_asset_blobs(stable_manifest, args.assets_dir, dist)" in script


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


def test_local_release_glowup_rejects_root_relative_runtime_asset_urls() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()
    checker = script.split("def check_generated_release(", maxsplit=1)[1].split(
        "\ndef release_asset_urls", maxsplit=1
    )[0]

    assert 'elif url.startswith("/")' not in checker
    assert "generated VM asset URL is not absolute" in checker


def test_local_release_glowup_validates_vm_asset_blobs_are_served() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "release_asset_urls" in script
    assert "release is missing VM asset blob" in script
    assert '"/assets/releases/"' in script


def test_local_release_glowup_preflights_stable_and_nightly_manifests() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert (
        'check_generated_release(base_url, stable_manifest_url, stable_deb, dist, "stable")'
        in script
    )
    assert (
        'check_generated_release(base_url, nightly_manifest_url, nightly_deb, dist, "nightly")'
        in script
    )


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
            """{
  "packages": [
    {
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "%s/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }
  ],
  "profiles": {
    "co-work": {
      "architectures": [
        {
          "images": [
            {"url": "%s/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}
          ],
          "evidence": [
            {"url": "%s/assets/releases/2026.0709.13/obom.cdx.json"}
          ]
        }
      ]
    }
  }
}
"""
            % (base_url, base_url, base_url),
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


def test_local_release_glowup_generated_release_checker_accepts_local_assets(
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
        for relative in (
            "assets/releases/2026.0709.13/x86_64-rootfs.erofs",
            "assets/releases/2026.0709.13/obom.cdx.json",
        ):
            target = dist / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b"fixture")
        manifest_path = dist / "assets" / "nightly" / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            """{
  "packages": [
    {
      "name": "Capsem_1.5.1_amd64.deb",
      "url": "%s/releases/download/v1.5.1/Capsem_1.5.1_amd64.deb"
    }
  ],
  "profiles": {
    "co-work": {
      "architectures": [
        {
          "images": [
            {"url": "%s/assets/releases/2026.0709.13/x86_64-rootfs.erofs"}
          ],
          "evidence": [
            {"url": "%s/assets/releases/2026.0709.13/obom.cdx.json"}
          ]
        }
      ]
    }
  }
}
"""
            % (base_url, base_url, base_url),
            encoding="utf-8",
        )

        glowup.check_generated_release(
            base_url,
            f"{base_url}/assets/nightly/manifest.json",
            deb,
            dist,
            "nightly",
        )


def test_local_release_glowup_installed_path_asserts_channel_round_trip_and_provenance(
    monkeypatch,
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
    )

    assert len(calls) == 1
    script = calls[0][-1]
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
    assert "update --yes" not in script
    assert '"package_version": "1.5.101"' not in script
    assert "check_service_installed" in script
    assert '"$HOME/.capsem/bin/capsem" status' in script
    assert 'grep -F "Installed: true"' in script
    assert 'grep -F "Running:   true"' in script
    assert 'grep -F "Service:   ok"' in script
    assert 'grep -F "Gateway:   ok"' in script
    assert "scripts/verify-installed-release.py" in script
    assert (
        "verify_installed_release http://127.0.0.1:1234/assets/stable/manifest.json stable"
        in script
    )
    assert (
        "verify_installed_release http://127.0.0.1:1234/assets/nightly/manifest.json nightly"
        in script
    )
    assert "verify_installed_release http://127.0.0.1:1234/corp/manifest.json corp" in script
    assert "service status" not in script
    assert "check_binary_versions 1.5.100" in script
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
    assert "update --assets --channel nightly" in script
    assert "update --assets --channel stable" in script
    assert "check_origin_channel corp" in script


def test_local_release_glowup_forbids_metadata_only_binary_cohorts() -> None:
    script = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text()

    assert "rewrite_deb_version" not in script
    assert "next_patch_version" not in script
    assert "without recompiling a second binary cohort" not in script


def test_local_native_install_uses_public_manifest_contract_by_default() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    install = justfile.split(
        "install: _pnpm-install _stamp-version _check-assets _pack-initrd _materialize-config",
        maxsplit=1,
    )[1].split("\n# Run install e2e tests", maxsplit=1)[0]

    assert (
        'MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"'
        in install
    )
    assert '--manifest "$MANIFEST_URL"' in install
    assert '--manifest "file://$PWD/' not in install
    assert 'MANIFEST_CHANNEL="${CAPSEM_INSTALL_CHANNEL:-stable}"' in install
    assert "scripts/verify-installed-release.py" in install
    assert '--manifest-url "$MANIFEST_URL"' in install
    assert '--channel "$MANIFEST_CHANNEL"' in install
    assert '--package-version "$VERSION"' in install
    assert "scripts/prove-installed-shell.py" in install
    assert "CAPSEM_LOCAL_NATIVE_INSTALL_SHELL_OK" in install
    assert install.index("scripts/verify-installed-release.py") < install.index(
        "scripts/prove-installed-shell.py"
    )


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
    qualification = (
        PROJECT_ROOT / ".github" / "workflows" / "release-qualification.yaml"
    ).read_text()

    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "Complete canonical release gate (just test)" in qualification
    assert "run: just test" not in workflow
    assert "scripts/check-release-qualification.py" in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
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


def test_asset_build_recipes_skip_kvm_only_for_build_prereq_doctor() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    doctor_linux = (PROJECT_ROOT / "scripts" / "doctor-linux.sh").read_text()

    assert "CAPSEM_SKIP_KVM_CHECK" in doctor_linux
    assert 'skip "/dev/kvm (CAPSEM_SKIP_KVM_CHECK set)"' in doctor_linux

    for recipe in ("build-kernel", "build-rootfs", "build-assets"):
        block = justfile.split(f"\n{recipe} ", 1)[1].split("\n# ", 1)[0]
        assert "CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor" in block

    smoke_block = justfile.split("\nsmoke", 1)[1].split("\n# ", 1)[0]
    assert "CAPSEM_SKIP_KVM_CHECK" not in smoke_block


def test_only_systemd_package_proof_receives_kvm_devices() -> None:
    cross_compile = _just_recipe_block("cross-compile")
    proof = _just_recipe_block("_prove-linux-deb")

    assert "DOCKER_KVM_ARGS" not in cross_compile
    assert "--device /dev/kvm" not in cross_compile
    assert "--device /dev/vhost-vsock" not in cross_compile
    assert "DEVICE_ARGS=(" in proof
    assert "--device /dev/kvm" in proof
    assert "--device /dev/vhost-vsock" in proof
    assert '"${DEVICE_ARGS[@]}"' in proof


def test_cross_compile_clock_sync_uses_bounded_colima_command() -> None:
    cross_compile = _just_recipe_block("cross-compile")

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

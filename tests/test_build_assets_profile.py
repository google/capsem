"""Profile-owned asset build rail tests."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line == name or line.startswith(f"{name} "))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def test_build_assets_requires_profile_and_uses_capsem_admin() -> None:
    block = _recipe_block("build-assets")

    assert 'if [[ -z "$PROFILE_ARG" ]]' in block
    assert "profile id required" in block
    assert block.index('if [[ -z "$PROFILE_ARG" ]]') < block.index("just _install-tools")
    assert "cargo run -p capsem-admin -- image build" in block
    assert '--profile "config/profiles/${PROFILE_ARG}/profile.toml"' in block
    assert "${PROFILE_ARG#profile=}" not in block
    assert "uv run capsem-builder build guest/" not in block


def test_asset_build_primitives_accept_an_isolated_output_root() -> None:
    for recipe in ("build-kernel", "build-rootfs", "build-assets"):
        block = _recipe_block(recipe)
        assert "output=assets_dir" in block
        assert 'OUTPUT_ARG="{{output}}"' in block
        assert '--output "$OUTPUT_ARG"' in block


def test_just_test_owns_the_complete_asset_build_and_boot_gate() -> None:
    test = _recipe_block("test:")
    asset_gate = _recipe_block("test-assets:")

    assert "just test-assets" in test
    assert "profile_paths=(config/profiles/*/profile.toml)" in asset_gate
    assert 'for profile_path in "${profile_paths[@]}"; do' in asset_gate
    assert "for arch in arm64 x86_64; do" in asset_gate
    assert 'just build-kernel "$arch" "$profile" "$lane_assets"' in asset_gate
    assert 'just build-rootfs "$arch" "$profile" "$lane_assets"' in asset_gate
    assert 'ln -sfn "$HOST_ARCH" "$profile_assets/current"' in asset_gate
    assert 'readlink "$profile_assets/current"' in asset_gate
    assert 'cp target/config/settings/settings.toml "$profile_home/settings.toml"' not in asset_gate
    assert "mktemp -d /tmp/capsem-a.XXXXXX" in asset_gate
    assert 'profile_run="$profile_root/run"' not in asset_gate
    assert 'cp -R "$profile_run"/. "$profile_root/run-failure"/' in asset_gate
    assert 'python3 scripts/create_hash_assets.py "$profile_assets"' in asset_gate
    assert asset_gate.index("scripts/create_hash_assets.py") < asset_gate.index(
        "cargo run -p capsem-admin -- manifest check"
    )
    assert "cargo run -p capsem-admin -- manifest check" in asset_gate
    assert "scripts/prove-installed-shell.py" in asset_gate
    assert 'CAPSEM_ASSETS_DIR="$profile_assets"' in asset_gate
    assert 'CAPSEM_PROFILES_DIR="$profile_config/profiles"' in asset_gate


def test_asset_gate_runs_architecture_lanes_in_parallel_before_boot_proofs() -> None:
    asset_gate = _recipe_block("test-assets:")

    assert asset_gate.startswith("test-assets: _bootstrap ")
    assert "build_arch_lane()" in asset_gate
    assert 'build_arch_lane arm64 &' in asset_gate
    assert 'build_arch_lane x86_64 &' in asset_gate
    assert 'wait "$ARM64_BUILD_PID"' in asset_gate
    assert 'wait "$X86_64_BUILD_PID"' in asset_gate
    assert 'lane_assets="$profile_root/build-$arch"' in asset_gate
    assert 'cargo run -p capsem-admin -- manifest generate "$profile_assets"' in asset_gate
    assert asset_gate.index('wait "$ARM64_BUILD_PID"') < asset_gate.index(
        'cargo run -p capsem-admin -- manifest generate "$profile_assets"'
    )
    assert asset_gate.index('wait "$X86_64_BUILD_PID"') < asset_gate.index(
        'scripts/prove-installed-shell.py'
    )


def test_asset_gate_reaps_gateway_and_service_between_profile_proofs() -> None:
    asset_gate = _recipe_block("test-assets:")

    assert "stop_gate_pidfile" in asset_gate
    assert "gate_pid_running" in asset_gate
    assert "ps -o stat=" in asset_gate
    assert '"$state" != Z*' in asset_gate
    assert 'stop_gate_pidfile "$run_dir/gateway.pid"' in asset_gate
    assert 'stop_gate_pidfile "$run_dir/service.pid"' in asset_gate
    assert asset_gate.index('stop_gate_pidfile "$run_dir/gateway.pid"') < asset_gate.index(
        'stop_gate_pidfile "$run_dir/service.pid"'
    )
    assert asset_gate.index('stop_gate_pidfile "$run_dir/service.pid"') < asset_gate.index(
        'rm -rf "$profile_run"'
    )


def test_asset_ci_uses_primitives_owned_by_just_test() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    asset_gate = _recipe_block("test-assets:")

    assert 'just build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert 'just build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert "just build-kernel" in asset_gate
    assert "just build-rootfs" in asset_gate


def test_check_assets_recovers_by_iterating_checked_in_profiles() -> None:
    block = _recipe_block("_check-assets:")

    assert "for profile in config/profiles/*/profile.toml; do" in block
    assert 'just build-assets "$(basename "$(dirname "$profile")")" "$arch"' in block
    assert "just build-assets code" not in block


def test_runtime_recipes_materialize_generated_config_before_service() -> None:
    for recipe in ["shell:", "run-service:", "smoke:", "bench:", "install:"]:
        block = _recipe_block(recipe)
        assert "_pack-initrd" in block
        assert "_materialize-config" in block
        assert block.index("_pack-initrd") < block.index("_materialize-config")


def test_materialize_config_uses_admin_profile_command() -> None:
    block = _recipe_block("_materialize-config:")

    assert 'bash "$ROOT/scripts/materialize-config.sh"' in block

    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()
    assert "cargo run -p capsem-admin -- profile materialize" in script
    assert "normalize_arch()" in script
    assert 'case "$arch" in' in script
    assert "arm64|aarch64)" in script
    assert "--config-root" in script
    assert "--manifest" in script
    assert "--output-root" in script
    assert "target/config" in script


def test_materialize_config_falls_back_to_sole_manifest_arch_for_ci_runner() -> None:
    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'manifest["assets"]["current"]' in script
    assert 'manifest["assets"]["releases"][current]["arches"]' in script
    assert 'if [ "$arch_source" = "host" ] && [ "$manifest_arch_count" = "1" ]; then' in script
    assert "using sole manifest arch" in script
    assert 'arch_source="CAPSEM_ARCH"' in script
    assert "materialize arch $arch from $arch_source is not present" in script


def test_materialize_config_materializes_entire_checked_in_profile_catalog() -> None:
    block = _recipe_block("_materialize-config:")
    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'rm -rf "$ROOT/target/config"' in script
    assert 'profile_paths=("$ROOT"/config/profiles/*/profile.toml)' in script
    assert 'for profile_path in "${profile_paths[@]}"; do' in script
    assert '--profile "$profile_path"' in script
    assert '--profile "$ROOT/config/profiles/code/profile.toml"' not in script
    assert "scripts/materialize-config.sh" in block


def test_ensure_service_uses_generated_profiles() -> None:
    block = _recipe_block("_ensure-service:")

    assert 'GENERATED_PROFILES="$ROOT/target/config/profiles"' in block
    assert 'CAPSEM_PROFILES_DIR="$GENERATED_PROFILES"' in block
    assert "generated profiles missing" in block


def test_isolated_test_recipes_trap_test_home_service_cleanup() -> None:
    for recipe in ["test:", "smoke:"]:
        block = _recipe_block(recipe)
        assert "cleanup_test_capsem_home_service()" in block
        assert "trap cleanup_test_capsem_home_service EXIT" in block
        assert 'PIDFILE="$CAPSEM_RUN_DIR/service.pid"' in block
        assert 'kill "$OLD_PID"' in block
        assert "pkill -f" not in block


def test_release_workflow_uses_same_config_materializer() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert workflow.count("cargo run -p capsem-admin -- profile materialize") >= 2
    assert "--output-root target/config" in workflow
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow


def test_asset_workflow_publishes_obom_not_debug_build_ledger() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert "npm install -g @cyclonedx/cdxgen@latest" in workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in workflow
    upload_step = workflow.split("- name: Publish immutable GitHub asset release", maxsplit=1)[
        1
    ].split("\n      - name: Attest VM asset provenance", maxsplit=1)[0]
    attest_step = workflow.split("- name: Attest VM asset provenance", maxsplit=1)[1].split(
        "\n      - uses: actions/upload-artifact@v7", maxsplit=1
    )[0]
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        assert logical_name in upload_step
        assert logical_name in attest_step
    assert "vm-build-ledger-" not in workflow
    assert "build-ledger.log" not in upload_step
    assert "build-ledger.log" not in attest_step
    assert "B3SUMS" not in upload_step
    assert "B3SUMS" not in attest_step
    assert "obom.cdx.json" not in release
    assert "Skipping debug-only $arch/$base from release upload" not in release

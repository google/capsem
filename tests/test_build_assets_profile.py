"""Profile-owned asset build rail tests."""

from __future__ import annotations

from pathlib import Path
import re


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
    block = "\n".join(lines[start:end])
    if name == "test:":
        block = f"{block}\n{_recipe_block('_test-candidate:')}"
    return block


def test_build_assets_requires_profile_and_uses_capsem_admin() -> None:
    block = _recipe_block("_build-assets")

    assert 'if [[ -z "$PROFILE_ARG" ]]' in block
    assert "internal _build-assets requires" in block
    assert block.index('if [[ -z "$PROFILE_ARG" ]]') < block.index("just _install-tools")
    assert "cargo run -p capsem-admin -- image build" in block
    assert '--profile "config/profiles/${PROFILE_ARG}/profile.toml"' in block
    assert "${PROFILE_ARG#profile=}" not in block
    assert "uv run capsem-builder build guest/" not in block


def test_asset_build_primitives_accept_an_isolated_output_root() -> None:
    for recipe in ("_build-kernel", "_build-rootfs", "_build-assets"):
        block = _recipe_block(recipe)
        assert "output=assets_dir" in block
        assert 'OUTPUT_ARG="{{output}}"' in block
        if recipe == "_build-assets":
            assert '--output "$OUTPUT_ARG"' in block
        else:
            assert '"$OUTPUT_ARG"' in block
    primitive = _recipe_block("_build-image-template")
    assert " output template:" in primitive.splitlines()[0]
    assert 'OUTPUT_ARG="{{output}}"' in primitive
    assert '--output "$OUTPUT_ARG"' in primitive


def test_just_test_owns_the_complete_asset_build_and_boot_gate() -> None:
    test = _recipe_block("_test-candidate-run:")
    asset_gate = _recipe_block("_gate-assets:")

    assert "just _gate-assets" in test
    assert "profile_paths=(config/profiles/*/profile.toml)" in asset_gate
    assert 'for profile_path in "${profile_paths[@]}"; do' in asset_gate
    assert "for arch in arm64 x86_64; do" in asset_gate
    assert 'just _build-image-template "$arch" "$profile" "$lane_assets" kernel' in asset_gate
    assert 'just _build-image-template "$arch" "$profile" "$lane_assets" rootfs' in asset_gate
    assert 'ln -sfn "$HOST_ARCH" "$profile_assets/current"' in asset_gate
    assert 'readlink "$profile_assets/current"' in asset_gate
    assert 'cp target/config/settings/settings.toml "$profile_home/settings.toml"' not in asset_gate
    assert "mktemp -d /tmp/capsem-a.XXXXXX" in asset_gate
    assert 'profile_run="$profile_root/run"' not in asset_gate
    # A failed proof leaves its session behind, so the evidence copy must walk
    # the run dir for the host-side logs that name the boot failure -- and only
    # those. A blanket `cp -R` also takes the guest's workspace, duplicated
    # once per auto-snapshot generation, into target/.
    assert 'cp -R "$profile_run"/. "$profile_root/run-failure"/' not in asset_gate
    assert "-name '*.log'" in asset_gate
    assert r"\( -name guest -o -name auto_snapshots \) -prune" in asset_gate
    assert 'cp "$evidence" "$profile_root/run-failure/$evidence"' in asset_gate
    assert 'python3 scripts/create_hash_assets.py "$profile_assets"' in asset_gate
    assert asset_gate.index("scripts/create_hash_assets.py") < asset_gate.index(
        "cargo run -p capsem-admin -- manifest check"
    )
    assert "cargo run -p capsem-admin -- manifest check" in asset_gate
    assert "scripts/prove-installed-shell.py" in asset_gate
    assert '--profile "$profile"' in asset_gate
    assert 'CAPSEM_ASSETS_DIR="$profile_assets"' in asset_gate
    assert 'CAPSEM_PROFILES_DIR="$profile_config/profiles"' in asset_gate


def test_asset_gate_runs_architecture_lanes_in_parallel_before_boot_proofs() -> None:
    asset_gate = _recipe_block("_gate-assets:")

    assert asset_gate.startswith("_gate-assets: _bootstrap ")
    assert "build_arch_lane()" in asset_gate
    assert 'build_arch_lane arm64 2>&1 | tee "$ARM64_BUILD_LOG"' in asset_gate
    assert 'build_arch_lane x86_64 2>&1 | tee "$X86_64_BUILD_LOG"' in asset_gate
    assert 'wait "$ARM64_BUILD_PID"' in asset_gate
    assert 'wait "$X86_64_BUILD_PID"' in asset_gate
    assert 'lane_assets="$profile_root/build-$arch"' in asset_gate
    assert 'cargo run -p capsem-admin -- manifest generate "$profile_assets"' in asset_gate
    assert asset_gate.index('wait "$ARM64_BUILD_PID"') < asset_gate.index(
        'cargo run -p capsem-admin -- manifest generate "$profile_assets"'
    )
    assert asset_gate.index('wait "$X86_64_BUILD_PID"') < asset_gate.index(
        "scripts/prove-installed-shell.py"
    )


def test_asset_gate_reaps_gateway_and_service_between_profile_proofs() -> None:
    asset_gate = _recipe_block("_gate-assets:")

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
    asset_gate = _recipe_block("_gate-assets:")

    assert 'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert 'just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"' in workflow
    assert "just _build-image-template" in asset_gate


def test_asset_ci_installs_pinned_pnpm_before_running_build_primitives() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    build_assets = workflow.split("\n  build-assets:\n", maxsplit=1)[1].split(
        "\n  reuse-assets:", maxsplit=1
    )[0]
    pnpm_setup = (
        "- uses: pnpm/action-setup@fc06bc1257f339d1d5d8b3a19a8cae5388b55320\n"
        "        with:\n"
        "          version: 10"
    )

    assert pnpm_setup in build_assets
    assert build_assets.index(pnpm_setup) < build_assets.index(
        "actions/setup-node@a0853c24544627f65ddf259abe73b1d18a591444"
    )
    assert build_assets.index(pnpm_setup) < build_assets.index("just _build-kernel")


def test_asset_matrix_preflights_once_and_reuses_the_public_build_primitive() -> None:
    asset_gate = _recipe_block("_gate-assets:")
    kernel = _recipe_block("_build-kernel")
    rootfs = _recipe_block("_build-rootfs")
    primitive = _recipe_block("_build-image-template")

    assert 'just _build-image-template "{{arch}}" "$PROFILE_ARG" "$OUTPUT_ARG" kernel' in kernel
    assert 'just _build-image-template "{{arch}}" "$PROFILE_ARG" "$OUTPUT_ARG" rootfs' in rootfs
    assert "cargo run -p capsem-admin -- image build" in primitive
    assert "just _install-tools" not in primitive
    assert "just doctor" not in primitive
    assert asset_gate.startswith("_gate-assets: _bootstrap ")
    assert "CAPSEM_SKIP_ASSET_CHECK=1 CAPSEM_SKIP_KVM_CHECK=1 just doctor" not in asset_gate


def test_check_assets_recovers_by_iterating_checked_in_profiles() -> None:
    block = _recipe_block("_check-assets:")

    assert "for profile in config/profiles/*/profile.toml; do" in block
    assert 'just _build-assets "$(basename "$(dirname "$profile")")" "$arch"' in block
    assert "just _build-assets code" not in block


def test_in_container_commands_write_only_where_the_container_user_owns() -> None:
    """/src is a bind mount of the host checkout. On Linux the host UID does not
    own it, so anything `docker exec -u capsem` writes outside an explicitly
    chowned path fails with EACCES -- and macOS maps the mount cleanly, so only
    CI ever sees it. Four separate release-gate failures came from this one
    shape: the builder's git, the staging rm, pytest's cache, and the
    unmaterialized profile catalog."""
    gate = _recipe_block("_gate-install:")

    # Removing target/install-test-* needs write permission on their parent.
    assert "chown capsem:capsem /src/target" in gate

    for command in re.findall(r'docker exec[^\n]*-u capsem[^\n]*\n?[^\n]*', gate):
        if "pytest" not in command:
            continue
        assert "TMPDIR=/home/capsem" in command, (
            f"in-container pytest must keep temp files off /src: {command[:120]}"
        )
        assert "cache_dir=/home/capsem" in command, (
            "in-container pytest must keep its cache off /src; the default "
            f"rootdir cache write fails with EACCES on Linux: {command[:120]}"
        )


def test_runtime_recipes_materialize_generated_config_before_service() -> None:
    # `_prepared-runtime` owns this sequence for every runtime entry point, so
    # the ordering is asserted once where it lives rather than re-checked in
    # each caller. Four copies of it is how a fifth caller drops a step.
    prepared = _recipe_block("_prepared-runtime:")
    assert "_pack-initrd" in prepared
    assert "_materialize-config" in prepared
    assert prepared.index("_pack-initrd") < prepared.index("_materialize-config")

    for recipe in ["shell:", "run-service:", "smoke:"]:
        assert "_prepared-runtime" in _recipe_block(recipe)


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


def test_materialize_config_uses_release_manifest_profile_membership() -> None:
    block = _recipe_block("_materialize-config:")
    script = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'rm -rf "$OUTPUT_ROOT"' in script
    assert 'rm -rf "$ROOT/target/config"' not in script
    assert 'manifest_schema="release"' in script
    assert 'profile_ids="$(' in script
    assert 'profile_path="$CONFIG_ROOT/profiles/$profile_id/profile.toml"' in script
    assert "selected release profile source is missing" in script
    assert 'profile_paths=("$CONFIG_ROOT"/profiles/*/profile.toml)' in script
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
    for recipe in ["_test-candidate-run:", "smoke:"]:
        block = _recipe_block(recipe)
        assert "cleanup_test_capsem_home_service()" in block
        assert "trap cleanup_test_capsem_home_service EXIT" in block
        assert 'PIDFILE="$CAPSEM_RUN_DIR/service.pid"' in block
        assert 'kill "$OLD_PID"' in block
        assert "pkill -f" not in block


def test_release_workflow_uses_same_config_materializer() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert workflow.count("bash scripts/materialize-config.sh") >= 2
    assert workflow.count('CAPSEM_ASSET_MANIFEST="$PREACTIVATION_MANIFEST"') >= 2
    assert 'CAPSEM_ARCH="${{ matrix.arch }}"' in workflow


def test_asset_workflow_publishes_obom_not_debug_build_ledger() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    stager = (PROJECT_ROOT / "scripts/stage-profile-publication.py").read_text()

    assert "npm install -g @cyclonedx/cdxgen@12.7.0" in workflow
    assert "@cyclonedx/cdxgen@latest" not in workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in workflow
    stage_step = workflow.split(
        "- name: Stage and verify immutable profile publication once", maxsplit=1
    )[1].split(
        "\n      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        maxsplit=1,
    )[0]
    upload_step = workflow.split("- name: Publish immutable GitHub profile release", maxsplit=1)[
        1
    ].split("\n      - name: Attest VM asset provenance", maxsplit=1)[0]
    attest_step = workflow.split("- name: Attest VM asset provenance", maxsplit=1)[1].split(
        "\n      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        maxsplit=1,
    )[0]
    assert "scripts/stage-profile-publication.py" in stage_step
    assert "scripts/verify-profile-publication.py" in stage_step
    assert "scripts/stage-profile-publication.py" not in upload_step
    assert "scripts/verify-profile-publication.py" in upload_step
    assert "scripts/publish-immutable-release-assets.sh" in upload_step
    assert "gh release create" not in upload_step
    assert "gh release upload" not in upload_step
    assert 'files=("$RELEASE_DIR"/*)' in upload_step
    assert 'for section in ("config", "images", "evidence")' in stager
    assert "subject-path: target/asset-release/profile-*/*" in attest_step
    assert "vm-build-ledger-" not in workflow
    assert "build-ledger.log" not in upload_step
    assert "build-ledger.log" not in attest_step
    assert "B3SUMS" not in upload_step
    assert "B3SUMS" not in attest_step
    assert "obom.cdx.json" not in release
    assert "Skipping debug-only $arch/$base from release upload" not in release

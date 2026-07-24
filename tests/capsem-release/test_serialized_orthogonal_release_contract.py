"""Contracts for serialized binary/profile release ownership.

These tests intentionally inspect only public commands and workflow orchestration.
Artifact correctness remains covered by the executable lane and glow-up suites.
"""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
CHANNEL_GROUP = "group: capsem-release-${{ inputs.channel }}"


def _read(path: str | Path) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def _workflow(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def _recipe_block(justfile: str, recipe: str) -> str:
    marker = f"\n{recipe} "
    start = justfile.index(marker)
    rest = justfile[start + 1 :]
    next_recipe = rest.find("\n\n")
    return rest if next_recipe < 0 else rest[:next_recipe]


def test_release_commands_are_two_single_purpose_recipes() -> None:
    justfile = "\n" + _read("justfile")

    binary = _recipe_block(justfile, "release-binaries")
    profile = _recipe_block(justfile, "release-profile")

    assert "scripts/release-binaries.py" in binary
    assert "capsem-admin" not in binary
    assert "_build-kernel" not in binary
    assert "_build-rootfs" not in binary

    assert "capsem-admin -- release" in profile
    assert "scripts/release-binaries.py" not in profile
    assert "_cross-compile" not in profile
    assert "build-pkg" not in profile

    retired_commands = (
        "release",
        "prepare-release",
        "qualify-" + "release",
        "cut-" + "release",
    )
    for retired in retired_commands:
        assert f"\n{retired}:" not in justfile
        assert f"\n{retired} " not in justfile


def test_binary_and_profile_workflows_share_channel_transaction_lock() -> None:
    for name in ("release.yaml", "release-assets.yaml"):
        workflow = _workflow(name)
        assert CHANNEL_GROUP in workflow
        assert "cancel-in-progress: false" in workflow
        assert workflow.index("concurrency:") < workflow.index("jobs:")
        group_line = next(
            line.strip() for line in workflow.splitlines() if line.strip().startswith("group:")
        )
        assert group_line == CHANNEL_GROUP
        assert "github.sha" not in group_line
        assert "inputs.profile" not in group_line
        assert "inputs.tag" not in group_line


def test_binary_lane_pulls_profiles_and_never_builds_them() -> None:
    workflow = _workflow("release.yaml")

    assert "Fetch latest selected channel source manifest" in workflow
    assert "binary-channel-source" in workflow
    assert "Fetch selected channel manifest and profiles" in workflow
    assert 'file://$PWD/target/channel-source/manifest.json' in workflow
    assert "just _test-artifacts" in workflow
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow
    assert "just _test-release-contracts" in workflow
    assert "--config-root target/release-config" in workflow
    assert 'CAPSEM_CONFIG_ROOT="$PWD/target/release-config"' in workflow
    assert '--package-file "$package"' in workflow
    assert "cp target/release-package-root/usr/bin/capsem*" not in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/target/debug/capsem" in workflow

    for forbidden in (
        "just _build-kernel",
        "just _build-rootfs",
        "capsem-admin -- image build",
    ):
        assert forbidden not in workflow


def test_profile_lane_pulls_binary_and_never_builds_packages() -> None:
    workflow = _workflow("release-assets.yaml")

    assert "Validate selected channel profile through capsem-admin" in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "Fetch selected channel binary packages" in workflow
    assert 'file://$PWD/target/channel-source/manifest.json' in workflow
    assert "capsem-admin -- release" in workflow
    assert "--publication-base" in workflow
    assert 'channel-source-$CHANNEL.json' in workflow
    assert "steps.profile-delta.outputs.changed == 'true'" in workflow
    assert "check-profile-release-delta.py" in workflow
    assert "check-asset-release-delta.py" not in workflow
    assert "just _test-artifacts" in workflow
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow
    assert "just _test-release-contracts" in workflow
    assert "--input-dir target/release-inputs" in workflow
    assert "--binary-dir target/debug" in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/target/debug/capsem" in workflow

    for forbidden in (
        "just _cross-compile",
        "scripts/build-pkg.sh",
        "scripts/repack-deb.sh",
        "cargo tauri build",
    ):
        assert forbidden not in workflow


def test_production_deploy_has_no_unserialized_direct_entrypoint() -> None:
    deploy = _workflow("release-channel.yaml")
    assert "workflow_dispatch:" not in deploy
    assert "workflow_call:" in deploy
    assert "capsem-admin -- release" not in deploy
    assert "record-binary" not in deploy

    production_callers = []
    for path in WORKFLOWS.glob("*.yaml"):
        text = path.read_text(encoding="utf-8")
        if "uses: ./.github/workflows/release-channel.yaml" in text:
            production_callers.append((path.name, text))

    assert {name for name, _ in production_callers} >= {
        "release.yaml",
        "release-assets.yaml",
    }
    for name, workflow in production_callers:
        if name == "release-channel-staging.yaml":
            assert "deploy_branch: ${{ inputs.deploy_branch }}" in workflow
            assert "validate_complete_public_channels: false" in workflow
            continue
        assert CHANNEL_GROUP in workflow, f"{name} deploys production without the channel lock"


def test_retired_independent_release_authority_is_absent() -> None:
    retired_workflow = "release-" + "qualification.yaml"
    retired_checker = "check-release-" + "qualification.py"
    assert not (WORKFLOWS / retired_workflow).exists()
    assert not (ROOT / "scripts" / retired_checker).exists()
    assert not (
        ROOT / "tests" / "capsem-build-chain" / "test_release_qualification.py"
    ).exists()

    release = _workflow("release.yaml")
    assert retired_checker not in release
    assert "Verify exact commit passed remote " + "qualification" not in release


def test_runtime_preflight_is_reused_without_independent_sha_authority() -> None:
    preflight = _workflow("release-runtime-preflight.yaml")
    assert "workflow_call:" in preflight
    assert "workflow_dispatch:" not in preflight
    assert "inputs.sha" not in preflight
    assert "EXPECTED_SHA" not in preflight
    assert "materialize-config.sh" in preflight
    assert "profiles/co-work" not in preflight
    assert "profiles/code" not in preflight

    for name in ("release.yaml", "release-assets.yaml"):
        workflow = _workflow(name)
        assert "uses: ./.github/workflows/release-runtime-preflight.yaml" in workflow

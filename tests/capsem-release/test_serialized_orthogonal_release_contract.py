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


def _job_block(workflow: str, job: str) -> str:
    lines = workflow.splitlines()
    start = lines.index(f"  {job}:")
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = index
            break
    return "\n".join(lines[start:end])


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


def test_release_lanes_run_one_reusable_fast_gate_before_builders() -> None:
    reusable = _workflow("fast-gate.yaml")
    assert "workflow_call:" in reusable
    assert "run: just _test-static" in reusable

    binary = _workflow("release.yaml")
    assert "  fast-gate:\n    uses: ./.github/workflows/fast-gate.yaml" in binary
    assert "needs: [runtime-preflight, fast-gate]" in _job_block(binary, "preflight")
    assert "Run shared static module" not in _job_block(binary, "test-binary-pairing")

    profile = _workflow("release-assets.yaml")
    assert "  fast-gate:\n    uses: ./.github/workflows/fast-gate.yaml" in profile
    build_assets = _job_block(profile, "build-assets")
    assert "fast-gate" in build_assets.splitlines()[1]
    assert "Run shared static module" not in _job_block(profile, "test-profile-pairing")


def test_release_profile_downloads_share_one_manifest_addressed_cache_module() -> None:
    action = _read(".github/actions/fetch-release-inputs/action.yaml")

    assert "scripts/fetch-release-artifacts.py" in action
    assert '--manifest-url "${{ inputs.manifest-url }}"' in action
    assert "--cache-dir target/release-input-cache" in action
    assert "--prune-cache" not in action
    assert "actions/cache/restore@" in action
    assert "actions/cache/save@" in action
    assert "steps.fetch.outputs.cache-misses != '0'" in action
    cache_key = next(line for line in action.splitlines() if line.strip().startswith("key:"))
    assert "inputs.channel" not in cache_key
    assert "inputs.manifest-url" not in cache_key

    assert "./.github/actions/fetch-release-inputs" in _workflow("release.yaml")
    assert "./.github/actions/fetch-release-inputs" in _workflow("release-assets.yaml")


def test_binary_lane_pulls_profiles_and_never_builds_them() -> None:
    workflow = _workflow("release.yaml")

    assert "Fetch latest selected channel source manifest" in workflow
    assert "binary-channel-source" in workflow
    assert "Resolve exact candidate-after profiles" in workflow
    assert "file://$PWD/target/binary-channel/$RELEASE_CHANNEL/manifest.json" in workflow
    assert "just _test-artifacts" in workflow
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow
    assert "just _test-release-contracts" in workflow
    assert "--config-root target/release-config" in workflow
    assert "--shared-config-root config" in workflow
    assert 'CAPSEM_CONFIG_ROOT="$PWD/target/release-config"' in workflow
    assert '--package-file "$package"' in workflow
    assert 'scripts/install-deb-runtime-dependencies.py "$package"' in workflow
    assert "cp target/release-package-root/usr/bin/capsem*" not in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/target/debug/capsem" in workflow

    for forbidden in (
        "just _build-kernel",
        "just _build-rootfs",
        "capsem-admin -- image build",
    ):
        assert forbidden not in workflow


def test_profile_lane_installs_pulled_package_runtime_dependencies() -> None:
    workflow = _workflow("release-assets.yaml")
    pairing = _job_block(workflow, "test-profile-pairing")

    resolve_package = pairing.index("--print-package-path")
    install_dependencies = pairing.index(
        'scripts/install-deb-runtime-dependencies.py "$package"'
    )
    functional = pairing.index("run: just _test-functional")

    assert resolve_package < install_dependencies < functional
    assert "sudo dpkg -i" not in pairing
    assert "sudo apt-get install" not in pairing


def test_binary_candidate_manifest_is_authored_once_before_pairing() -> None:
    workflow = _workflow("release.yaml")
    author = _job_block(workflow, "author-binary-candidate")
    pairing = _job_block(workflow, "test-binary-pairing")
    create = _job_block(workflow, "create-release")
    assemble = _job_block(workflow, "assemble-release-channel")

    assert "needs: [build-app-macos, build-app-linux, resolve-channel-source]" in author
    assert author.count("assets channel record-binary") == 1
    assert "binary-channel-candidate" in author
    assert "manifest.before.json" in author
    assert "manifest.json" in author

    assert "author-binary-candidate" in pairing.splitlines()[1]
    assert "binary-channel-candidate" in pairing
    assert (
        "manifest-url: file://${{ github.workspace }}/target/binary-channel/"
        "${{ inputs.channel }}/manifest.json"
    ) in pairing
    assert "assets channel record-binary" not in pairing

    assert "test-binary-pairing" in create.splitlines()[1]
    assert "author-binary-candidate" in assemble.splitlines()[1]
    assert "binary-channel-candidate" in assemble
    assert "assets channel record-binary" not in assemble
    assert "generate-host-binary-sbom.py" not in assemble


def test_binary_pairing_uses_exact_public_before_and_candidate_after_cohorts() -> None:
    workflow = _workflow("release.yaml")
    resolve = _job_block(workflow, "resolve-channel-source")
    pairing = _job_block(workflow, "test-binary-pairing")

    assert "manifest-url: ${{ env.ASSET_MANIFEST_URL }}" in resolve
    assert "kind: packages" in resolve
    assert "kind: profiles" in resolve
    assert "architecture: x86_64" in resolve
    assert "binary-public-before-packages" in resolve
    assert "binary-public-before-profiles" in resolve

    assert "binary-public-before-packages" in pairing
    assert "binary-public-before-profiles" in pairing
    assert (
        "manifest-url: file://${{ github.workspace }}/target/binary-channel/"
        "${{ inputs.channel }}/manifest.json"
    ) in pairing
    assert "kind: profiles" in pairing
    assert "target/candidate-profile-inputs" in pairing
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION=auto",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS",
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS",
    ):
        assert variable in pairing


def test_profile_lane_pulls_binary_and_never_builds_packages() -> None:
    workflow = _workflow("release-assets.yaml")

    assert "Validate selected channel profile through capsem-admin" in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "--bootstrap-missing-first-party" in workflow
    assert '--profile "${{ inputs.profile }}"' in workflow
    assert "Fetch exact deployed public-before package" in workflow
    assert "Fetch exact deployed public-before profiles" in workflow
    assert "manifest-url: ${{ env.ASSET_MANIFEST_URL }}" in workflow
    assert workflow.count("bootstrap-manifest-url:") == 2
    assert "--bootstrap-manifest-url" not in workflow
    assert 'allow-empty-profiles: "true"' in workflow
    assert "capsem-admin -- release" in workflow
    assert "--publication-base" in workflow
    assert "channel-source-$CHANNEL.json" in workflow
    assert "steps.profile-delta.outputs.changed == 'true'" in workflow
    assert "check-profile-release-delta.py" in workflow
    assert "check-asset-release-delta.py" not in workflow
    assert "just _test-artifacts" in workflow
    assert "just _test-functional" in workflow
    assert "just _test-glowup" in workflow
    assert "--shared-config-root config" in workflow
    assert "just _test-release-contracts" in workflow
    assert "--input-dir target/profile-public-before/packages" in workflow
    assert "--binary-dir target/debug" in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/target/debug/capsem" in workflow

    for forbidden in (
        "just _cross-compile",
        "scripts/build-pkg.sh",
        "scripts/repack-deb.sh",
        "cargo tauri build",
    ):
        assert forbidden not in workflow


def test_profile_selection_creates_clean_runner_output_parent() -> None:
    resolve = _job_block(_workflow("release-assets.yaml"), "resolve-current-binary")

    create_parent = resolve.index("mkdir -p target")
    validate = resolve.index("cargo run -p capsem-admin -- validate")
    redirect = resolve.index("> target/profile-release-selection.json")

    assert create_parent < validate < redirect


def test_profile_pairing_reuses_one_staged_publication_and_exact_public_before() -> None:
    workflow = _workflow("release-assets.yaml")
    resolve = _job_block(workflow, "resolve-current-binary")
    author = _job_block(workflow, "author-profile-release")
    pairing = _job_block(workflow, "test-profile-pairing")
    publish = _job_block(workflow, "publish-profile-release")

    assert "manifest-url: ${{ env.ASSET_MANIFEST_URL }}" in resolve
    assert "kind: packages" in resolve
    assert "kind: profiles" in resolve
    assert "architecture: x86_64" in resolve
    assert "profile-public-before-packages" in resolve
    assert "profile-public-before-profiles" in resolve

    assert "stage-profile-publication.py" in author
    assert "verify-profile-publication.py" in author
    assert "name: authored-profile-publication" in author

    for artifact in (
        "profile-public-before-packages",
        "profile-public-before-profiles",
        "authored-profile-publication",
    ):
        assert artifact in pairing
    assert "--local-publication-base" in pairing
    assert "--local-publication-dir" in pairing
    assert "target/candidate-profile-inputs" in pairing
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION=profile_only",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS",
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS",
        "CAPSEM_RELEASE_PROFILE",
        "CAPSEM_RELEASE_CANDIDATE_PROFILE_PUBLICATION",
        "CAPSEM_RELEASE_PUBLICATION_BASE",
    ):
        assert variable in pairing

    assert "name: authored-profile-publication" in publish
    assert "stage-profile-publication.py" not in publish
    assert "verify-profile-publication.py" in publish


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
    assert not (ROOT / "tests" / "capsem-build-chain" / "test_release_qualification.py").exists()

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


def test_profile_runtime_preflight_bootstraps_only_from_manifest_catalog() -> None:
    preflight = _workflow("release-runtime-preflight.yaml")
    binary = _workflow("release.yaml")
    profile = _workflow("release-assets.yaml")

    assert "bootstrap_missing_first_party:" in preflight
    assert "scripts/select-runtime-preflight-manifest.py" in preflight
    assert "--bootstrap-missing-first-party" in preflight
    assert "steps.manifest.outputs.manifest-url" in preflight
    assert "ASSET_MANIFEST_URL" not in preflight

    assert "bootstrap_missing_first_party: true" in profile
    assert "bootstrap_missing_first_party: true" not in binary

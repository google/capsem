"""Contracts for staging a profile once and activating it with a later binary."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import yaml
from helpers.workflow_contract import workflow_job_source, workflow_step_source
from rust_sources import sibling_tests

from capsem.gate.versions import workspace_version

ROOT = Path(__file__).resolve().parents[2]
ADMIN = ROOT / "crates" / "capsem-admin" / "src" / "main.rs"
RELEASE_GRAPH = ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
PROFILE_WORKFLOW = ROOT / ".github" / "workflows" / "release-assets.yaml"
BINARY_WORKFLOW = ROOT / ".github" / "workflows" / "release.yaml"
PUBLICATION_IDENTITY = f"profile-stable-code-{workspace_version(ROOT)}"


def _function(source: str, name: str, next_name: str) -> str:
    return source.split(f"fn {name}", maxsplit=1)[1].split(f"fn {next_name}", maxsplit=1)[0]


def _job(workflow: str, name: str, _next_name: str) -> str:
    return workflow_job_source(workflow, name)


def _step(job: str, name: str, next_name: str | None) -> str:
    del next_name
    return workflow_step_source(job, name)


def _release_plan(command: str, *arguments: str):
    """The plan a release command would run, without running any of it."""
    import argparse

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand
    from capsem.gate.proc import Runner
    from capsem.gate.sourcecommit import SourceCommit

    parsed = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        source_commit=SourceCommit("0" * 40),
        **dict(zip(("channel", "profile"), arguments, strict=False)),
    )
    return GateCommand.registry[command](Runner(ROOT), parsed).plan()


def test_staged_profile_declares_minimum_and_maximum_binary_bounds() -> None:
    graph = RELEASE_GRAPH.read_text(encoding="utf-8")
    profile = graph.split("pub struct ProfileDocument", maxsplit=1)[1].split(
        "pub struct SoftwareInventoryRow", maxsplit=1
    )[0]

    assert "pub min_capsem_version: Option<String>" in profile
    assert "pub max_capsem_version: Option<String>" in profile


def test_profile_then_binary_compatibility_checks_every_current_package() -> None:
    source = ADMIN.read_text(encoding="utf-8")
    compatibility = _function(
        source,
        "graph_profile_matches_current_binary",
        "validate_graph_profiles_match_current_binary",
    )

    assert '"min_capsem_version"' in compatibility
    assert '"max_capsem_version"' in compatibility
    assert "minimum > maximum" in compatibility
    assert '== Some("current")' in compatibility
    assert "versions.iter().all" in compatibility
    assert "versions.is_empty()" in compatibility


def test_staged_profile_cannot_activate_until_binary_bounds_match() -> None:
    source = ADMIN.read_text(encoding="utf-8")
    build = _function(
        source,
        "build_assets_channel_from_graph",
        "record_graph_binary_release_metadata",
    )
    record = _function(
        source,
        "record_graph_binary_release_metadata",
        "validate_binary_release_files",
    )

    assert "validate_graph_profiles_match_current_binary(&graph_manifest)?" in build
    assert "validate_graph_profiles_match_current_binary(&manifest)?" in record
    assert record.index('manifest["packages"]') < record.index(
        "validate_graph_profiles_match_current_binary"
    )
    assert record.index("validate_graph_profiles_match_current_binary") < record.index("fs::write")
    assert (
        "staged_profile_then_binary_activation_enforces_bounds_without_rebuilding_profile"
        in sibling_tests(ADMIN)
    )


def test_staged_profile_is_authored_once_before_pairing_tests_and_publication() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    author = _job(workflow, "author-profile-release", "test-profile-pairing")
    pairing = _job(workflow, "test-profile-pairing", "publish-profile-release")
    publish = _job(workflow, "publish-profile-release", "deploy-channel")

    assert "needs: [build-assets, reuse-assets, resolve-current-binary]" in author
    assert "source_changed:" in author
    assert "activation_needed:" in author
    assert "release_needed:" in author
    assert "product_compatible:" in author
    assert "functional_ready:" in author
    assert "activation_ready:" in author
    assert "publication_identity:" in author
    assert workflow.count("cargo run -p capsem-admin -- release") == 1
    assert "cargo run -p capsem-admin -- release" in author
    assert "cargo run -p capsem-admin -- release" not in pairing
    assert "cargo run -p capsem-admin -- release" not in publish
    assert "gh release create" not in author
    assert "scripts/publish-immutable-release-assets.sh" in publish
    assert "gh release create" not in publish
    assert "name: authored-profile-channel-source" in author
    assert "name: authored-profile-candidate" in author
    assert "name: authored-profile-publication" in author
    assert "name: authored-profile-publication" in pairing
    assert "name: authored-profile-publication" in publish
    assert "stage-profile-publication.py" in author
    assert "stage-profile-publication.py" not in publish
    assert "needs: [author-profile-release, resolve-current-binary]" in pairing
    assert "if: ${{ always()" in pairing
    assert "needs.author-profile-release.result == 'success'" in pairing
    assert "needs.resolve-current-binary.result == 'success'" in pairing
    assert "needs.author-profile-release.outputs.release_needed == 'true'" in pairing
    assert "needs: [author-profile-release, test-profile-pairing]" in publish
    assert "needs.test-profile-pairing.result == 'success'" in publish


def test_profile_retry_reuses_one_prior_artifact_cohort_without_rebuilding() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    resolver = _job(workflow, "resolve-profile-assets", "build-assets")
    build = _job(workflow, "build-assets", "reuse-assets")
    reuse = _job(workflow, "reuse-assets", "test-profile-pairing")
    author = _job(workflow, "author-profile-release", "publish-profile-release")
    publish = _job(workflow, "publish-profile-release", "deploy-channel")

    assert "resolve-reusable-profile-assets.py" in resolver
    permissions = workflow.split("permissions:", 1)[1].split("\nenv:", 1)[0]
    assert "actions: read" in permissions
    assert "profile-release-selection" in resolver
    assert "reuse_run_id:" in resolver
    assert '--source-commit "$SOURCE_COMMIT"' in resolver
    assert "needs.resolve-profile-assets.outputs.reuse_run_id == ''" in build
    assert "needs.resolve-profile-assets.outputs.reuse_run_id != ''" in reuse
    assert "run-id: ${{ needs.resolve-profile-assets.outputs.reuse_run_id }}" in reuse
    assert "github-token: ${{ github.token }}" in reuse
    assert "arch: [arm64, x86_64]" in reuse
    assert reuse.count("actions/download-artifact@") == 1
    assert reuse.count("actions/upload-artifact@") == 1
    assert "just _build-kernel" not in reuse
    assert "just _build-rootfs" not in reuse
    assert "needs.build-assets.result == 'success'" in author
    assert "needs.reuse-assets.result == 'success'" in author
    assert "build-assets" not in publish.splitlines()[0]


def test_staged_incompatible_profile_runs_every_non_activation_gate() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    reusable_fast_gate = (ROOT / ".github" / "workflows" / "fast-gate.yaml").read_text(
        encoding="utf-8"
    )
    fast_gate = _job(workflow, "fast-gate", "resolve-current-binary")
    build_assets = _job(workflow, "build-assets", "test-profile-pairing")
    pairing = _job(workflow, "test-profile-pairing", "publish-profile-release")
    publish = _job(workflow, "publish-profile-release", "deploy-channel")
    deploy = workflow.split("  deploy-channel:\n", maxsplit=1)[1]
    activation_ready = "needs.author-profile-release.outputs.activation_ready == 'true'"
    pairing_setup = _step(
        pairing,
        "Prepare exact profile and pulled binary pairing",
        "Qualify the profile assets",
    )
    qualify = _step(
        pairing,
        "Qualify the profile assets",
        "Record deferred profile staging boundary",
    )
    deferred = _step(pairing, "Record deferred profile staging boundary", None)
    deployable = _step(
        publish,
        "Build deployable channel from authored source manifest",
        "Attest VM asset provenance",
    )
    immutable = _step(publish, "Publish immutable GitHub profile release", None)

    assert "needs.author-profile-release.outputs.release_needed == 'true'" in pairing
    assert "uses: ./.github/workflows/fast-gate.yaml" in fast_gate
    assert "if:" not in fast_gate
    assert "fast-gate" in build_assets.splitlines()[0]
    assert "Run the complete fast gate" not in pairing
    assert "ACTIVATION_READY:" in pairing_setup
    assert "needs.author-profile-release.outputs.activation_ready" in pairing_setup

    # One verb, both shapes. The deferred/active choice is the lane's, made
    # from this flag, rather than two `if:`-guarded step pairs whose halves
    # could drift apart.
    assert "just qualify-assets" in qualify
    assert '"$PWD/target/candidate-profile-inputs"' in qualify
    assert "inputs.profile" in qualify
    assert "outputs.activation_ready" in qualify
    assert "needs.author-profile-release.outputs.activation_ready" in qualify
    assert "if:" not in qualify

    assert "Run the complete fast gate" in reusable_fast_gate
    assert "run: just fast-test" in reusable_fast_gate
    assert "run: uv run capsem-gate test-release-contracts" in reusable_fast_gate
    assert "needs.author-profile-release.outputs.activation_ready != 'true'" in deferred
    assert "outputs.product_compatible" in deferred
    assert "outputs.functional_ready" in deferred
    assert "outside this profile's declared compatibility range" in deferred
    assert "complete functional release binary cohort" in deferred
    assert "activation-ready profile cannot defer complete pairing gates" in deferred
    assert f"if: ${{{{ {activation_ready} }}}}" in deployable
    assert "if:" not in immutable
    assert "needs.publish-profile-release.outputs.activation_ready == 'true'" in deploy


def test_cold_channel_pairing_branches_before_package_selection() -> None:
    """The first profile can publish inactive before this channel has binaries.

    The hosted stable/code run 31876487898 reached this exact boundary with an
    explicit empty-package public-before report and failed because package
    selection was unconditional.  Keep that decision ahead of every package
    staging/install action while retaining manifest/profile verification.
    """
    script = (ROOT / "scripts" / "stage-profile-pairing.sh").read_text(encoding="utf-8")
    branch = script.index('if [[ "$ACTIVATION_READY" == "false" ]]')

    assert script.index("scripts/verify-release-inputs.py") < branch
    assert script.index("scripts/fetch-release-artifacts.py") < branch
    assert branch < script.index("--binary-dir target/debug")
    assert branch < script.index("--print-package-path")
    assert branch < script.index("scripts/install-deb-runtime-dependencies.py")
    assert '[[ "$ACTIVATION_READY" == "true" ]]' in script


def test_cold_channel_pairing_executes_no_package_action(
    tmp_path: Path,
) -> None:
    calls = tmp_path / "uv-calls"
    github_env = tmp_path / "github-env"
    github_env.touch()
    for family in ("packages", "profiles"):
        destination = tmp_path / "target" / "profile-public-before" / family
        destination.mkdir(parents=True)
        (destination / "manifest.json").write_text("{}\n", encoding="utf-8")

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    uv = bin_dir / "uv"
    uv.write_text(
        '#!/bin/sh\nprintf "%s\\n" "$*" >> "$UV_CALLS"\n',
        encoding="utf-8",
    )
    uv.chmod(0o755)
    environment = {
        **os.environ,
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "UV_CALLS": str(calls),
        "GITHUB_ENV": str(github_env),
        "GITHUB_REPOSITORY": "google/capsem",
        "PUBLICATION_IDENTITY": PUBLICATION_IDENTITY,
        "RELEASE_CHANNEL": "stable",
        "RELEASE_BASELINE_CHANNEL": "stable",
        "RELEASE_PROFILE": "code",
        "ACTIVATION_READY": "false",
    }

    subprocess.run(
        ["bash", str(ROOT / "scripts" / "stage-profile-pairing.sh")],
        cwd=tmp_path,
        env=environment,
        check=True,
    )

    invoked = calls.read_text(encoding="utf-8")
    assert invoked.count("scripts/verify-release-inputs.py") == 3
    assert "scripts/fetch-release-artifacts.py" in invoked
    for forbidden in (
        "scripts/stage-release-test-inputs.py",
        "scripts/install-deb-runtime-dependencies.py",
        "scripts/materialize-config.sh",
    ):
        assert forbidden not in invoked
    assert github_env.read_text(encoding="utf-8") == ""


def test_cold_channel_pairing_rejects_an_unknown_activation_decision(
    tmp_path: Path,
) -> None:
    github_env = tmp_path / "github-env"
    completed = subprocess.run(
        ["bash", str(ROOT / "scripts" / "stage-profile-pairing.sh")],
        cwd=tmp_path,
        env={
            **os.environ,
            "GITHUB_ENV": str(github_env),
            "GITHUB_REPOSITORY": "google/capsem",
            "PUBLICATION_IDENTITY": PUBLICATION_IDENTITY,
            "RELEASE_CHANNEL": "stable",
            "RELEASE_BASELINE_CHANNEL": "stable",
            "RELEASE_PROFILE": "code",
            "ACTIVATION_READY": "unknown",
        },
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode != 0
    assert "must be true or false" in completed.stderr
    assert not github_env.exists()


def test_hosted_macos_never_claims_the_local_apple_vz_proof() -> None:
    """GitHub-hosted macOS cannot nest Apple Virtualization.framework, so the
    boot proof belongs to a local run and the workflow must not claim it."""
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    release_skill = (ROOT / "skills" / "release-process" / "SKILL.md").read_text(encoding="utf-8")
    local_gate = (ROOT / "justfile").read_text(encoding="utf-8")

    assert "test-profile-arm64-boot:" not in workflow
    assert "scripts/prove-release-profile-assets.py" not in workflow
    assert "Local Apple Silicon `just test-clean` owns that VZ proof" in release_skill
    assert "_gate-assets" in local_gate

    # Local Apple VZ remains a deliberate pre-release diagnostic; the hosted
    # lane is the publication authority and never claims nested VZ support.
    order = list(_release_plan("release-profile", "stable", "code").labels)
    assert order[0] == "source.worktree-clean"
    assert "qualification.accept" not in order
    assert order.index("source.publish-ref") < order.index("release")

    nightly = list(_release_plan("release-profile", "nightly", "code").labels)
    assert "qualification.accept" not in nightly


def test_profile_activation_readiness_requires_the_pulled_binary_functional_cohort() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    resolver = _job(workflow, "resolve-current-binary", "cloudflare-release-site-preflight")
    author = _job(workflow, "author-profile-release", "test-profile-pairing")

    assert "functional_ready:" in resolver
    assert "--check-functional-cohort" in resolver
    assert "id: functional-cohort" in resolver
    assert "needs.resolve-current-binary.outputs.functional_ready" in author
    assert "compatible_with_current_binary" in author
    assert "PRODUCT_COMPATIBLE=" in author
    assert "FUNCTIONAL_READY=" in author
    assert "ACTIVATION_READY=false" in author


def test_profile_then_binary_reuses_authored_source_without_rebuilding_assets() -> None:
    profile_workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    publish = _job(profile_workflow, "publish-profile-release", "deploy-channel")
    binary = BINARY_WORKFLOW.read_text(encoding="utf-8")

    assert "name: authored-profile-channel-source" in publish
    assert "name: authored-profile-candidate" in publish
    assert "--profile-source-root target/profile-candidate" in publish
    assert "just _build-kernel" not in publish
    assert "just _build-rootfs" not in publish
    assert "cargo run -p capsem-admin -- release" not in publish

    assert "Fetch latest selected channel source manifest" in binary
    assert "Resolve exact candidate-after profiles" in binary
    assert "kind: profiles" in binary
    assert binary.index("Record binary candidate metadata once") < binary.index(
        "Qualify the candidate binaries"
    )
    assert "Prove binary candidate preserved every profile" in binary
    assert 'before.get("profiles") != after.get("profiles")' in binary
    assert "name: binary-channel-candidate" in binary


def test_profile_publication_retry_verifies_owned_bytes_and_uploads_only_missing() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    publisher = (ROOT / "scripts" / "publish-immutable-release-assets.sh").read_text(
        encoding="utf-8"
    )
    publish = _job(workflow, "publish-profile-release", "deploy-channel")
    immutable = _step(publish, "Publish immutable GitHub profile release", None)

    assert "scripts/publish-immutable-release-assets.sh" in immutable
    assert (
        'CAPSEM_RELEASE_CREATE_TITLE="Capsem $CHANNEL/${{ inputs.profile }} '
        '$PROFILE_REVISION ($SOURCE_COMMIT)"' in immutable
    )
    assert "CAPSEM_RELEASE_CREATE_NOTES_FILE=" in immutable
    assert 'CAPSEM_RELEASE_CREATE_TARGET="$SOURCE_COMMIT"' in immutable
    assert "gh release create" not in immutable
    assert "gh release download" not in immutable
    assert "gh release upload" not in immutable
    assert "--clobber" not in immutable
    assert publisher.count("--resume-owned") == 2
    assert publisher.count("--missing-output") == 2
    assert "while IFS= read -r missing" in publisher
    assert 'gh release upload "$release_tag"' in publisher


def test_profile_provenance_precedes_authoritative_source_publication() -> None:
    workflow = PROFILE_WORKFLOW.read_text(encoding="utf-8")
    publish = _job(workflow, "publish-profile-release", "deploy-channel")
    names = [
        step.get("name")
        for step in (yaml.safe_load(publish) or {}).get("steps") or ()
        if isinstance(step, dict)
    ]

    assert names.index("Attest VM asset provenance") < names.index(
        "Publish immutable GitHub profile release"
    )


def test_binary_publication_retry_verifies_owned_bytes_and_uploads_only_missing() -> None:
    workflow = BINARY_WORKFLOW.read_text(encoding="utf-8")
    publisher = (ROOT / "scripts" / "publish-immutable-release-assets.sh").read_text(
        encoding="utf-8"
    )
    create = _job(workflow, "create-release", "assemble-release-channel")
    assemble = _job(workflow, "assemble-release-channel", "verify-release-candidate")
    verify = _job(workflow, "verify-release-candidate", "deploy-release-channel")
    github_release = _step(create, "Create GitHub release", None)
    source_manifest = _step(
        verify,
        "Persist mutated source manifest on the immutable binary release",
        None,
    )

    for step in (github_release, source_manifest):
        assert "scripts/publish-immutable-release-assets.sh" in step
        assert "gh release upload" not in step
        assert "--clobber" not in step

    assert "Persist mutated source manifest" not in assemble
    assert verify.index(
        "Prove install.sh selects and installs the candidate Linux package"
    ) < verify.index("Persist mutated source manifest on the immutable binary release")
    assert workflow.count("scripts/publish-immutable-release-assets.sh") == 2
    assert "gh release download" in publisher
    assert "verify-immutable-publication.py" in publisher
    assert publisher.count("--resume-owned") == 2
    assert publisher.count("--missing-output") == 2
    assert "while IFS= read -r missing" in publisher
    assert 'gh release upload "$release_tag"' in publisher
    assert "--clobber" not in publisher
    assert 'gh release create "$release_tag"' in publisher

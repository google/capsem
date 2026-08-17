"""Contracts for serialized binary/profile release ownership.

These tests intentionally inspect only public commands and workflow orchestration.
Artifact correctness remains covered by the executable lane and glow-up suites.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from helpers.workflow_contract import workflow_reachable_text

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


def _release_plan(command: str, *arguments: str):
    """The plan a release command would run, without running any of it."""
    import argparse

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand
    from capsem.gate.proc import Runner
    from capsem.gate.sourcecommit import SourceCommit

    names = ("channel", "profile")
    parsed = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        source_commit=SourceCommit("0" * 40),
        **dict(zip(names, arguments, strict=False)),
    )
    return GateCommand.registry[command](Runner(ROOT), parsed).plan()


def _publishing(plan) -> str:
    """What the steps after immutable source publication would run."""
    labels = list(plan.labels)
    after = labels[labels.index("source.publish-ref") :]
    return "\n".join(line for label in after for line in plan.step_named(label).render())


def _release_order(command: str, *arguments: str) -> list[str]:
    """Every step, in an order the graph permits."""
    return list(_release_plan(command, *arguments).labels)


def _context(runner):
    """A context for *reading* a release plan, against the real checkout.

    `observing` is the whole point. These tests run real plans -- built from
    the real config, so the argv is real -- and a plan that runs does what its
    actions say. `source.record` sits ahead of the step this file fails on, so
    without this it wrote the recording runner's empty output over the state
    file of whichever gate was running, and that gate died forty minutes later
    in `source.verify` reporting a HEAD change on a tree nobody had touched.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.context import Context

    return Context(runner, gate_config.load(ROOT), observing=True)


def test_release_commands_are_two_single_purpose_recipes() -> None:
    """Each owns one artifact family, and neither rebuilds the other's.

    The recipes dispatch, so this asks the plans. That is the stronger
    question: a recipe body could stop *containing* `just test` while still
    running it, and could contain it while running it too late.
    """
    justfile = "\n" + _read("justfile")

    binary_plan = _release_plan("release-binaries", "nightly")
    profile_plan = _release_plan("release-profile", "nightly", "code")
    binary = _publishing(binary_plan)
    profile = _publishing(profile_plan)

    # Each lane owns one artifact family, and neither rebuilds the other's.
    assert "scripts/release-binaries.py" in binary
    assert "capsem-admin" not in binary

    assert "capsem-admin -- release" in profile
    assert "scripts/release-binaries.py" not in profile

    retired_commands = (
        "release",
        "prepare-release",
        "qualify-" + "release",
        "cut-" + "release",
    )
    for retired in retired_commands:
        assert f"\n{retired}:" not in justfile
        assert f"\n{retired} " not in justfile


@pytest.mark.parametrize(
    "command, arguments, publication",
    [
        ("release-binaries", ("stable",), "release"),
        ("release-profile", ("stable", "code"), "release"),
    ],
)
def test_nothing_is_published_before_the_complete_gate_passes(
    command: str, arguments: tuple[str, ...], publication: str
) -> None:
    """Nothing publishes before exact qualification is revalidated."""
    order = _release_order(command, *arguments)

    assert order[0] == "source.worktree-clean"
    assert order.index("qualification.accept") < order.index("source.remote-main")
    if command == "release-binaries":
        assert order.index("source.remote-main") < order.index("precheck")
        assert order.index("precheck") < order.index("source.publish-ref")
    assert not any(
        label.startswith(("fast.", "static.", "artifacts.", "functional.", "glowup."))
        for label in order
    )
    assert order.index("source.publish-ref") < order.index(publication)


@pytest.mark.parametrize(
    ("recipe", "arguments", "release_trace"),
    (
        (
            "release-binaries",
            ("stable",),
            r"scripts/release-binaries\.py stable 0{40}",
        ),
        (
            "release-profile",
            ("stable", "code"),
            r"capsem-admin -- release --channel stable --profile code --source-commit 0{40}",
        ),
    ),
)
def test_public_release_command_accepts_journal_then_runs_preflight_before_mutation(
    tmp_path: Path,
    recipe: str,
    arguments: tuple[str, ...],
    release_trace: str,
) -> None:
    """The graph orders journal acceptance, preflight, then publication."""
    plan = _release_plan(recipe, *arguments)
    order = list(plan.labels)

    rendered = plan.describe()
    assert "require complete qualification journal" in rendered
    assert "publish-release-source.py" in rendered
    assert "--check" in rendered
    if recipe == "release-binaries":
        assert "release-binaries.py --precheck stable" in rendered
        assert "fetch-channel-source-manifest.py" in rendered

    assert order[0] == "source.worktree-clean"
    if recipe == "release-binaries":
        assert order.index("source.remote-main") < order.index("precheck")
    assert order.index("qualification.accept") < order.index("source.publish-ref")
    assert order.index("source.publish-ref") < order.index("release")

    # And the publishing step is the one the trace names.
    import re

    assert re.search(release_trace, "\n".join(plan.step_named("release").render())), (
        f"the release step does not run {release_trace}"
    )


@pytest.mark.parametrize(
    ("recipe", "arguments"),
    (
        ("release-binaries", ("stable",)),
        ("release-profile", ("stable", "code")),
    ),
)
def test_missing_qualification_prevents_every_release_side_effect(
    tmp_path: Path,
    recipe: str,
    arguments: tuple[str, ...],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A missing exact journal fails the first edge and skips publication."""
    from helpers.gate import RecordingRunner

    from capsem.gate.errors import GateError

    del tmp_path, monkeypatch
    runner = RecordingRunner(ROOT)
    plan = _release_plan(recipe, *arguments)

    with pytest.raises(GateError):
        plan.run(_context(runner))

    issued = "\n".join(runner.rendered)
    for mutation in (
        "scripts/release-binaries.py stable 0000000000000000000000000000000000000000",
        "capsem-admin -- release",
    ):
        assert mutation not in issued, f"{mutation} ran after a failing gate"
    assert "publish-release-source.py" not in issued

    outcomes = plan.outcomes
    assert outcomes["qualification.accept"].status == "failed"
    assert outcomes["source.publish-ref"].status == "skipped"
    assert outcomes["release"].status == "skipped"


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


def test_profile_dispatch_is_correlated_and_awaited_before_the_next_lane() -> None:
    workflow = _workflow("release-assets.yaml")
    admin = _read("crates/capsem-admin/src/main.rs")

    assert (
        "run-name: Release profile ${{ inputs.channel }}/${{ inputs.profile }} "
        "${{ inputs.dispatch_id }}"
    ) in workflow
    assert "dispatch_id:" in workflow
    assert (
        "required: true"
        in workflow.split("dispatch_id:", maxsplit=1)[1].split("dry_run:", maxsplit=1)[0]
    )
    assert 'format!("dispatch_id={dispatch_id}")' in admin
    assert '"watch".to_string()' in admin
    assert '"--exit-status".to_string()' in admin
    assert "run_id: Option<u64>" in admin


def test_daily_scheduler_runs_unattended_with_no_local_qualification() -> None:
    """Nightly takes the latest `main`, builds it, and publishes what passes.

    Spec 13.2: freeze the SHA, invoke the profile command per selected profile,
    then the binary command, and dispatch nothing directly. Nothing here
    qualifies anything -- the lanes it dispatches prove themselves, publishing
    only when their pairing job succeeded.

    That is only possible because `[release].locally_qualified_channels` omits
    nightly. A qualification journal is written solely by `just test` and
    archived per machine, so requiring one would require a human at a
    particular keyboard every morning. Every nightly run from 2026-08-05 failed
    for want of a journal a hosted runner cannot produce.
    """
    workflow = _workflow("release-nightly.yaml")
    release = _job_block(workflow, "nightly-release")

    assert workflow.count("cron:") == 1
    assert "push:" not in workflow

    commands = [
        "just release-profile nightly code ${{ github.sha }}",
        "just release-profile nightly co-work ${{ github.sha }}",
        "just release-binaries nightly ${{ github.sha }}",
    ]
    for command in commands:
        assert command in release, f"nightly scheduler must run {command!r}"
    offsets = [release.index(command) for command in commands]
    assert offsets == sorted(offsets), "profiles before binaries"

    # It qualifies nothing, and it builds nothing: it dispatches and waits.
    assert "just test" not in workflow
    assert "/dev/kvm" not in workflow
    assert "musl" not in workflow

    assert workflow.count("runs-on:") == 1
    assert "needs:" not in workflow
    assert "ref: main" not in workflow
    assert workflow.count("ref: ${{ github.sha }}") == 1
    assert "release.yaml" not in workflow
    assert "release-assets.yaml" not in workflow


def test_the_scheduler_meets_every_precondition_its_release_commands_check() -> None:
    """The runner builds nothing, but it is not therefore setup-free.

    Its sibling above pins what this job must *not* do -- no `just test`, no
    KVM, no musl -- because it dispatches rather than builds. Nothing pinned
    what it must still provide, and a refactor that stripped it down to a
    dispatcher removed a fourth step along with those three. It looked like
    build machinery. It was not: it served the release guard.

    Five consecutive nights then failed at the first release command with
    `source commit ... is not already on local main`, which is the guard
    working correctly against an absence nothing was asserting.

    Both preconditions here are about the checkout rather than the build:

      - `actions/checkout` leaves a detached HEAD, so there is no local `main`
        for `require_local_main` to read
      - publishing the immutable source ref is a `git push` over HTTPS from a
        detached prefix under ~/.cg that never saw the checkout's credential
        header, and a token in the environment is not a git credential

    Both must precede the first release command, since the first one to run
    checks them.
    """
    release = _job_block(_workflow("release-nightly.yaml"), "nightly-release")

    local_main = release.find("git branch -f main")
    credentials = release.find("insteadOf")
    first_release = release.find("just release-")

    assert local_main != -1, (
        "the scheduler must give require_local_main a local branch to read; "
        "a detached checkout has none and every release command refuses"
    )
    assert credentials != -1, (
        "the scheduler must let git authenticate, or publishing the immutable "
        "source ref prompts for a username and dies with no terminal"
    )
    assert first_release != -1
    assert local_main < first_release, "establish local main before releasing"
    assert credentials < first_release, "authenticate before releasing"


def test_nightly_releases_without_a_journal_while_stable_still_demands_one() -> None:
    """The one asymmetry that lets an unattended rebuild exist at all."""
    from capsem.gate import config as gate_config

    qualified = gate_config.load(ROOT).release.locally_qualified_channels
    assert "stable" in qualified
    assert "nightly" not in qualified

    for command, arguments in (
        ("release-binaries", ("nightly",)),
        ("release-profile", ("nightly", "code")),
    ):
        order = _release_order(command, *arguments)
        assert "qualification.accept" not in order, (
            f"{command} nightly plans a journal step it can never satisfy unattended"
        )
        assert "source.remote-main" in order
        assert order.index("source.publish-ref") < order.index("release")


def test_daily_scheduler_forwards_the_channel_source_token() -> None:
    release = _job_block(_workflow("release-nightly.yaml"), "nightly-release")

    assert "GITHUB_TOKEN: ${{ github.token }}" in release


def test_nightly_binary_rebuild_is_correlated_but_does_not_republish_identity() -> None:
    workflow = _workflow("release.yaml")
    script = _read("scripts/release-binaries.py")
    create = _job_block(workflow, "create-release")

    assert (
        "run-name: Release ${{ inputs.channel }} ${{ inputs.tag }} ${{ inputs.dispatch_id }}"
    ) in workflow
    assert "dispatch_id:" in workflow
    assert "publish:" in workflow
    assert "if: ${{ inputs.publish == true }}" in create
    assert 'f"dispatch_id={dispatch_id}"' in script
    assert 'f"publish={str(publish).lower()}"' in script
    assert '"--exit-status"' in script


def test_nightly_profiles_always_rebuild_while_stable_retry_can_reuse() -> None:
    workflow = _workflow("release-assets.yaml")
    resolver = _job_block(workflow, "resolve-profile-assets")
    build = _job_block(workflow, "build-assets")
    reuse = _job_block(workflow, "reuse-assets")

    assert "if: ${{ inputs.channel == 'stable' }}" in resolver
    assert "inputs.channel == 'nightly'" in build
    assert "inputs.channel == 'stable'" in reuse
    assert "resolve-reusable-profile-assets.py" in resolver


def test_release_lanes_run_one_reusable_fast_gate_before_builders() -> None:
    reusable = _workflow("fast-gate.yaml")
    assert "workflow_call:" in reusable
    assert "run: just fast-test" in reusable
    linux_prerequisites = reusable.index("Install Linux workspace lint prerequisites")
    gate = reusable.index("Run the complete fast gate")
    assert linux_prerequisites < gate

    binary = _workflow("release.yaml")
    assert "uses: ./.github/workflows/fast-gate.yaml" in _job_block(binary, "fast-gate")
    assert "needs: [runtime-preflight, fast-gate]" in _job_block(binary, "preflight")
    assert "Run the complete fast gate" not in _job_block(binary, "test-binary-pairing")

    profile = _workflow("release-assets.yaml")
    assert "uses: ./.github/workflows/fast-gate.yaml" in _job_block(profile, "fast-gate")
    build_assets = _job_block(profile, "build-assets")
    assert "fast-gate" in build_assets.splitlines()[1]
    assert "Run shared static module" not in _job_block(profile, "test-profile-pairing")
    assert "Run shared release contracts" not in _job_block(profile, "test-profile-pairing")


def test_release_profile_downloads_share_one_manifest_addressed_cache_module() -> None:
    action = _read(".github/actions/fetch-release-inputs/action.yaml")

    assert "scripts/fetch-release-artifacts.py" in action
    assert '--manifest-url "${{ inputs.manifest-url }}"' in action
    assert "--cache-dir target/release-input-cache" in action
    assert "--prune-cache" not in action
    assert "actions/cache/restore@" in action
    assert "actions/cache/save@" in action
    assert "steps.fetch.outputs.cache-misses != '0'" in action
    assert "local-publication-base:" in action
    assert "local-publication-dir:" in action
    assert '--local-publication-base "${{ inputs.local-publication-base }}"' in action
    assert '--local-publication-dir "${{ inputs.local-publication-dir }}"' in action
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
    assert "just qualify-binaries" in workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
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
    pairing = workflow_reachable_text(
        ROOT,
        WORKFLOWS / "release-assets.yaml",
        job="test-profile-pairing",
    )

    resolve_package = pairing.index("--print-package-path")
    install_dependencies = pairing.index('scripts/install-deb-runtime-dependencies.py "$package"')
    functional = pairing.index("just qualify-assets")

    assert resolve_package < install_dependencies < functional
    assert "sudo dpkg -i" not in pairing
    assert not re.search(r"sudo apt-get install[^\n]*(?:\$package|\.deb)", pairing)


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

    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in resolve
    assert "allow-empty-profiles: ${{ steps.public-before.outputs.bootstrap }}" in resolve
    assert "allow-empty-packages: ${{ steps.public-before.outputs.bootstrap }}" in resolve
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
    workflow = workflow_reachable_text(ROOT, WORKFLOWS / "release-assets.yaml")

    assert "Validate selected channel profile through capsem-admin" in workflow
    assert "Select exact public-before manifest" in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "--bootstrap-missing-first-party" in workflow
    assert '--source-commit "${{ inputs.source_commit }}"' in workflow
    assert '--profile "${{ inputs.profile }}"' in workflow
    assert "Project inactive first-channel public-before state" in workflow
    assert "scripts/project-first-channel-before.py" in workflow
    assert '--retired "${{ steps.public-before.outputs.retired }}"' in workflow
    assert "Select public-before authority for exact pairing" in workflow
    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in workflow
    assert "Fetch exact deployed public-before package" in workflow
    assert "Fetch exact deployed public-before profiles" in workflow
    assert "bootstrap-manifest-url:" not in workflow
    assert "allow-empty-profiles: ${{ steps.public-before.outputs.bootstrap }}" in workflow
    assert "capsem-admin -- release" in workflow
    assert "--publication-base" in workflow
    assert "channel-source-$CHANNEL.json" in workflow
    assert "--public-manifest target/profile-public-before/profiles/manifest.json" in workflow
    assert "steps.profile-delta.outputs.release_needed == 'true'" in workflow
    assert "check-profile-release-delta.py" in workflow
    assert "check-asset-release-delta.py" not in workflow
    assert "just qualify-assets" in workflow
    assert "--shared-config-root config" in workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
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
    pairing = workflow_reachable_text(
        ROOT,
        WORKFLOWS / "release-assets.yaml",
        job="test-profile-pairing",
    )
    publish = _job_block(workflow, "publish-profile-release")

    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in resolve
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
    assert "group: capsem-public-channel-deploy" in deploy
    assert "cancel-in-progress: false" in deploy
    assert "check-channel-deploy-freshness.py" in deploy
    assert "build-complete-release-channel.py" not in deploy

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


def test_release_runtime_preflight_bootstraps_only_from_manifest_catalog() -> None:
    preflight = _workflow("release-runtime-preflight.yaml")
    binary = _workflow("release.yaml")
    profile = _workflow("release-assets.yaml")

    assert "bootstrap_missing_first_party:" in preflight
    assert "scripts/select-runtime-preflight-manifest.py" in preflight
    assert "--bootstrap-missing-first-party" in preflight
    assert "steps.manifest.outputs.manifest-url" in preflight
    assert "ASSET_MANIFEST_URL" not in preflight

    assert "bootstrap_missing_first_party: true" in profile
    assert "bootstrap_missing_first_party: true" in binary


def test_binary_bootstrap_uses_donor_only_as_public_before() -> None:
    binary = _workflow("release.yaml")
    resolver = binary.split("  resolve-channel-source:\n", maxsplit=1)[1].split(
        "\n  preflight:\n", maxsplit=1
    )[0]

    assert "scripts/select-runtime-preflight-manifest.py" in resolver
    assert "--bootstrap-missing-first-party" in resolver
    assert "steps.public-before.outputs.manifest-url" in resolver
    assert "steps.public-before.outputs.bootstrap" in resolver
    assert "steps.public-before.outputs.retired" in resolver
    assert '--source-commit "${{ inputs.source_commit }}"' in resolver
    assert "scripts/project-first-channel-before.py" in resolver
    assert "Fetch latest selected channel source manifest" in resolver
    source_fetch = resolver.split(
        "- name: Fetch latest selected channel source manifest", maxsplit=1
    )[1].split("- name:", maxsplit=1)[0]
    assert "--bootstrap-missing-first-party" not in source_fetch
    assert "--require-profile-membership" in source_fetch

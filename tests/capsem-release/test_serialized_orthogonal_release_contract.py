"""Contracts for serialized binary/runtime release ownership.

These tests intentionally inspect only public commands and workflow orchestration.
Artifact correctness remains covered by the executable lane and glow-up suites.
"""

from __future__ import annotations

import importlib
import tomllib
from itertools import product
from pathlib import Path

import pytest
from capsem_builder.release.tools import nightly_release_scheduler, release_binaries
from helpers.workflow_contract import (
    emitted_assignment_names,
    parsed_commands,
    workflow_reachable_shell,
    workflow_reachable_text,
    workflow_step,
)

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
CHANNEL_GROUP = "group: capsem-release-${{ inputs.channel }}"
SOURCE_COMMIT = "a" * 40


def _read(path: str | Path) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def _workflow(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def _nightly_scheduler():
    return importlib.reload(nightly_release_scheduler)


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

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.proc import Runner
    from capsem_builder.gate.sourcecommit import SourceCommit

    (channel,) = arguments
    parsed = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        source_commit=SourceCommit("0" * 40),
        channel=channel,
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
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.context import Context

    return Context(runner, gate_config.load(ROOT), observing=True)


def test_release_commands_are_two_single_purpose_recipes() -> None:
    """Each owns one artifact family, and neither rebuilds the other's.

    The recipes dispatch, so this asks the plans. That is the stronger
    question: a recipe body could stop *containing* `just test` while still
    running it, and could contain it while running it too late.
    """
    justfile = "\n" + _read("justfile")

    binary = _publishing(_release_plan("release-binaries", "nightly"))
    runtime = _publishing(_release_plan("release-assets", "nightly"))

    # Each lane owns one artifact family, and neither rebuilds the other's.
    assert "build_system/scripts/release/release-binaries.py" in binary
    assert "capsem-admin" not in binary

    assert "capsem-admin -- release" in runtime
    assert "build_system/scripts/release/release-binaries.py" not in runtime

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
        ("release-assets", ("stable",), "release"),
    ],
)
def test_nothing_is_published_before_release_preflight_passes(
    command: str, arguments: tuple[str, ...], publication: str
) -> None:
    """The dispatcher consumes local proof and validates source before any lane runs."""
    order = _release_order(command, *arguments)

    assert order[0] == "source.worktree-clean"
    assert order[1] == "qualification.accept"
    assert order.index("source.worktree-clean") < order.index("source.remote-main")
    if command == "release-binaries":
        assert order.index("source.remote-main") < order.index("precheck")
        assert order.index("precheck") < order.index("source.publish-ref")
    assert not any(
        label.startswith(("fast.", "static.", "artifacts.", "functional.", "glowup."))
        for label in order
    )
    assert order.index("source.publish-ref") < order.index(publication)


def test_hosted_release_failure_cleans_only_its_unpublished_version_claim() -> None:
    workflow = _workflow("release.yaml")
    cleanup = _job_block(workflow, "cleanup-unpublished-version-claim")

    assert "always()" in cleanup
    assert "failure() || cancelled()" in cleanup
    assert "needs: [verify-release-downloads]" in cleanup
    assert "release_version_tag.py cleanup-exact" in cleanup
    assert '--tag "$RELEASE_TAG"' in cleanup
    assert '--source-commit "$SOURCE_COMMIT"' in cleanup
    assert '--repository "$GITHUB_REPOSITORY"' in cleanup


@pytest.mark.parametrize(
    ("recipe", "arguments", "release_trace"),
    (
        (
            "release-binaries",
            ("stable",),
            f"build_system/scripts/release/release-binaries.py stable {'0' * 40}",
        ),
        (
            "release-assets",
            ("stable",),
            f"capsem-admin -- release --channel stable --source-commit {'0' * 40}",
        ),
    ),
)
def test_public_release_command_runs_preflight_before_dispatching_qualification(
    tmp_path: Path,
    recipe: str,
    arguments: tuple[str, ...],
    release_trace: str,
) -> None:
    """The graph orders source preflight before the qualifying lane dispatch."""
    plan = _release_plan(recipe, *arguments)
    order = list(plan.labels)

    rendered = plan.describe()
    assert f"require complete qualification journal for {'0' * 40}" in rendered
    assert "publish-release-source.py" in rendered
    assert "--check" in rendered
    if recipe == "release-binaries":
        assert "release-binaries.py --precheck stable" in rendered
        assert "fetch-channel-source-manifest.py" in rendered
        # A binary release needs a staged runtime to pair with, the same
        # requirement the hosted lane states.
        assert "--require-runtime" in rendered

    assert order[0] == "source.worktree-clean"
    if recipe == "release-binaries":
        assert order.index("source.remote-main") < order.index("precheck")
    assert order.index("qualification.accept") < order.index("source.remote-main")
    assert order.index("source.publish-ref") < order.index("release")

    # And the publishing step is the one the trace names.
    assert release_trace in "\n".join(plan.step_named("release").render()), (
        f"the release step does not run {release_trace}"
    )


@pytest.mark.parametrize(
    ("recipe", "arguments"),
    (
        ("release-binaries", ("stable",)),
        ("release-assets", ("stable",)),
    ),
)
def test_release_dispatch_plan_only_revalidates_the_local_journal(
    tmp_path: Path,
    recipe: str,
    arguments: tuple[str, ...],
) -> None:
    """The dispatcher only revalidates the local journal; it runs no local suite."""
    del tmp_path
    plan = _release_plan(recipe, *arguments)
    assert plan.labels[1] == "qualification.accept"
    assert "qualification.waived" not in plan.labels
    assert next(iter(plan.labels)) == "source.worktree-clean"
    assert list(plan.labels).index("source.publish-ref") < list(plan.labels).index("release")


def test_binary_and_runtime_workflows_share_channel_transaction_lock() -> None:
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
        assert "inputs.tag" not in group_line


def test_runtime_dispatch_is_correlated_and_awaited_before_the_next_lane() -> None:
    workflow = _workflow("release-assets.yaml")
    admin = _read("crates/capsem-admin/src/main.rs")

    assert "run-name: Release runtime ${{ inputs.channel }} ${{ inputs.dispatch_id }}" in workflow
    assert 'format!("Release runtime {channel} {dispatch_id}")' in admin
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

    Spec 13.2: freeze the SHA, invoke the runtime asset command, then the
    binary command, and dispatch nothing directly. Nothing here
    qualifies anything -- the lanes it dispatches prove themselves, publishing
    only when their pairing job succeeded.

    Nightly is the one channel that consumes no machine-local journal: this
    runner is fresh and unattended and cannot run `just test`. Every channel
    an operator releases requires one (`[release].unattended_channels`).
    """
    workflow = _workflow("release-nightly.yaml")
    release = _job_block(workflow, "nightly-release")

    assert workflow.count("cron:") == 1
    assert "push:" not in workflow

    assert "build_system/scripts/release/nightly_release_scheduler.py" in release
    assert "--channel nightly" in release
    assert ' --source-commit "${{ github.sha }}"' in release
    assert "just release-" not in release

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
    assert "timeout-minutes: 360" in release


@pytest.mark.parametrize("exit_codes", tuple(product((0, 19), repeat=2)))
def test_nightly_scheduler_runs_every_lane_before_aggregate_verdict(
    exit_codes: tuple[int, int],
) -> None:
    scheduler = _nightly_scheduler()

    class Runner:
        def __init__(self) -> None:
            self.remaining = list(exit_codes)
            self.calls: list[tuple[str, ...]] = []

        def run(self, argv) -> int:
            self.calls.append(tuple(argv))
            return self.remaining.pop(0)

    runner = Runner()
    result = scheduler.run_schedule("nightly", SOURCE_COMMIT, runner)

    assert runner.calls == [
        ("just", "release-assets", "nightly", SOURCE_COMMIT),
        ("just", "release-binaries", "nightly", SOURCE_COMMIT),
    ]
    assert [outcome.lane for outcome in result.lanes] == ["assets", "binaries"]
    assert [outcome.exit_code for outcome in result.lanes] == list(exit_codes)
    assert result.ok is all(exit_code == 0 for exit_code in exit_codes)
    assert result.as_dict()["status"] == ("success" if result.ok else "failed")


def test_nightly_scheduler_records_launch_error_and_continues() -> None:
    scheduler = _nightly_scheduler()

    class Runner:
        def __init__(self) -> None:
            self.calls = 0

        def run(self, _argv) -> int:
            self.calls += 1
            if self.calls == 1:
                raise OSError("just is unavailable")
            return 0

    runner = Runner()
    result = scheduler.run_schedule("nightly", SOURCE_COMMIT, runner)

    assert runner.calls == 2
    assert [outcome.status for outcome in result.lanes] == ["launch-error", "success"]
    assert result.ok is False


@pytest.mark.parametrize(
    ("channel", "source_commit"),
    [
        ("stable", SOURCE_COMMIT),
        ("nightly", "main"),
        ("nightly", "A" * 40),
        ("nightly", SOURCE_COMMIT[:39]),
    ],
)
def test_nightly_scheduler_rejects_ambiguous_or_mutable_inputs(
    channel: str,
    source_commit: str,
) -> None:
    scheduler = _nightly_scheduler()

    with pytest.raises(ValueError):
        scheduler.schedule(channel, source_commit)


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
        detached prefix under cache/worktrees that never saw the checkout's credential
        header, and a token in the environment is not a git credential

    Both must precede the first release command, since the first one to run
    checks them.
    """
    release = _job_block(_workflow("release-nightly.yaml"), "nightly-release")

    local_main = release.find("git branch -f main")
    credentials = release.find("insteadOf")
    first_release = release.find("build_system/scripts/release/nightly_release_scheduler.py")

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


def test_only_the_unattended_channel_dispatches_without_a_local_journal() -> None:
    for command, arguments, journal in (
        ("release-binaries", ("stable",), True),
        ("release-assets", ("stable",), True),
        ("release-binaries", ("corp",), True),
        ("release-binaries", ("nightly",), False),
        ("release-assets", ("nightly",), False),
    ):
        order = _release_order(command, *arguments)
        assert ("qualification.accept" in order) is journal, (
            f"{command} {arguments[0]}: an operator release consumes the exact "
            "`just test` journal; only the unattended nightly scheduler does not"
        )
        assert "source.remote-main" in order
        assert order.index("source.publish-ref") < order.index("release")


def test_daily_scheduler_forwards_the_channel_source_token() -> None:
    release = _job_block(_workflow("release-nightly.yaml"), "nightly-release")

    assert "GITHUB_TOKEN: ${{ github.token }}" in release


def test_nightly_binary_rebuild_is_correlated_but_does_not_republish_identity() -> None:
    workflow = _workflow("release.yaml")
    script = Path(release_binaries.__file__).read_text(encoding="utf-8")
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


def test_runtime_lane_builds_only_a_runtime_the_channel_lacks_and_reuses_nothing() -> None:
    """A runtime revision is per commit, so there is no earlier run to reuse.

    The lane asks the channel source whether this commit's runtime is already
    authored before any image is built, and builds every architecture fresh
    when it is not.
    """
    workflow = _workflow("release-assets.yaml")
    resolve = _job_block(workflow, "resolve-current-binary")
    build = _job_block(workflow, "build-assets")

    assert "release_needed: ${{ steps.runtime-delta.outputs.release_needed }}" in resolve
    assert "check-runtime-release-delta.py" in resolve
    assert "if: ${{ needs.resolve-current-binary.outputs.release_needed == 'true' }}" in build
    assert 'just build-assets "$ASSET_ARCH"' in build
    assert "\n  reuse-assets:" not in workflow
    assert "actions/download-artifact" not in build


def test_release_lanes_run_one_reusable_fast_gate_before_builders() -> None:
    reusable = _workflow("fast-gate.yaml")
    assert "workflow_call:" in reusable
    assert "run: just fast-test" in reusable
    assert (
        "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in reusable
    )
    linux_prerequisites = reusable.index("Install Linux workspace lint prerequisites")
    gate = reusable.index("Run the complete fast gate")
    assert linux_prerequisites < gate

    binary = _workflow("release.yaml")
    assert "uses: ./.github/workflows/fast-gate.yaml" in _job_block(binary, "fast-gate")
    assert "needs: [runtime-preflight, fast-gate]" in _job_block(binary, "preflight")
    assert "Run the complete fast gate" not in _job_block(binary, "test-binary-pairing")

    runtime = _workflow("release-assets.yaml")
    assert "uses: ./.github/workflows/fast-gate.yaml" in _job_block(runtime, "fast-gate")
    build_assets = _job_block(runtime, "build-assets")
    assert "fast-gate" in build_assets.splitlines()[1]
    assert "Run shared static module" not in _job_block(runtime, "test-runtime-pairing")
    assert "Run shared release contracts" not in _job_block(runtime, "test-runtime-pairing")


def test_release_runtime_downloads_share_one_manifest_addressed_cache_module() -> None:
    action = _read(".github/actions/fetch-release-inputs/action.yaml")

    assert "build_system/scripts/release/fetch-release-artifacts.py" in action
    assert '--manifest-url "${{ inputs.manifest-url }}"' in action
    assert "--cache-dir cache/target/release/staging/input-cache" in action
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


def test_binary_lane_pulls_the_runtime_and_never_builds_it() -> None:
    workflow = _workflow("release.yaml")

    assert "Fetch latest selected channel source manifest" in workflow
    assert "binary-channel-source" in workflow
    assert "Resolve exact candidate-after runtime" in workflow
    assert "file://$PWD/cache/target/binary-channel/$RELEASE_CHANNEL/manifest.json" in workflow
    assert "just qualify-binaries" in workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
    # A runtime publishes no config: the service config is the checkout's.
    assert "--config-root" not in workflow
    assert "CAPSEM_CONFIG_ROOT=" not in workflow
    assert '--package-file "$package"' in workflow
    assert 'build_system/packaging/linux/install-deb-runtime-dependencies.py "$package"' in workflow
    assert "cp cache/target/release/staging/package-root/usr/bin/capsem*" not in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/cache/target/cargo/debug/capsem" in workflow

    for forbidden in (
        "just _build-kernel",
        "just _build-rootfs",
        "capsem-admin -- image build",
    ):
        assert forbidden not in workflow


def test_runtime_lane_installs_pulled_package_runtime_dependencies() -> None:
    pairing = workflow_reachable_shell(
        ROOT,
        WORKFLOWS / "release-assets.yaml",
        job="test-runtime-pairing",
    )

    resolve_package = pairing.index("--print-package-path")
    install_dependencies = pairing.index(
        'build_system/packaging/linux/install-deb-runtime-dependencies.py "$package"'
    )
    functional = pairing.index("just qualify-assets")

    assert resolve_package < install_dependencies < functional
    assert "sudo dpkg -i" not in pairing
    assert not any(
        command.program == "apt-get"
        and "sudo" in command.argv
        and any(word == "$package" or word.endswith(".deb") for word in command.argv)
        for command in parsed_commands(pairing, origin="release-assets:test-runtime-pairing")
    )


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
        "manifest-url: file://${{ github.workspace }}/cache/target/binary-channel/"
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

    assert '--channel "stable"' in resolve
    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in resolve
    assert "allow-empty-runtime: true" in resolve
    assert "allow-empty-packages: ${{ steps.public-before.outputs.bootstrap }}" in resolve
    assert "kind: packages" in resolve
    assert "kind: runtime" in resolve
    assert "architecture: x86_64" in resolve
    assert "binary-public-before-packages" in resolve
    assert "binary-public-before-runtime" in resolve

    assert "binary-public-before-packages" in pairing
    assert "binary-public-before-runtime" in pairing
    assert (
        "manifest-url: file://${{ github.workspace }}/cache/target/binary-channel/"
        "${{ inputs.channel }}/manifest.json"
    ) in pairing
    assert "kind: runtime" in pairing
    assert "cache/target/candidate-runtime-inputs" in pairing
    activation = workflow_step(
        WORKFLOWS / "release.yaml",
        "test-binary-pairing",
        "Activate exact candidate package binaries for functional tests",
    )
    exported = emitted_assignment_names(
        str(activation["run"]), origin="release.yaml:test-binary-pairing:activate"
    )
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_BASELINE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_INPUTS",
        "CAPSEM_RELEASE_AFTER_INPUTS",
    ):
        assert variable in exported


def test_macos_package_consumes_cargo_release_output() -> None:
    step = workflow_step(WORKFLOWS / "release.yaml", "build-app-macos", "Build .pkg installer")
    commands = parsed_commands(step["run"], origin="release:macos-package")
    package = next(command for command in commands if "build-pkg.sh" in command.argv[1])
    cargo = tomllib.loads(_read(".cargo/config.toml"))
    binary_dir = Path(cargo["build"]["target-dir"]) / "release"
    assert Path(package.argv[5]) == binary_dir, (
        "macOS packaging must consume the signed Cargo binaries, not the release distribution directory"
    )
    assert Path(package.argv[4]) == binary_dir / "bundle/macos/Capsem.app"
    assert package.argv[6] == "cache/target/release/staging/assets"


def test_runtime_lane_pulls_binary_and_never_builds_packages() -> None:
    workflow = workflow_reachable_text(ROOT, WORKFLOWS / "release-assets.yaml")

    assert "Validate the runtime release through capsem-admin" in workflow
    assert "Select exact public-before manifest" in workflow
    assert '--channel "stable"' in workflow
    assert "Fetch latest selected channel source manifest" in workflow
    assert "--bootstrap-missing-first-party" in workflow
    assert "SOURCE_COMMIT: ${{ inputs.source_commit }}" in workflow
    assert '--source-commit "$SOURCE_COMMIT"' in workflow
    assert "--runtime-revision" in workflow
    assert "Project inactive first-channel public-before state" in workflow
    assert "build_system/scripts/release/project-first-channel-before.py" in workflow
    assert "PUBLIC_BEFORE_RETIRED: ${{ steps.public-before.outputs.retired }}" in workflow
    assert '--retired "$PUBLIC_BEFORE_RETIRED"' in workflow
    assert "Select public-before authority for exact pairing" in workflow
    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in workflow
    assert "Fetch exact deployed public-before package" in workflow
    assert "Fetch exact deployed public-before runtime" in workflow
    assert "bootstrap-manifest-url:" not in workflow
    assert "allow-empty-runtime: true" in workflow
    assert "capsem-admin -- release" in workflow
    assert "--publication-base" in workflow
    assert "channel-source-$CHANNEL.json" in workflow
    assert "needs.resolve-current-binary.outputs.release_needed == 'true'" in workflow
    assert "check-runtime-release-delta.py" in workflow
    assert "check-asset-release-delta.py" not in workflow
    assert "just qualify-assets" in workflow
    assert "--shared-config-root" not in workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
    assert "--input-dir cache/target/runtime-public-before/packages" in workflow
    assert "--binary-dir cache/target/cargo/debug" in workflow
    assert "CAPSEM_TEST_BINARY=$PWD/cache/target/cargo/debug/capsem" in workflow

    for forbidden in (
        "just _cross-compile",
        "build_system/packaging/macos/build-pkg.sh",
        "build_system/packaging/linux/repack-deb.sh",
        "cargo tauri build",
    ):
        assert forbidden not in workflow


def test_runtime_selection_creates_clean_runner_output_parent() -> None:
    resolve = _job_block(_workflow("release-assets.yaml"), "resolve-current-binary")

    create_parent = resolve.index("mkdir -p cache/target")
    validate = resolve.index("cargo run -p capsem-admin -- validate")
    redirect = resolve.index("> cache/target/runtime-release-selection.json")

    assert create_parent < validate < redirect


def test_runtime_pairing_reuses_one_staged_publication_and_exact_public_before() -> None:
    workflow = _workflow("release-assets.yaml")
    resolve = _job_block(workflow, "resolve-current-binary")
    author = _job_block(workflow, "author-runtime-release")
    pairing = workflow_reachable_text(
        ROOT,
        WORKFLOWS / "release-assets.yaml",
        job="test-runtime-pairing",
    )
    publish = _job_block(workflow, "publish-runtime-release")

    assert "manifest-url: ${{ steps.public-before-authority.outputs.manifest-url }}" in resolve
    assert "kind: packages" in resolve
    assert "kind: runtime" in resolve
    assert "architecture: x86_64" in resolve
    assert "runtime-public-before-packages" in resolve
    assert "runtime-public-before-runtime" in resolve

    assert "stage-runtime-publication.py" in author
    assert "verify-runtime-publication.py" in author
    assert "name: authored-runtime-publication" in author

    for artifact in (
        "runtime-public-before-packages",
        "runtime-public-before-runtime",
        "authored-runtime-publication",
    ):
        assert artifact in pairing
    assert "--local-publication-base" in pairing
    assert "--local-publication-dir" in pairing
    assert "cache/target/candidate-runtime-inputs" in pairing
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_BASELINE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION=auto",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_INPUTS",
        "CAPSEM_RELEASE_AFTER_INPUTS",
        "CAPSEM_RELEASE_RUNTIME=1",
        "CAPSEM_RELEASE_CANDIDATE_RUNTIME_PUBLICATION",
        "CAPSEM_RELEASE_PUBLICATION_BASE",
    ):
        assert variable in pairing

    assert "name: authored-runtime-publication" in publish
    assert "stage-runtime-publication.py" not in publish
    assert "verify-runtime-publication.py" in publish


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
            assert "activate_production: false" in workflow
            continue
        assert CHANNEL_GROUP in workflow, f"{name} deploys production without the channel lock"
        assert "activate_production: false" not in workflow, (
            f"{name} bypasses preview-verified production activation"
        )


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

    for name in ("release.yaml", "release-assets.yaml"):
        workflow = _workflow(name)
        assert "uses: ./.github/workflows/release-runtime-preflight.yaml" in workflow


def test_release_runtime_preflight_bootstraps_only_from_manifest_catalog() -> None:
    preflight = _workflow("release-runtime-preflight.yaml")
    binary = _workflow("release.yaml")
    runtime = _workflow("release-assets.yaml")

    assert "bootstrap_missing_first_party:" in preflight
    assert "build_system/scripts/bootstrap/select-runtime-preflight-manifest.py" in preflight
    assert "--bootstrap-missing-first-party" in preflight
    assert "steps.manifest.outputs.manifest-url" in preflight
    assert "ASSET_MANIFEST_URL" not in preflight

    assert "bootstrap_missing_first_party: true" in runtime
    assert "bootstrap_missing_first_party: true" in binary


def test_binary_bootstrap_uses_donor_only_as_public_before() -> None:
    binary = _workflow("release.yaml")
    resolver = binary.split("  resolve-channel-source:\n", maxsplit=1)[1].split(
        "\n  preflight:\n", maxsplit=1
    )[0]

    assert "build_system/scripts/bootstrap/select-runtime-preflight-manifest.py" in resolver
    assert "--bootstrap-missing-first-party" in resolver
    assert "steps.public-before.outputs.manifest-url" in resolver
    assert "steps.public-before.outputs.bootstrap" in resolver
    assert "steps.public-before.outputs.retired" in resolver
    assert "SOURCE_COMMIT: ${{ inputs.source_commit }}" in binary
    assert '--source-commit "$SOURCE_COMMIT"' in resolver
    assert "build_system/scripts/release/project-first-channel-before.py" in resolver
    assert "Fetch latest selected channel source manifest" in resolver
    source_fetch = resolver.split(
        "- name: Fetch latest selected channel source manifest", maxsplit=1
    )[1].split("- name:", maxsplit=1)[0]
    assert "--bootstrap-missing-first-party" not in source_fetch
    assert "--require-runtime" in source_fetch

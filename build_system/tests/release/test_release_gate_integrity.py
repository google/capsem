"""Release gate identity, toolchain, and publication-order contracts."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
PINNED_RUST = "1.97.1"


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text(encoding="utf-8")


def _job_block(workflow: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [a-zA-Z0-9_-]+:\n|\Z)",
        workflow,
    )
    assert match is not None, f"workflow job {name!r} missing"
    return match.group(0)


def test_install_test_inherits_uv_through_its_exact_local_helper() -> None:
    """One parent owns uv; neither sealed child resolves a second image."""
    from capsem_builder.gate import config as gate_config

    parent = _read("build_system/docker/Dockerfile.host-builder")
    helper = _read("build_system/docker/Dockerfile.install-builder")
    child = _read("build_system/docker/Dockerfile.install-test")
    config = gate_config.load(PROJECT_ROOT)

    assert "FROM ${UV_IMAGE} AS uv-runtime" in parent
    assert "COPY --from=uv-runtime /uv /uvx /usr/local/bin/" in parent
    assert re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", config.hostimage.uv_image)
    assert helper.count("FROM ${BASE}") == 2
    assert "FROM ${BASE}" in child
    assert "astral-sh/uv" not in helper
    assert "astral-sh/uv" not in child


def selected_tools(job: str) -> set[str]:
    """The crates a job installs, resolved through the sets it names.

    A job used to spell its tools; now it names a set and
    `build_system/scripts/ci/gate-tool-list.py` derives them, so a guard asking "does this job
    install X" has to resolve the same way. That indirection is the point: the
    membership lives in one file, and a job cannot quietly hold a different
    subset from another job running the same suite.
    """
    import re
    import tomllib

    declared = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))[
        "toolchain"
    ]["sets"]
    chosen: set[str] = set()
    for match in re.findall(r"--sets ([a-z,]+)", job):
        for label in match.split(","):
            chosen.update(declared[label])
    return chosen


def test_every_fresh_ci_test_runner_preinstalls_the_exact_nextest() -> None:
    pin = "cargo-nextest"
    jobs = (
        (
            "fast-gate.yaml",
            "static",
            "Materialize locked qualification dependencies",
            "just fast-test",
        ),
        ("ci.yaml", "test-linux", None, "just test-linux-rust"),
        ("ci.yaml", "test", None, "cargo llvm-cov nextest"),
        (
            "release.yaml",
            "test-binary-pairing",
            "Prove Linux sandbox boundary",
            "just qualify-binaries",
        ),
        (
            "release-assets.yaml",
            "test-profile-pairing",
            "Prove Linux sandbox boundary",
            "just qualify-assets",
        ),
    )

    marker = "build_system/scripts/ci/gate-tool-list.py"
    for workflow_name, job_name, seal, consumer in jobs:
        job = _job_block(_read(f".github/workflows/{workflow_name}"), job_name)
        assert pin in selected_tools(job), f"{workflow_name}:{job_name} does not install {pin}"
        assert job.index(marker) < job.index(consumer)
        if seal is not None:
            assert job.index(marker) < job.index(seal) < job.index(consumer)


def test_every_runner_of_the_broad_suite_installs_what_that_suite_shells_out_to() -> None:
    """The gap the `b3sum` failure came through.

    One guard checked that *every* runner installs `cargo-nextest`. Another
    checked that *one* job installs `b3sum`. Neither said that a job installs
    what the suite it runs actually invokes, so the binary pairing gate went
    without `b3sum` while running the same broad suite as the fast gate -- and
    three asset-integrity tests failed on a missing tool ten minutes into a
    release where every binary had already built and installed.
    """
    needed = {"cargo-nextest", "b3sum"}
    runners = (
        ("fast-gate.yaml", "static"),
        ("ci.yaml", "test"),
        ("release.yaml", "test-binary-pairing"),
        ("release-assets.yaml", "test-profile-pairing"),
    )
    missing = []
    for workflow_name, job_name in runners:
        job = _job_block(_read(f".github/workflows/{workflow_name}"), job_name)
        absent = sorted(needed - selected_tools(job))
        if absent:
            missing.append(f"{workflow_name}:{job_name} lacks {absent}")
    assert not missing, "these run the broad suite without the tools it invokes: " + "; ".join(
        missing
    )


def test_host_builder_installs_the_same_exact_nextest() -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import hostimage

    config = gate_config.load(PROJECT_ROOT)
    host_builder = _read("build_system/docker/Dockerfile.host-builder")
    argument = next(
        name for name, tool in config.hostimage.cargo_tool_args.items() if tool == "cargo-nextest"
    )
    package, version = hostimage.cargo_tool(config=config, argument=argument)

    assert package == "cargo-nextest"
    assert version
    assert f'cargo install {package} --version "${{{argument}}}" --locked' in host_builder
    assert f"ARG {argument}" in host_builder


def test_just_test_holds_source_state_stable_without_archiving_benchmarks() -> None:
    """Both HEAD and the working-tree digest are captured before and compared
    after: a gate that qualified a HEAD nobody has, or a tree edited halfway
    through, proved nothing about any particular version."""
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.proc import Runner

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    command = GateCommand.registry["candidate"](
        Runner(root), argparse.Namespace(dry_run=False, graph=False, timing=False)
    )

    assert "capsem-gate candidate" in _read("justfile")

    # The source state is bracketed by two steps rather than read while the
    # plan is built: a value captured during construction names whatever was
    # checked out when the description was assembled, not what ran.
    labels = list(command._describe().labels)
    assert labels[0] == "source.record"
    assert labels[-1] == "source.verify"
    assert config.candidate.source_digest_script.endswith("source-state-digest.py")

    # Colima is given back on every path, including the aborted one -- which is
    # why it is a resource and not a step, and why the shell wrapper that used
    # to cover only part of the gate is gone.
    from helpers.gate import RecordingRunner

    held = {resource.name for resource in command.resources(RecordingRunner(PROJECT_ROOT))}
    assert {"colima", "orphan-accounting", "failure-evidence"} <= held

    # Declared in `[environment]`, which Workspace exports and Service reads.
    # This asserted the literal was spelled in `workspace.py`; both spelled it
    # separately, so an isolated workspace could export one name while the
    # daemon inside it honoured another.
    from capsem_builder.gate import config as gate_config

    names = gate_config.load(PROJECT_ROOT).environment
    assert names.benchmark_root == "CAPSEM_BENCHMARK_OUTPUT_ROOT"
    assert "environment.benchmark_root" in _read("build_system/builder/gate/workspace.py") or (
        "names.benchmark_root" in _read("build_system/builder/gate/workspace.py")
    )
    assert config.workspace.benchmark_root == "target/test-benchmarks"
    assert "benchmarks/**/data_*.json" in _read(".gitignore")


def _gate_plan():
    """The complete gate, as one plan.

    These contracts were written against `_test-candidate`'s shell body. The
    ordering they are about is edges in a single graph now, so it is read from
    there -- which also means they cover the whole gate rather than the part
    that happened to live in one recipe.
    """
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    return GateCommand.registry["candidate"](
        RecordingRunner(Path(__file__).resolve().parents[3]),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()


def _step_at(labels: list[str], fragment: str) -> int:
    for position, label in enumerate(labels):
        if fragment in label:
            return position
    raise AssertionError(f"no step matching {fragment!r} in:\n  " + "\n  ".join(labels))


def test_gate_run_retains_the_vm_performance_recordings_it_produces() -> None:
    """`functional` writes the VM recordings and `glowup` runs after it.

    While the wipe lived in the per-module runner, the later module deleted the
    earlier one's numbers, so a full gate produced a complete set and then threw
    most of it away. It is cleared exactly once, in the preparation phase,
    before any module runs.
    """
    from capsem_builder.gate import config as gate_config

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    labels = list(_gate_plan().labels)
    workspace = (root / "build_system/builder/gate/workspace.py").read_text(encoding="utf-8")

    assert config.workspace.benchmark_root == "target/test-benchmarks"
    # The workspace deliberately does not clear it on acquire: one gate runs
    # several modules through one workspace.
    assert "Deliberately not the benchmark root" in workspace

    cleared = _step_at(labels, "prepare.storage-budget")
    assert cleared < _step_at(labels, "functional.")
    assert cleared < _step_at(labels, "glowup.")


def test_full_gate_runs_capsem_bench_baseline_for_every_selected_profile() -> None:
    """One recorded baseline per selected profile, and exactly one.

    Two files launching VMs at once measure each other rather than Capsem,
    which is why the baseline claims `apple_vz` and runs alone.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import profiles

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    labels = list(_gate_plan().labels)

    for profile in profiles.selected(config):
        matching = [label for label in labels if label.endswith(f"pytest.benchmark.{profile}")]
        assert len(matching) == 1, f"{profile} has {len(matching)} recorded baselines, expected one"

    step = _gate_plan().step_named(next(label for label in labels if "pytest.benchmark." in label))
    assert [e.name for e in step.contends] == ["apple_vz"]


def test_full_gate_serializes_host_snapshot_files_without_dropping_coverage() -> None:
    """The snapshot files run once, alone, and are excluded from the parallel run.

    Production has one service and one service-scoped save/restore lock; an
    xdist worker per service does not reproduce that. Run in both places they
    would run twice, once in the way that proves nothing.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import pytestsuite

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    base = config.suites.pytest.base_profile

    broad = pytestsuite.broad(config, profile=base).argv(config)
    snapshot = pytestsuite.host_snapshot(config, profile=base)

    for path in config.suites.pytest.host_snapshot_serial:
        assert f"--ignore={path}" in broad, f"{path} runs twice"
        assert path in snapshot.argv(config)

    assert config.suites.pytest.stop_at_first in broad
    assert [e.name for e in snapshot.contends] == ["host_service"]
    assert not snapshot.parallel

    labels = list(_gate_plan().labels)
    assert (
        _step_at(labels, f"pytest.broad.{base}")
        < _step_at(labels, f"pytest.host-snapshot.{base}")
        < _step_at(labels, f"pytest.timing.{base}")
    )


def test_local_gate_bootstraps_docker_before_storage_preflight() -> None:
    """A storage budget measured before the daemon exists measures nothing."""
    labels = list(_gate_plan().labels)

    assert _step_at(labels, "prepare.bootstrap") < _step_at(labels, "prepare.storage-budget")


def test_macos_full_gate_holds_a_system_sleep_assertion() -> None:
    """A forty-minute run that dies at minute thirty because the machine slept
    proves nothing, and by then it is usually unattended."""
    from capsem_builder.gate import config as gate_config

    settings = gate_config.load(PROJECT_ROOT).candidate

    assert settings.keep_awake_command[0] == "caffeinate"
    assert settings.keep_awake_marker == "CAPSEM_TEST_CAFFEINATED"
    assert "keep_awake" in _read("build_system/builder/gate/candidate.py")


def test_toolchain_and_workflow_inputs_are_immutable_and_consistent() -> None:
    from capsem_builder.gate import config as gate_config

    toolchain = tomllib.loads(_read("rust-toolchain.toml"))
    assert toolchain["toolchain"]["channel"] == PINNED_RUST

    workflow_text = "\n".join(path.read_text(encoding="utf-8") for path in WORKFLOWS.glob("*.yaml"))
    assert "dtolnay/rust-toolchain@stable" not in workflow_text
    assert "toolchain: stable" not in workflow_text
    for block in workflow_text.split("uses: dtolnay/rust-toolchain@")[1:]:
        step = block.split("\n      - ", maxsplit=1)[0]
        assert f"toolchain: {PINNED_RUST}" in step
    for block in workflow_text.split("uses: taiki-e/install-action@")[1:]:
        step = block.split("\n      - ", maxsplit=1)[0]
        tool_line = next(line for line in step.splitlines() if "tool:" in line)
        handed = tool_line.split("tool:", maxsplit=1)[1].strip()
        # Derived from `[toolchain.sets]` rather than pinned in YAML. The
        # versions are still exact; they are just declared once, where the rest
        # of the toolchain is. See tests/citadel/test_ci_tools_come_from_config.
        assert handed == "${{ steps.gate_tools.outputs.list }}", handed

    builder = _read("build_system/docker/Dockerfile.host-builder")
    config = gate_config.load(PROJECT_ROOT)
    assert "FROM ${RUST_IMAGE} AS rust-toolchain" in builder
    assert PINNED_RUST in config.hostimage.rust_image
    assert 'case "$rustc_version" in "rustc ${RUST_TOOLCHAIN} "*)' in builder
    assert "--default-toolchain stable" not in builder and "rustup.rs" not in builder

    bootstrap = _read("bootstrap.sh")
    assert '--default-toolchain "$CAPSEM_RUST_TOOLCHAIN"' in bootstrap
    assert 'capsem_rust_toolchain "$SCRIPT_DIR/rust-toolchain.toml"' in bootstrap
    assert f"--default-toolchain {PINNED_RUST}" not in bootstrap
    assert "--default-toolchain stable" not in bootstrap

    uses_pattern = re.compile(r"^\s*- uses:\s+([^\s#]+)", re.MULTILINE)
    upload_refs: set[str] = set()
    failures: list[str] = []

    for path in WORKFLOWS.glob("*.yaml"):
        text = path.read_text(encoding="utf-8")
        for use in uses_pattern.findall(text):
            if use.startswith("./"):
                continue
            action, separator, ref = use.partition("@")
            if separator != "@" or re.fullmatch(r"[0-9a-f]{40}", ref) is None:
                failures.append(f"{path.name}: {use}")
            if action == "actions/upload-artifact":
                upload_refs.add(ref)

    assert failures == []
    assert len(upload_refs) == 1

    security_audit = _read(".github/workflows/security-audit.yaml")
    assert "schedule:" in security_audit
    assert "cron:" in security_audit
    assert "workflow_dispatch:" in security_audit
    assert "run: python3 build_system/scripts/audit/check-cargo-audit.py" in security_audit
    assert "run: python3 build_system/scripts/audit/audit-pnpm-bulk.py" in security_audit


def test_host_builder_base_images_are_immutable() -> None:
    """A sealed rebuild must resolve exact bytes, not refresh mutable tags."""
    from capsem_builder.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    builder = _read("build_system/docker/Dockerfile.host-builder")
    bases = [line.split()[1] for line in builder.splitlines() if line.startswith("FROM ")]
    dynamic = {"${RUST_IMAGE}", "${UV_IMAGE}"}

    assert bases
    assert {base for base in bases if base.startswith("${")} == dynamic
    assert all("@sha256:" in base for base in bases if base not in dynamic), bases
    assert all(
        re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image)
        for image in (config.hostimage.rust_image, config.hostimage.uv_image)
    )


def test_every_guest_builder_base_is_an_exact_platform_child_manifest() -> None:
    """A mutable tag lets a warm host and a cold release runner build different bytes."""
    from capsem_builder.gate import config as gate_config
    from capsem_builder.image.config import load_guest_config

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    build = load_guest_config(root / config.imagebuild.source_config).build
    refs = {
        image
        for arch in build.architectures.values()
        for image in (arch.base_image, arch.rust_builder_base_image)
    }

    assert len(refs) == 2 * len(build.architectures)
    for arch in build.architectures.values():
        for image in (arch.base_image, arch.rust_builder_base_image):
            repository, digest = image.rsplit("@sha256:", 1)
            assert repository
            assert len(digest) == 64
            assert digest == digest.lower()


def test_host_builder_trusts_the_bind_mounted_source_checkout() -> None:
    """On Linux CI the checkout's owner is not the image's user, so git rejects
    the mount as dubious ownership -- and `build.rs` answers that by embedding
    `unknown` rather than failing, which is how a binary with no source identity
    reaches the provenance check.

    The mount path is config now, and the probe that reproduces the condition
    is a step rather than a hope.
    """
    from capsem_builder.gate import config as gate_config

    root = Path(__file__).resolve().parents[3]
    config = gate_config.load(root)
    builder = _read(config.hostimage.dockerfile)

    assert f"git config --system --add safe.directory {config.hostimage.mount}" in builder
    # Was two asserts on another module's *source text*. Those break on every
    # behaviour-preserving refactor and pass on behaviour-changing ones, which
    # is the wrong way round -- six of them broke at once earlier in this work.
    # The probe they named existed so a container reading a bind-mounted
    # checkout would not embed an "unknown" build hash; no lane mounts the
    # checkout now, so the property is that the revision arrives as a declared
    # input, asserted against the value rather than the text.
    assert config.environment.package.build_revision == "CAPSEM_BUILD_REVISION"
    assert config.hostimage.mount == "/src"


def test_remote_storage_images_are_immutable() -> None:
    policy = tomllib.loads(_read("config/storage-policy.toml"))
    floating: list[str] = []

    def visit(value: object) -> None:
        if isinstance(value, dict):
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)
        elif (
            isinstance(value, str) and value.endswith(":latest") and not value.startswith("capsem-")
        ):
            floating.append(value)

    visit(policy)
    assert floating == []

    tart_image = policy["tart"]["base_image"]
    assert re.fullmatch(
        r"ghcr\.io/cirruslabs/macos-sequoia-base@sha256:[0-9a-f]{64}",
        tart_image,
    )


def test_public_release_storage_is_verified_before_channel_deployment() -> None:
    workflow = _read(".github/workflows/release.yaml")
    create = _job_block(workflow, "create-release")
    candidate = _job_block(workflow, "verify-release-candidate")
    deploy = _job_block(workflow, "deploy-release-channel")
    public = _job_block(workflow, "verify-release-downloads")
    candidate_proof = _read("scripts/prove-candidate-installer.sh")

    assert "scripts/publish-immutable-release-assets.sh" in create
    assert "gh release create" not in create
    assert "--draft" not in create
    assert "needs: [create-release, assemble-release-channel]" in candidate
    assert "binary-channel-preview" in candidate
    assert "https://capsem.org/install.sh" in candidate_proof
    assert "CAPSEM_MANIFEST_URL" in candidate_proof
    assert "github.com/${{ github.repository }}/releases/download" in candidate
    assert "scripts/release-package-contract.py verify-storage" in candidate
    assert "scripts/prove-candidate-installer.sh" in candidate
    assert "needs: [verify-release-candidate]" in deploy
    assert "needs: [deploy-release-channel]" in public


def test_assembly_recovery_reuses_qualified_artifacts_and_keeps_public_proof() -> None:
    workflow = _read(".github/workflows/release-publication-recovery.yaml")
    recover = _job_block(workflow, "recover-release-channel")
    deploy = _job_block(workflow, "deploy-release-channel")
    public = _job_block(workflow, "verify-release-downloads")
    candidate_proof = _read("scripts/prove-candidate-installer.sh")

    assert "workflow_dispatch:" in workflow
    assert "group: capsem-release-${{ inputs.channel }}" in workflow
    assert "--failed-job assemble-release-channel" in recover
    assert "binary-channel-candidate" in recover
    assert "run-id: ${{ inputs.failed_run_id }}" in recover
    assert "scripts/build-complete-release-channel.py" in recover
    assert "CAPSEM_MANIFEST_URL" in candidate_proof
    assert "scripts/publish-immutable-release-assets.sh" in recover
    assert "binary-channel-preview" in recover
    assert "binary-channel-before" in recover
    assert "release-binaries" not in workflow
    assert "qualify-binaries" not in workflow

    assert "needs: [recover-release-channel]" in deploy
    assert "uses: ./.github/workflows/release-channel.yaml" in deploy
    assert "source_commit: ${{ github.sha }}" in deploy

    assert "needs: [deploy-release-channel]" in public
    assert "scripts/verify-channel-downloads.py" in public
    assert "scripts/check-public-binary-release.py" in public
    assert "Enable KVM for live public-install VM proof" in public
    assert "scripts/prove-live-public-install.sh" in public
    live_proof = _read("scripts/prove-live-public-install.sh")
    assert "CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK" in live_proof

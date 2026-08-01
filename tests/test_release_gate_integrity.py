"""Release gate identity, toolchain, and publication-order contracts."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
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


def test_just_test_holds_source_state_stable_without_archiving_benchmarks() -> None:
    """Both HEAD and the working-tree digest are captured before and compared
    after: a gate that qualified a HEAD nobody has, or a tree edited halfway
    through, proved nothing about any particular version."""
    import argparse

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate import config as gate_config
    from capsem.gate.command import GateCommand
    from capsem.gate.proc import Runner

    root = Path(__file__).resolve().parents[1]
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
    held = {resource.name for resource in command.resources()}
    assert {"colima", "orphan-accounting", "failure-evidence"} <= held

    assert "CAPSEM_BENCHMARK_OUTPUT_ROOT" in _read("src/capsem/gate/workspace.py")
    assert config.workspace.benchmark_root == "target/test-benchmarks"
    assert "benchmarks/**/data_*.json" in _read(".gitignore")


def test_gate_run_retains_the_vm_performance_recordings_it_produces() -> None:
    """`functional` writes the VM performance recordings and `glowup` runs after
    it. While the wipe lived in the per-module runner, the later module deleted
    the earlier one's numbers, so a full gate produced a complete set and then
    destroyed it -- a fortnight of green runs left target/test-benchmarks empty
    and froze the published arm64 history at 1.3. Clearing belongs to the gate
    run, once, not to each module inside it."""
    justfile = _read("justfile")
    runner = justfile.split("\n_test-candidate-run:", maxsplit=1)[1].split(
        "\n\n_", maxsplit=1
    )[0]

    assert 'rm -rf "$CAPSEM_BENCHMARK_OUTPUT_ROOT"' not in runner

    # The composition recipe clears it once, before the module sequence runs.
    candidate = justfile.split("\n_test-candidate:", maxsplit=1)[1].split(
        "\n_test-candidate-run:", maxsplit=1
    )[0]
    clear = candidate.index('rm -rf "{{justfile_directory()}}/target/test-benchmarks"')
    assert clear < candidate.index("just _test-functional")
    assert clear < candidate.index("just _test-glowup")


def test_full_gate_runs_capsem_bench_baseline_for_every_selected_profile() -> None:
    justfile = _read("justfile")
    candidate = justfile.split("\n_test-candidate:", maxsplit=1)[1].split(
        "\n_build-host-image:", maxsplit=1
    )[0]
    base_profile, remaining_profiles = candidate.split(
        'for TEST_PROFILE in "${TEST_PROFILES[@]:1}"; do',
        maxsplit=1,
    )
    benchmark = "tests/capsem-serial/test_capsem_bench_baseline.py"

    assert base_profile.count(benchmark) == 1
    assert remaining_profiles.count(benchmark) == 1
    assert 'CAPSEM_TEST_PROFILE="$BASE_PROFILE"' in base_profile
    assert 'CAPSEM_TEST_PROFILE="$TEST_PROFILE"' in remaining_profiles


def test_full_gate_serializes_host_snapshot_files_without_dropping_coverage() -> None:
    justfile = _read("justfile")
    candidate = justfile.split("\n_test-candidate:", maxsplit=1)[1].split(
        "\n_build-host-image:", maxsplit=1
    )[0]
    snapshot_files = (
        "tests/capsem-mcp/test_state_transitions.py",
        "tests/capsem-service/test_svc_resume_paths.py",
        "tests/capsem-service/test_svc_suspend_corruption.py",
        "tests/capsem-service/test_svc_loop_device_after_resume.py",
    )

    declaration = candidate.split("HOST_SNAPSHOT_SERIAL=(", maxsplit=1)[1].split(
        ")", maxsplit=1
    )[0]
    for path in snapshot_files:
        assert f'"{path}"' in declaration
        assert candidate.count(path) == 1

    parallel = candidate.index("=== Python: non-serial tests (n=4 parallel) ===")
    serial = candidate.index("=== Python: host snapshot tests (serial) ===")
    timing = candidate.index("=== Python: serial timing and benchmark tests ===")
    assert parallel < serial < timing
    assert '"${HOST_SNAPSHOT_IGNORE_ARGS[@]}"' in candidate[parallel:serial]
    assert "--maxfail=1" in candidate[parallel:serial]
    assert '"${HOST_SNAPSHOT_SERIAL[@]}"' in candidate[serial:timing]


def test_local_gate_bootstraps_docker_before_storage_preflight() -> None:
    justfile = _read("justfile")
    candidate = justfile.split("\n_test-candidate:", maxsplit=1)[1].split(
        "\n_test-fast:", maxsplit=1
    )[0]

    assert candidate.index("just _bootstrap") < candidate.index(
        "just _bound-docker-test-storage"
    )


def test_macos_full_gate_holds_a_system_sleep_assertion() -> None:
    """A forty-minute run that dies at minute thirty because the machine slept
    proves nothing, and by then it is usually unattended."""
    from capsem.gate import config as gate_config

    settings = gate_config.load(PROJECT_ROOT).candidate

    assert settings.keep_awake_command[0] == "caffeinate"
    assert settings.keep_awake_marker == "CAPSEM_TEST_CAFFEINATED"
    assert "keep_awake" in _read("src/capsem/gate/candidate.py")


def test_toolchain_and_workflow_inputs_are_immutable_and_consistent() -> None:
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
        tools = tool_line.split("tool:", maxsplit=1)[1].strip().split(",")
        assert all("@" in tool for tool in tools)

    builder = _read("docker/Dockerfile.host-builder")
    assert f"--default-toolchain {PINNED_RUST}" in builder
    assert "--default-toolchain stable" not in builder

    bootstrap = _read("bootstrap.sh")
    assert f"--default-toolchain {PINNED_RUST}" in bootstrap
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
    assert "run: python3 scripts/check-cargo-audit.py" in security_audit
    assert "run: python3 scripts/audit-pnpm-bulk.py" in security_audit


def test_host_builder_trusts_the_bind_mounted_source_checkout() -> None:
    """/src is a bind mount of the host checkout, so on Linux its owner is not
    the container user and git rejects it as "dubious ownership". That failure
    is quiet where it matters: crates/capsem/build.rs embeds "unknown" for the
    build hash rather than failing, so without this the only thing between a
    provenance-less binary and a release is check-build-provenance.sh."""
    builder = _read("docker/Dockerfile.host-builder")

    assert "git config --system --add safe.directory /src" in builder
    assert "/src" in _read("justfile"), "the builder still bind-mounts /src"


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
            isinstance(value, str)
            and value.endswith(":latest")
            and not value.startswith("capsem-")
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

    assert "scripts/publish-immutable-release-assets.sh" in create
    assert "gh release create" not in create
    assert "--draft" not in create
    assert "needs: [create-release, assemble-release-channel]" in candidate
    assert "binary-channel-preview" in candidate
    assert "https://capsem.org/install.sh" in candidate
    assert "CAPSEM_MANIFEST_URL" in candidate
    assert "github.com/${{ github.repository }}/releases/download" in candidate
    assert "b3sum -c -" in candidate
    assert "needs: [verify-release-candidate]" in deploy
    assert "needs: [deploy-release-channel]" in public

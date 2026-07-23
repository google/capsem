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


def test_just_test_binds_clean_tree_to_one_commit_without_archiving_benchmarks() -> None:
    justfile = _read("justfile")
    wrapper = justfile.split("\ntest:", maxsplit=1)[1].split("\n_test-candidate:", maxsplit=1)[0]

    assert "git status --porcelain" in wrapper
    assert "TESTED_HEAD=$(git rev-parse HEAD)" in wrapper
    assert 'test "$(git rev-parse HEAD)" = "$TESTED_HEAD"' in wrapper
    assert "scripts/with-gate-colima.sh just _test-candidate" in wrapper
    assert "CAPSEM_BENCHMARK_OUTPUT_ROOT" in justfile
    assert "target/test-benchmarks" in justfile
    assert "benchmarks/**/data_*.json" in _read(".gitignore")


def test_full_gate_runs_capsem_bench_baseline_exactly_once() -> None:
    justfile = _read("justfile")
    candidate = justfile.split("\n_test-candidate:", maxsplit=1)[1].split(
        "\n_build-host-image:", maxsplit=1
    )[0]

    assert candidate.count("tests/capsem-serial/test_capsem_bench_baseline.py") == 1


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
    dependency_line = next(
        line for line in justfile.splitlines() if line.startswith("_test-candidate:")
    )

    assert dependency_line.index("_bootstrap") < dependency_line.index(
        "_bound-docker-test-storage"
    )


def test_macos_full_gate_holds_a_system_sleep_assertion() -> None:
    justfile = _read("justfile")
    wrapper = justfile.split("\ntest:", maxsplit=1)[1].split(
        "\n_test-candidate:", maxsplit=1
    )[0]

    assert "caffeinate" in wrapper
    assert "CAPSEM_TEST_CAFFEINATED" in wrapper


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
    assert "run: cargo audit" in security_audit
    assert "run: python3 scripts/audit-pnpm-bulk.py --project-dir frontend" in security_audit


def test_public_release_storage_is_verified_before_channel_deployment() -> None:
    workflow = _read(".github/workflows/release.yaml")
    create = _job_block(workflow, "create-release")
    candidate = _job_block(workflow, "verify-release-candidate")
    deploy = _job_block(workflow, "deploy-release-channel")
    public = _job_block(workflow, "verify-release-downloads")

    assert 'gh release create "$RELEASE_TAG"' in create
    assert "--draft" not in create
    assert "needs: [create-release, assemble-release-channel]" in candidate
    assert "binary-channel-preview" in candidate
    assert "https://capsem.org/install.sh" in candidate
    assert "CAPSEM_MANIFEST_URL" in candidate
    assert "github.com/${{ github.repository }}/releases/download" in candidate
    assert "b3sum -c -" in candidate
    assert "needs: [verify-release-candidate]" in deploy
    assert "needs: [deploy-release-channel]" in public

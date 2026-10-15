"""Cold CI must build the production capabilities its tests consume.

Local qualification already built native workers and the source runtime.
Hosted unit tests once discovered no MCP executable, and install CI demanded
that stable already publish the first runtime that this candidate introduces.
"""

from pathlib import Path

from helpers.workflow_contract import parsed_commands, workflow_jobs

ROOT = Path(__file__).resolve().parents[2]
WORKERS = {"capsem-mcp-aggregator", "capsem-mcp-builtin"}


def native_builds(source: str) -> set[str]:
    return {
        command.argv[index + 1]
        for command in parsed_commands(source, origin="native CI prerequisite")
        if command.argv[:3] == ("cargo", "llvm-cov", "run")
        and "--no-report" in command.argv
        and command.argv[-2:] == ("--", "--version")
        for index, argument in enumerate(command.argv[:-1])
        if argument == "-p"
    }


def test_macos_builds_real_mcp_workers_before_unit_coverage() -> None:
    steps = workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test"]["steps"]
    coverage = next(index for index, step in enumerate(steps) if step.get("name") == "Unit tests with coverage")
    source = "\n".join(step.get("run", "") for step in steps[:coverage])
    assert native_builds(source) >= WORKERS, "macOS unit tests require actual native MCP executables first"


def test_linux_builds_real_mcp_workers_before_unit_coverage() -> None:
    source = (ROOT / "build_system/scripts/test/test-linux-rust.sh").read_text()
    preparation, _ = source.split("cargo llvm-cov nextest", 1)
    assert native_builds(preparation) >= WORKERS, "Linux unit tests require actual native MCP executables first"


def test_source_install_ci_qualifies_its_runtime_without_a_prior_release() -> None:
    job = workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]
    commands = [
        command.argv
        for step in job["steps"]
        for command in parsed_commands(step.get("run", ""), origin="source install CI")
    ]
    assert any("capsem-gate" in argv and "test-glowup" in argv and "--slow" in argv for argv in commands), (
        "source install CI must construct and prove its current runtime through the canonical owner; "
        "the first runtime cannot depend on already being publicly released"
    )
    assert not any(step.get("uses") == "./.github/actions/fetch-release-inputs" for step in job["steps"])


def test_coverage_cleans_before_preparing_workers_and_preserves_them_for_reporting() -> None:
    mac_steps = workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test"]["steps"]
    for source in (
        "\n".join(step.get("run", "") for step in mac_steps),
        (ROOT / "build_system/scripts/test/test-linux-rust.sh").read_text(),
    ):
        commands = [command.argv for command in parsed_commands(source, origin="coverage cohort lifecycle")]
        clean = [i for i, argv in enumerate(commands) if argv[:3] == ("cargo", "llvm-cov", "clean")]
        workers = [i for i, argv in enumerate(commands) if argv[:3] == ("cargo", "llvm-cov", "run")]
        report = next(i for i, argv in enumerate(commands) if argv[:3] == ("cargo", "llvm-cov", "nextest"))
        assert len(clean) == 1 and clean[0] < min(workers) <= max(workers) < report, (
            "start one clean coverage cohort before native worker preparation"
        )
        assert "--workspace" in commands[clean[0]]
        assert "--no-clean" in commands[report], "reporting must not delete its prepared native workers"

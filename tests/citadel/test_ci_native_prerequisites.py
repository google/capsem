"""Cold CI must build the production capabilities its tests consume.

Local qualification already built native workers and the source runtime.
Hosted unit tests once discovered no MCP executable, and install CI demanded
that stable already publish the first runtime that this candidate introduces.
"""

from copy import deepcopy
from pathlib import Path

import pytest
from helpers.workflow_contract import parsed_commands, workflow_jobs

ROOT = Path(__file__).resolve().parents[2]
WORKERS = {"capsem-mcp-aggregator", "capsem-mcp-builtin"}
INSTALL_BINFMT_RATIONALE = (
    "The complete install owner builds both architectures. Provision distro QEMU/binfmt "
    "through the existing host bootstrap before entering the gate; CI otherwise fails "
    "assets.preflight after spending twenty minutes building the install environment."
)
PREBUILT_BOOTSTRAP_RATIONALE = (
    "Install the declared prebuilt tools before the complete host bootstrap. "
    "CI previously spent 18 minutes source-building the same pinned Cargo tools "
    "that a later step downloaded in 6 seconds. Keep bootstrap's host prerequisites."
)


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


def _assert_install_bootstrap(steps) -> None:
    proof = next(index for index, step in enumerate(steps) if step.get("id") == "install_e2e")
    bootstraps = [
        step for step in steps[:proof]
        if any(
            command.argv == ("sh", "bootstrap.sh", "--yes")
            for command in parsed_commands(step.get("run", ""), origin="install host bootstrap")
        )
    ]
    assert len(bootstraps) == 1, INSTALL_BINFMT_RATIONALE
    bootstrap = bootstraps[0]
    assert "if" not in bootstrap and not bootstrap.get("continue-on-error"), INSTALL_BINFMT_RATIONALE
    assert bootstrap.get("env", {}).get("CAPSEM_SKIP_ASSET_CHECK") == "1", (
        "Bootstrap owns host prerequisites; the following test-glowup owns asset construction."
    )


def test_source_install_ci_bootstraps_cross_arch_host_before_complete_proof() -> None:
    _assert_install_bootstrap(workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]["steps"])


def _assert_prebuilt_before_bootstrap(steps) -> None:
    select = next(index for index, step in enumerate(steps) if step.get("id") == "gate_tools")
    install = next(index for index, step in enumerate(steps)
                   if step.get("with", {}).get("tool") == "${{ steps.gate_tools.outputs.list }}")
    bootstrap = next(index for index, step in enumerate(steps)
                     if step.get("run") == "sh bootstrap.sh --yes")
    assert select < install < bootstrap, PREBUILT_BOOTSTRAP_RATIONALE
    for step in (steps[select], steps[install]):
        assert "if" not in step and not step.get("continue-on-error"), PREBUILT_BOOTSTRAP_RATIONALE
    assert steps[install]["uses"].startswith("taiki-e/install-action@"), PREBUILT_BOOTSTRAP_RATIONALE
    _assert_install_bootstrap(steps)


def test_source_install_ci_uses_prebuilt_tools_before_bootstrap() -> None:
    _assert_prebuilt_before_bootstrap(workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]["steps"])


@pytest.mark.parametrize("mutation", ["tools-late", "select-late", "tools-conditional", "select-conditional", "ignored-failure"])
def test_prebuilt_bootstrap_guard_refuses_disabled_or_late_tools(mutation: str) -> None:
    steps = deepcopy(workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]["steps"])
    _assert_prebuilt_before_bootstrap(steps)
    select = next(step for step in steps if step.get("id") == "gate_tools")
    install = next(step for step in steps if step.get("with", {}).get("tool") == "${{ steps.gate_tools.outputs.list }}")
    if mutation == "tools-late":
        steps.remove(install)
        steps.append(install)
    elif mutation == "select-late":
        steps.remove(select)
        steps.append(select)
    elif mutation == "tools-conditional":
        install["if"] = "${{ false }}"
    elif mutation == "select-conditional":
        select["if"] = "${{ false }}"
    else:
        install["continue-on-error"] = True
    with pytest.raises(AssertionError, match="prebuilt tools"):
        _assert_prebuilt_before_bootstrap(steps)


@pytest.mark.parametrize("mutation", ["missing", "late", "conditional", "duplicate-assets"])
def test_install_bootstrap_guard_refuses_broken_order_or_ownership(mutation: str) -> None:
    steps = deepcopy(workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]["steps"])
    index = next(index for index, step in enumerate(steps) if step.get("run") == "sh bootstrap.sh --yes")
    if mutation == "missing":
        steps.pop(index)
    elif mutation == "late":
        steps.append(steps.pop(index))
    elif mutation == "conditional":
        steps[index]["if"] = "${{ false }}"
    else:
        steps[index]["env"]["CAPSEM_SKIP_ASSET_CHECK"] = "0"
    with pytest.raises(AssertionError):
        _assert_install_bootstrap(steps)


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

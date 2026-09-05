"""Would the gate work if we started now?

One Python package with one console script is easy to say and easy to have
wrong. `uv sync --project build_system` can succeed while the entry point resolves to a stale wheel, a
storage phase can name a rail the policy no longer declares, and a recipe can
dispatch to a subcommand that was renamed. Each of those surfaces deep inside a
run and reads as a product defect rather than an installation one.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import doctor, sandbox
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def _linux_platform(*, policy: str | None, interfaces: str = "eth0 lo") -> list[str]:
    """Exercise Doctor's shell contract independently of the current host."""
    script = r"""
set -eu
source build_system/scripts/doctor/doctor-linux.sh
section() { :; }
pass() { printf 'PASS:%s\n' "$1"; }
fail() { printf 'FAIL:%s\n' "$1"; }
warn() { printf 'WARN:%s\n' "$1"; }
skip() { printf 'SKIP:%s\n' "$1"; }
bwrap() { return 0; }
capsem_linux_network_interfaces() { printf '%s\n' ${CAPSEM_TEST_INTERFACES}; }
check_platform
"""
    environment = {
        **os.environ,
        "CAPSEM_SKIP_KVM_CHECK": "1",
        "CAPSEM_TEST_INTERFACES": interfaces,
    }
    variable = CONFIG.environment.command_sandbox_mode
    environment.pop(variable, None)
    if policy is not None:
        environment[variable] = policy
    completed = subprocess.run(
        ["bash", "-c", script],
        cwd=PROJECT_ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=True,
    )
    return completed.stdout.splitlines()


@pytest.mark.parametrize("policy", [None, sandbox.OFF.value])
def test_linux_doctor_probes_bubblewrap_for_network_open_commands(policy: str | None) -> None:
    lines = _linux_platform(policy=policy)

    assert "PASS:Bubblewrap gate network namespace and device mount" in lines
    assert not any("network namespace active" in line for line in lines)


def test_linux_doctor_enforce_policy_proves_the_live_kernel_boundary() -> None:
    good = _linux_platform(policy=sandbox.ENFORCE.value, interfaces="lo")
    escaped = _linux_platform(policy=sandbox.ENFORCE.value)

    assert "PASS:Bubblewrap gate network namespace active (loopback only)" in good
    assert any(line.startswith("FAIL:enforcing gate sandbox sees interfaces:") for line in escaped)


def test_linux_doctor_refuses_report_and_unknown_gate_policy() -> None:
    assert "FAIL:Linux gate sandbox report mode is unsupported" in _linux_platform(
        policy=sandbox.REPORT.value
    )
    assert "FAIL:unknown gate sandbox policy: forged" in _linux_platform(policy="forged")


def test_gate_lock_marker_alone_never_claims_kernel_enforcement(monkeypatch) -> None:
    monkeypatch.setenv(CONFIG.locks.gate.run_marker, "capsem-gate build-assets")

    lines = _linux_platform(policy=None)

    assert "PASS:Bubblewrap gate network namespace and device mount" in lines
    assert not any("network namespace active" in line for line in lines)


def test_bootstrap_and_doctor_keep_lock_ownership_separate_from_sandbox_policy() -> None:
    linux = (PROJECT_ROOT / "build_system/scripts/doctor/doctor-linux.sh").read_text(
        encoding="utf-8"
    )
    common = (PROJECT_ROOT / "build_system/scripts/doctor/doctor-common.sh").read_text(
        encoding="utf-8"
    )
    bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text(encoding="utf-8")
    variable = CONFIG.environment.command_sandbox_mode

    assert variable == "CAPSEM_GATE_COMMAND_SANDBOX_MODE"
    assert variable in linux
    assert CONFIG.locks.gate.run_marker not in linux
    assert CONFIG.locks.gate.run_marker in common
    assert CONFIG.locks.gate.run_marker in bootstrap


def test_macos_doctor_does_not_interpret_the_linux_command_policy() -> None:
    macos = (PROJECT_ROOT / "build_system/scripts/doctor/doctor-macos.sh").read_text(
        encoding="utf-8"
    )

    assert CONFIG.environment.command_sandbox_mode not in macos
    assert "CAPSEM_SKIP_TART_CHECK" in macos
    assert CONFIG.candidate.doctor_skips["CAPSEM_SKIP_TART_CHECK"] == "1"
    assert "CAPSEM_SKIP_TART_CHECK" not in CONFIG.imagebuild.doctor_skips


def _checkout(tmp_path: Path, *, gate_toml: str | None = None) -> Path:
    (tmp_path / "justfile").write_text((PROJECT_ROOT / "justfile").read_text(encoding="utf-8"))
    (tmp_path / "build_system").mkdir(exist_ok=True)
    (tmp_path / "build_system/pyproject.toml").write_text(
        (PROJECT_ROOT / "build_system/pyproject.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "config").mkdir(exist_ok=True)
    (tmp_path / "config" / "gate.toml").write_text(
        gate_toml or (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "config" / "cache.toml").write_text(
        (PROJECT_ROOT / "config" / "cache.toml").read_text(encoding="utf-8")
    )
    return tmp_path


def test_this_checkout_is_ready() -> None:
    """The real thing, which is the only assertion that matters day to day."""
    assert doctor.check(RecordingRunner(PROJECT_ROOT)) == []


def test_an_invalid_cache_control_is_reported(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    policy = root / "config/cache.toml"
    policy.write_text(
        policy.read_text(encoding="utf-8").replace(
            "max_size_bytes = 193273528320 # 180 GiB",
            "max_size_bytes = 0 # invalid",
            1,
        ),
        encoding="utf-8",
    )

    findings = doctor.check(RecordingRunner(root))

    assert [finding.check for finding in findings] == ["cache policy"]
    assert "max_size_bytes" in findings[0].detail


def test_a_recipe_dispatching_to_an_unknown_subcommand_is_reported(
    tmp_path: Path,
) -> None:
    """A renamed subcommand leaves the recipe failing on an argparse error,
    which reads as a recipe defect with no obvious cause."""
    root = _checkout(tmp_path)
    justfile = root / "justfile"
    justfile.write_text(
        justfile.read_text(encoding="utf-8").replace(
            "capsem-gate stamp-version", "capsem-gate stamp-the-version"
        )
    )

    findings = doctor.check(RecordingRunner(root))

    assert any("stamp-the-version" in finding.check for finding in findings)
    assert any("not a subcommand" in finding.detail for finding in findings)


def test_a_missing_cache_policy_is_reported_rather_than_raised(
    tmp_path: Path,
) -> None:
    root = _checkout(tmp_path)
    (root / "config" / "cache.toml").unlink()

    findings = doctor.check(RecordingRunner(root))

    assert [finding.check for finding in findings] == ["cache policy"]


def test_the_command_names_every_problem_at_once(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Reporting one at a time turns setup into a guessing game."""
    monkeypatch.setattr(
        doctor,
        "check",
        lambda _runner: [
            doctor.Finding("first", "one thing"),
            doctor.Finding("second", "another"),
        ],
    )

    with pytest.raises(GateError) as failure:
        doctor.report(Context(RecordingRunner(PROJECT_ROOT), gate_config.load(PROJECT_ROOT)))

    assert "one thing" in str(failure.value)
    assert "another" in str(failure.value)


def test_a_ready_gate_says_so(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(doctor, "check", lambda _runner: [])
    runner = RecordingRunner(PROJECT_ROOT)

    doctor.report(Context(runner, gate_config.load(PROJECT_ROOT)))
    assert any("configuration valid" in note for note in runner.notes)


def test_every_declared_console_script_is_runnable() -> None:
    """`uv sync --project build_system` succeeding is not the same as the entry points working.

    Run rather than read. The name it resolves to moved once already -- to
    `capsem_builder.gatelaunch:main`, which re-execs under an isolated bytecode cache
    before importing the package -- and a string comparison would have been
    green for a launcher that never reached the CLI at the other end.
    """
    import tomllib

    declared = tomllib.loads(
        (PROJECT_ROOT / "build_system/pyproject.toml").read_text(encoding="utf-8")
    )["project"]["scripts"]
    assert set(declared) == {"capsem-builder", "capsem-cache", "capsem-gate"}

    results = {
        executable: subprocess.run(
            ["uv", "run", "--project", "build_system", "--frozen", executable, "--help"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            timeout=180,
        )
        for executable in declared
    }

    expected_help = {
        "capsem-builder": "Capsem builder -- backend helper tooling.",
        "capsem-cache": "Inspect and control Capsem's repository cache.",
        "capsem-gate": "candidate",
    }
    for executable, result in results.items():
        assert result.returncode == 0, result.stderr
        assert expected_help[executable] in result.stdout
    # It got past the launcher: the subcommands only exist once `capsem_builder.gate`
    # has been imported, which happens on the far side of the re-exec.
    assert "candidate" in results["capsem-gate"].stdout


def test_the_justfile_dispatches_to_the_gate_rather_than_reimplementing_it() -> None:
    """The whole point of the boundary: recipes call, they do not decide.

    The extraction ratchet is gone because the extraction finished. What
    replaces it is stronger and unconditional -- no recipe carries a shell
    body at all, held by `build_system/tests/gate/test_gate_boundary.py`.
    """
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    config = gate_config.load(PROJECT_ROOT)

    assert "uv run --project build_system --frozen capsem-gate" in justfile
    assert config.boundary.max_recipe_lines <= 5
    assert not config.boundary.recipes_with_inline_control_flow


# ---------------------------------------------------------------------------
# The source gates themselves
# ---------------------------------------------------------------------------


def _lint_plan(tmp_path: Path):
    """The `lint` command's plan, against a throwaway checkout.

    Read from the plan rather than by calling a function that ran the tools in
    sequence: they are three steps now, which is the point -- a graph can
    schedule, time and name them apart, and it aggregates their failures
    instead of a hand-written list doing it once out of sight.
    """
    import argparse
    import importlib

    from capsem_builder.gate.command import GateCommand

    importlib.import_module("capsem_builder.gate.cli")
    root = _checkout(tmp_path)
    for name in gate_config.load(root).lint.python_roots:
        (root / name).mkdir(parents=True, exist_ok=True)
    return GateCommand.registry["lint"](
        RecordingRunner(root),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()


def test_lint_runs_ruff_and_both_ty_passes(tmp_path: Path) -> None:
    """Ty used to omit release machinery from its checked source owners."""
    plan = _lint_plan(tmp_path)
    described = plan.describe()

    assert "ruff check --config build_system/pyproject.toml ." in described
    strict = " ".join(plan.step_named("python.ty.strict").render())
    relaxed = " ".join(plan.step_named("python.ty.relaxed").render())

    assert "build_system/builder" in strict
    assert "--ignore" in relaxed
    assert "--ignore" not in strict, (
        "build_system/builder passes every rule and must be checked with none held back"
    )


def test_lint_reports_every_failing_gate_not_just_the_first(tmp_path: Path) -> None:
    """Stopping at the first tool leaves the second's findings for the next
    push, which is how a gate takes three rounds to go green."""
    from capsem_builder.gate.context import Context
    from helpers.gate import RecordingJournal

    root = _checkout(tmp_path)
    for name in gate_config.load(root).lint.python_roots:
        (root / name).mkdir(parents=True, exist_ok=True)
    runner = RecordingRunner(root, failures=["ruff check", "ty check"])

    with pytest.raises(GateError) as failure:
        _lint_plan(tmp_path).run(
            Context(runner, gate_config.load(root), journal=RecordingJournal())
        )

    assert "python.ruff" in str(failure.value)
    assert "python.ty" in str(failure.value)


def test_lint_warnings_fail_the_gate(tmp_path: Path) -> None:
    """A ty warning exits zero, so a warning-level rule on the ratchet could
    never have been detected as fixed."""
    plan = _lint_plan(tmp_path)

    checks = [
        line
        for label in plan.labels
        if label.startswith("python.ty")
        for line in plan.step_named(label).render()
    ]
    assert checks
    assert all("--error-on-warning" in line for line in checks)


def test_a_subcommand_named_in_a_comment_is_not_a_dispatch(tmp_path: Path) -> None:
    """Prose about a command is not a call to it.

    The check reads every line containing `capsem-gate `, and a justfile
    comment explaining which command names a recipe --

        # `capsem-gate linux-rust` names this recipe when the image is missing.

    -- was parsed as a dispatch of ``linux-rust` ``, trailing backtick and all,
    then reported as an unknown subcommand. Three doctor tests went red on a
    comment.

    A commented-out dispatch is also not a dispatch, so skipping the line loses
    nothing the check was protecting.
    """
    root = _checkout(tmp_path)
    justfile = root / "justfile"
    justfile.write_text(
        justfile.read_text(encoding="utf-8")
        + "\n# `capsem-gate not-a-real-subcommand` is only mentioned here.\n"
        + "#     uv run --project build_system --frozen capsem-gate also-not-real\n",
        encoding="utf-8",
    )

    findings = doctor.check(RecordingRunner(root))
    assert [f for f in findings if "dispatch" in f.check] == [], findings

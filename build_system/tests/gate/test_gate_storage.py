"""Storage release boundaries, named once instead of spelled eleven times."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.sourcecommit import SourceCommit
from capsem_builder.gate.storage import Storage
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
PHASES = gate_config.load(PROJECT_ROOT).storage.phases


def test_a_release_phase_names_its_boundary_and_rail() -> None:
    runner = RecordingRunner(PROJECT_ROOT)

    Storage(runner).release("completed-docker-rails")

    assert runner.matching(
        r"docker-storage-policy.py release --boundary after-assets --rail package"
    )


def test_an_unknown_phase_fails_by_name() -> None:
    """Eleven recipes spelled these pairs by hand, so the legal set was a habit."""
    runner = RecordingRunner(PROJECT_ROOT)

    with pytest.raises(GateError) as failure:
        Storage(runner).release("after-everything")

    assert "after-everything" in str(failure.value)
    assert "completed-docker-rails" in str(failure.value)


def test_capture_failure_never_fails_the_run_further() -> None:
    """It runs on the failure path; a second failure would replace the first."""
    runner = RecordingRunner(PROJECT_ROOT, failures=["capture-failure"])

    Storage(runner).capture_failure(
        rail="default",
        label="abcdef123456",
        run_id="20260813-010203-abcdef-release-binaries",
        source_commit=SourceCommit("1" * 40),
    )

    assert runner.matching(
        r"capture-failure --rail default --label abcdef123456 "
        r"--run-id 20260813-010203-abcdef-release-binaries --source-commit 1{40}"
    )


def test_ensure_space_passes_the_optional_boundary_through() -> None:
    runner = RecordingRunner(PROJECT_ROOT)

    Storage(runner).ensure_space("default", "candidate-boundary")

    assert runner.rendered[0].endswith("ensure-docker-space.sh default candidate-boundary")


# ---------------------------------------------------------------------------
# Agreement with the script being driven
# ---------------------------------------------------------------------------


def _policy_parser():
    """The real parser from `scripts/docker-storage-policy.py`."""
    import importlib.util
    import sys

    script = PROJECT_ROOT / "scripts" / "docker-storage-policy.py"
    spec = importlib.util.spec_from_file_location("docker_storage_policy_args", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.build_parser()


@pytest.mark.parametrize("phase", sorted(PHASES))
def test_every_release_phase_parses_against_the_real_policy_script(phase: str) -> None:
    """A wrapper that agrees only with a test is a wrapper that drifts.

    The eleven hand-written call sites at least failed loudly on a renamed
    flag, because each one was the command. One wrapper asserted against a
    hand-copied expectation could pass forever while the script it drives moved
    underneath it.

    Not a complete guard: argparse accepts unambiguous prefixes, so `--bound`
    parses as `--boundary` here exactly as it would in the shell. It catches a
    flag that was renamed, not one that was shortened.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    Storage(runner).release(phase)

    argv = list(runner.commands[0].argv)
    tail = argv[argv.index("release") :]

    parsed = _policy_parser().parse_args(tail)
    assert (parsed.boundary, parsed.rail) == (PHASES[phase].boundary, PHASES[phase].rail)


def test_gc_clean_and_capture_failure_parse_against_the_real_script() -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    storage = Storage(runner)

    storage.gc(rail="install")
    storage.clean(scope="working", rail="default")
    storage.capture_failure(rail="default", label="abcdef123456")

    parser = _policy_parser()
    for command in runner.commands:
        argv = list(command.argv)
        start = next(
            index for index, part in enumerate(argv) if part in {"gc", "clean", "capture-failure"}
        )
        parser.parse_args(argv[start:])

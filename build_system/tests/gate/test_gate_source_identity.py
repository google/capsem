"""The code the gate runs is the code the gate recorded.

CPython decides a `.pyc` is current by comparing the source's mtime and size.
Two edits of the same length inside one timestamp tick therefore leave bytecode
that still validates, and the interpreter runs the stale one -- which during a
review of this package produced 74 identical false failures referring to a name
the source no longer contained.

The source guard records `HEAD` and a digest of the bytes on disk. It says
nothing about the bytes the interpreter is executing, so without this the gate
can qualify a plan built by code that is not in the tree being released.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import NoReturn, cast

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(PROJECT_ROOT / "tests"))

#: Same length, different meaning. The whole defect is that a same-size edit
#: within the timestamp's resolution is indistinguishable from no edit.
BEFORE = "stale"
AFTER = "fresh"


def _probe(directory: Path, value: str) -> Path:
    """A module whose only job is to say which version of itself ran."""
    module = directory / "capsem_bytecode_probe.py"
    module.write_text(f'VALUE = "{value}"\n', encoding="utf-8")
    return module


def _observed(directory: Path, environment: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        [sys.executable, "-c", "import capsem_bytecode_probe as p; print(p.VALUE)"],
        capture_output=True,
        text=True,
        timeout=60,
        env={
            **{k: v for k, v in os.environ.items() if k != "PYTHONPYCACHEPREFIX"},
            "PYTHONPATH": str(directory),
            **(environment or {}),
        },
    )
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


def _rewrite_preserving_timestamp(module: Path, value: str) -> None:
    """The edit CPython cannot see: same size, same mtime."""
    before = module.stat()
    module.write_text(f'VALUE = "{value}"\n', encoding="utf-8")
    assert module.stat().st_size == before.st_size, "the probe edit changed the size"
    os.utime(module, ns=(before.st_atime_ns, before.st_mtime_ns))


def test_an_ordinary_interpreter_runs_the_stale_bytecode(tmp_path: Path) -> None:
    """The hole itself, reproduced, so the fix below is not a guess."""
    module = _probe(tmp_path, BEFORE)
    assert _observed(tmp_path) == BEFORE  # compiles, and caches

    _rewrite_preserving_timestamp(module, AFTER)

    assert _observed(tmp_path) == BEFORE, (
        "this platform's timestamp resolution already defeats the stale cache, "
        "so the isolation below cannot be proven here"
    )


def test_the_launcher_environment_runs_the_current_source(tmp_path: Path) -> None:
    from capsem_builder.gatelaunch import isolated_environment

    module = _probe(tmp_path, BEFORE)
    _observed(tmp_path)
    _rewrite_preserving_timestamp(module, AFTER)

    assert _observed(tmp_path, isolated_environment(tmp_path)) == AFTER


def test_unchanged_source_reuses_one_abi_keyed_generation(tmp_path: Path) -> None:
    from capsem_builder.gatelaunch import MARKER, PYCACHE, isolated_environment

    first = isolated_environment(tmp_path)
    second = isolated_environment(tmp_path)

    assert first[PYCACHE] == second[PYCACHE]
    assert first[MARKER] == first[PYCACHE]
    for environment in (first, second):
        assert Path(environment[PYCACHE]).is_dir()


def test_changed_source_selects_a_new_generation(tmp_path: Path) -> None:
    from capsem_builder.gatelaunch import PYCACHE, isolated_environment

    module = _probe(tmp_path, BEFORE)
    first = isolated_environment(tmp_path)
    _rewrite_preserving_timestamp(module, AFTER)
    second = isolated_environment(tmp_path)

    assert first[PYCACHE] != second[PYCACHE]


def test_children_inherit_the_isolation(tmp_path: Path) -> None:
    """pytest, the builders and the scripts are all children of the gate.

    The 74 false failures came from a pytest run, not from the gate's own
    import, so isolating only this interpreter would fix the smaller half.
    """
    from capsem_builder.gatelaunch import PYCACHE, isolated_environment

    environment = isolated_environment(tmp_path)
    assert PYCACHE in environment, "the variable CPython reads must be exported"

    module = _probe(tmp_path, BEFORE)
    _observed(tmp_path)
    _rewrite_preserving_timestamp(module, AFTER)

    grandchild = subprocess.run(
        [
            sys.executable,
            "-c",
            "import subprocess, sys; "
            "sys.exit(subprocess.run([sys.executable, '-c', "
            "'import capsem_bytecode_probe as p; print(p.VALUE)']).returncode)",
        ],
        capture_output=True,
        text=True,
        timeout=60,
        env={**os.environ, "PYTHONPATH": str(tmp_path), **environment},
    )
    assert grandchild.stdout.strip() == AFTER, grandchild.stderr


# ---------------------------------------------------------------------------
# The wiring: the canonical entry point has to be the isolated one
# ---------------------------------------------------------------------------


def test_the_console_script_is_the_launcher() -> None:
    """Otherwise the isolation is a function nobody calls."""
    manifest = tomllib.loads(
        (PROJECT_ROOT / "build_system/pyproject.toml").read_text(encoding="utf-8")
    )

    assert manifest["project"]["scripts"]["capsem-gate"] == ("capsem_builder.gatelaunch:main")


def test_the_launcher_imports_nothing_from_the_package_it_protects() -> None:
    """An import at module scope compiles `capsem_builder.gate` before the fix runs.

    `capsem_builder.gate.__init__` carries real code, so its own stale bytecode is
    exactly the failure this exists to prevent.
    """
    source = (PROJECT_ROOT / "build_system/builder/gatelaunch.py").read_text(encoding="utf-8")

    top_level = [
        line
        for line in source.splitlines()
        if line.startswith(("import ", "from ")) and "capsem" in line
    ]
    assert not top_level, f"the launcher imports the gate at module scope: {top_level}"


def test_the_launcher_runs_the_gate_once_the_cache_is_isolated(
    monkeypatch: object,
) -> None:
    """With the exact marker and live prefix it must not re-exec forever."""
    import capsem_builder.gatelaunch as launcher

    calls: list[str] = []
    monkeypatch.setattr(os, "execv", lambda *a: calls.append("execv"))  # type: ignore[attr-defined]
    environment = launcher.isolated_environment()
    for name, value in environment.items():
        monkeypatch.setenv(name, value)  # type: ignore[attr-defined]
    cast(pytest.MonkeyPatch, monkeypatch).setattr(
        sys, "pycache_prefix", environment[launcher.PYCACHE]
    )
    monkeypatch.setattr(sys, "argv", ["capsem-gate", "--help"])  # type: ignore[attr-defined]

    from capsem_builder.gate import cli

    monkeypatch.setattr(cli, "main", lambda: 0)  # type: ignore[attr-defined]
    assert launcher.main() == 0
    assert not calls, "an isolated interpreter re-execed anyway"


def test_the_launcher_re_execs_when_the_cache_is_not_isolated(
    monkeypatch: object, tmp_path: Path
) -> None:
    import capsem_builder.gatelaunch as launcher

    issued: list[list[str]] = []

    class Replaced(BaseException):
        """A real `execv` never returns; the stub says so the same way."""

    def _execv(_program: str, argv: list[str]) -> NoReturn:
        issued.append(list(argv))
        raise Replaced

    monkeypatch.delenv(launcher.MARKER, raising=False)  # type: ignore[attr-defined]
    monkeypatch.setattr(os, "execv", _execv)  # type: ignore[attr-defined]
    monkeypatch.setattr(sys, "argv", ["capsem-gate", "candidate", "--dry-run"])  # type: ignore[attr-defined]
    monkeypatch.setattr(launcher, "checkout", lambda: tmp_path)  # type: ignore[attr-defined]

    with pytest.raises(Replaced):
        launcher.main()

    (argv,) = issued
    assert argv[1:] == ["-m", "capsem_builder.gate", "candidate", "--dry-run"]
    assert os.environ[launcher.PYCACHE].startswith(str(tmp_path))


# ---------------------------------------------------------------------------
# The identity the source guard was missing
# ---------------------------------------------------------------------------


def test_the_recorded_source_state_names_the_tree_the_gate_is_running_from() -> None:
    """`HEAD` and the digest describe a checkout; nothing said which code
    read them. Recorded, so a run can be read back and answered."""
    from capsem_builder.gate import sourcestate

    assert sourcestate.gate_source() == PROJECT_ROOT / "build_system/builder/gate"


def test_the_complete_gate_refuses_an_unisolated_interpreter(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """The claim the digest cannot make on its own.

    A checkout digest is of the bytes on disk. Whether the interpreter is
    executing those bytes is a different question, and the only evidence a
    running gate has is that it was entered through the launcher.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.errors import GateError
    from capsem_builder.gate.sourcestate import RequireIsolatedBytecode
    from capsem_builder.gatelaunch import MARKER
    from helpers.gate import RecordingRunner

    context = Context(RecordingRunner(PROJECT_ROOT), gate_config.load(PROJECT_ROOT))

    monkeypatch.delenv(MARKER, raising=False)
    with pytest.raises(GateError, match="stale"):
        RequireIsolatedBytecode().perform(context)

    monkeypatch.setenv(MARKER, str(tmp_path))
    RequireIsolatedBytecode().perform(context)


def test_the_complete_gate_checks_isolation_in_its_first_step() -> None:
    """Before forty minutes of work, not after."""
    from helpers.gate import gate_plan

    plan = gate_plan("candidate")
    first = plan.labels[0]

    assert first == "source.record"
    (step,) = [s for s in plan.steps if s.label == first]
    assert "require-isolated-bytecode" in [action.name for action in step.actions]


def test_a_run_records_what_built_its_plan(tmp_path: Path) -> None:
    """Beside HEAD and the source digest, which describe a checkout.

    Neither of those says which code read them. A run that measured one tree
    while executing another would be identical in every other field, so the
    two identities that answer it are written down: where the gate was
    imported from, and which bytecode cache it ran under.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.runhistory import read
    from capsem_builder.gate.runlog import RunLog
    from capsem_builder.gatelaunch import PYCACHE

    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"), encoding="utf-8"
    )
    config = gate_config.load(tmp_path)

    with RunLog.open(config, "test") as log:
        directory = log.directory

    (start,) = [e for e in read(directory, config.runlog) if e["event"] == "run.start"]
    assert start["gate_source"] == str(PROJECT_ROOT / "build_system/builder/gate")
    assert start["pycache"] == os.environ.get(PYCACHE, "")

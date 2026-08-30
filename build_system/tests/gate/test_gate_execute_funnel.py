"""The one function every command passes through, and what it refuses.

`execute` planned, locked, held and ran -- and trusted each command to log its
own subprocesses, to stay out of its own lock, and to build its plan without
touching anything. None of the three was true.

`RunLog.exec` had no production caller at all, so no run log ever recorded a
single command. Ten plan actions invoked `just` or `capsem-gate`, and because
the machine lock is not reentrant each was a child waiting out a 7200-second
timeout for the lock its own parent was holding. And `plan()` shelled out to
`git rev-parse` on the release path, so `--dry-run` -- the thing you reach for
precisely because you do not want to touch the machine -- touched the machine.

The lesson those three share is that a rule living outside the funnel is a rule
each new command has to remember. These tests put them inside it, where
forgetting is not available.
"""

from __future__ import annotations

import argparse
import json
import os
from contextlib import contextmanager
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Run, Script
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.execution import step
from capsem_builder.gate.lifecycle import Resource
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.sourcecommit import qualified_commit
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
RUN_MARKER = CONFIG.locks.gate.run_marker


# ---------------------------------------------------------------------------
# Fixtures: a command whose plan is whatever a test needs it to be
# ---------------------------------------------------------------------------


class Recorder(Resource, name="funnel-recorder"):
    """A resource that reports what it exports and whether it was taken."""

    def __init__(self, log: list[str], label: str, **environment: str) -> None:
        self._log, self._label, self._environment = log, label, environment
        self.acquired = False

    def acquire(self) -> None:
        self.acquired = True
        self._log.append(f"acquire {self._label}")

    def release(self) -> None:
        self._log.append(f"release {self._label}")

    def environment(self) -> dict[str, str]:
        return dict(self._environment)


class _Probe(GateCommand, name="funnel-probe", help="a command a test builds"):
    """Its plan and resources are whatever the test assigned before running."""

    steps: tuple = ()
    holdings: tuple[Resource, ...] = ()
    on_plan = None
    replacement: tuple[str, ...] | None = None

    def resources(self, runner=None) -> tuple[Resource, ...]:
        return self.holdings

    def reexec(self) -> tuple[str, ...] | None:
        return self.replacement

    def plan(self) -> Plan:
        if self.on_plan is not None:
            self.on_plan(self._runner)
        plan = Plan(self.name)
        for item in self.steps:
            plan.add(item)
        return plan


@pytest.fixture
def journal(monkeypatch) -> RecordingJournal:
    """A run log that keeps events in memory instead of under `target/`."""
    recording = RecordingJournal()

    @classmethod
    @contextmanager
    def _open(cls, config, command, *, argv=(), source_commit=None):
        assert source_commit is None
        yield recording

    monkeypatch.setattr("capsem_builder.gate.runlog.RunLog.open", _open)
    monkeypatch.setattr("capsem_builder.gate.recording.RunLog.open", _open)
    return recording


class _Exclusive(_Probe, name="funnel-exclusive-probe", help="a probe that takes the machine"):
    """The same probe, for the tests that are about the machine lock.

    A separate class rather than an assignment: `exclusive` is a `ClassVar`,
    because whether a command needs the machine is a property of the command
    and not of one invocation of it.
    """

    exclusive = True


def _checkout(root: Path) -> Path:
    """A throwaway checkout, so a real lock is taken somewhere harmless."""
    (root / "config").mkdir(parents=True, exist_ok=True)
    source = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    replacements = {
        'path = "~/.capsem-gate/capsem-test-execution.lock"': (
            f"path = {json.dumps(str(root / 'gate.lock'))}"
        ),
        'holder_record = "~/.capsem-gate/capsem-test-execution.holder"': (
            f"holder_record = {json.dumps(str(root / 'gate.holder'))}"
        ),
    }
    for production, isolated in replacements.items():
        assert source.count(production) == 1
        source = source.replace(production, isolated)
    (root / "config" / "gate.toml").write_text(source, encoding="utf-8")
    (root / "justfile").write_text("# a checkout needs one\n")
    return root


def _probe(runner, *, cls: type[_Probe] = _Probe, **attributes) -> _Probe:
    command = cls(runner, argparse.Namespace(dry_run=False, graph=False, timing=False))
    for name, value in attributes.items():
        setattr(command, name, value)
    return command


# ---------------------------------------------------------------------------
# Recursion, which the runner refuses rather than a test noticing
# ---------------------------------------------------------------------------


def test_the_real_lock_fixture_is_isolated_from_the_machine(tmp_path: Path) -> None:
    """A synthetic exclusive command must never contend with a running gate."""
    root = _checkout(tmp_path)
    lock = gate_config.load(root).locks.gate

    assert Path(lock.path).parent == root
    assert Path(lock.holder_record).parent == root


def test_a_plan_action_may_not_invoke_just(journal) -> None:
    """`just test-clean` from inside a plan is the deadlock, not a style problem.

    The child asks for the machine lock its own parent is holding and waits out
    the full timeout. Ten of these existed, and every one of them looked
    reasonable at the call site.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("qualify", Run(["just", "test"])),))

    with pytest.raises(GateError, match="qualify"):
        command.execute()

    assert not runner.commands, "it must be refused before it runs, not after"


def test_a_plan_action_may_not_invoke_another_gate_command(journal) -> None:
    """Both spellings, because the gate already used both."""
    for argv in (
        ["uv", "run", "capsem-gate", "assets"],
        ["capsem-gate", "assets"],
    ):
        runner = RecordingRunner(PROJECT_ROOT)
        command = _probe(runner, steps=(step("build", Run(argv)),))

        with pytest.raises(GateError, match="build"):
            command.execute()

        assert not runner.commands


def test_a_command_may_not_run_inside_another_gate_run(journal, monkeypatch) -> None:
    """The in-process spelling of the same deadlock, and the one that hid.

    `GuardedRunner` refuses a *subprocess* that starts a second gate. It cannot
    see a `cli.main([...])` called from Python inside a process the gate
    launched -- and the gate launches pytest, whose suite calls exactly that.
    The result was not an error: it was the worker blocking on the lock its own
    grandparent held, for the full 7200-second timeout, with the run looking
    alive the whole time.
    """
    monkeypatch.setenv(RUN_MARKER, "gate-20260801-abc123")
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, cls=_Exclusive, steps=(step("work", Run(["echo", "hello"])),))

    with pytest.raises(GateError, match="already inside"):
        command.execute()

    assert not runner.commands


def test_the_re_entry_error_names_the_run_that_is_holding_it(journal, monkeypatch) -> None:
    monkeypatch.setenv(RUN_MARKER, "gate-20260801-abc123")
    command = _probe(RecordingRunner(PROJECT_ROOT), cls=_Exclusive)

    with pytest.raises(GateError) as raised:
        command.execute()

    assert "gate-20260801-abc123" in str(raised.value)


def test_a_read_only_command_may_still_answer_inside_a_run(journal, monkeypatch) -> None:
    """`runs last` and `gc --dry-run` take no lock, so they cannot deadlock --
    and being able to ask what is happening from inside a running gate is the
    entire point of them."""
    monkeypatch.setenv(RUN_MARKER, "gate-20260801-abc123")
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("look", Run(["echo", "hello"])),))

    command.execute()

    assert runner.commands


def test_a_run_marks_the_environment_its_children_inherit(journal, monkeypatch, tmp_path) -> None:
    """Which is what makes the check above reach a subprocess at all.

    The lock exports it like any other resource, so it arrives the same way a
    workspace's `CAPSEM_HOME` does -- rather than through a mutated
    `os.environ` that a failure could leave behind.

    Against a throwaway checkout, because this is the one test here that really
    takes the lock, and it must not be the developer's own.
    """
    monkeypatch.delenv(RUN_MARKER, raising=False)
    runner = RecordingRunner(_checkout(tmp_path))
    command = _probe(runner, cls=_Exclusive, steps=(step("look", Run(["echo", "hello"])),))

    command.execute()

    assert runner.commands[-1].env.get(RUN_MARKER), "a child of this run cannot tell that it is one"
    assert os.environ.get(RUN_MARKER) is None, "and it does not leak into this process"


def test_re_entry_is_seen_through_a_wrapper(journal) -> None:
    """A wrapper in front is exactly how one of these gets past a naive check.

    `argv[0]` here is `caffeinate`, and the gate really does spell it this way
    on the keep-awake path.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "wrapped",
                Run(["caffeinate", "-dimsu", "env", "MARK=1", "capsem-gate", "candidate"]),
            ),
        ),
    )

    with pytest.raises(GateError, match="wrapped"):
        command.execute()


def test_the_error_says_what_to_do_instead(journal) -> None:
    """A refusal that does not name the alternative just gets worked around."""
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("nested", Run(["just", "_sign"])),))

    with pytest.raises(GateError, match="fragment"):
        command.execute()


def test_an_ordinary_program_is_not_mistaken_for_re_entry(journal) -> None:
    """The check must see through `uv run --project build_system --frozen` without swallowing what it wraps.

    `uv run --project build_system --frozen python build_system/scripts/...`
    is how command boundaries run, so a rule that flags it gets deleted.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "ordinary",
                Run(
                    [
                        "uv",
                        "run",
                        "--project",
                        "build_system",
                        "--frozen",
                        "python",
                        "build_system/scripts/audit/check-source-syntax.py",
                    ]
                ),
                Run(["cargo", "build", "--workspace"]),
                Run(["docker", "run", "--label", "just", "alpine"]),
                Script(CONFIG, "build_system/scripts/audit/audit-python-lock.sh"),
            ),
        ),
    )

    command.execute()

    assert len(runner.commands) == 4


# ---------------------------------------------------------------------------
# Plans that describe rather than do
# ---------------------------------------------------------------------------


def test_plan_construction_may_not_touch_the_machine(journal) -> None:
    """`--dry-run` is worthless if building the answer runs commands.

    The release plan captured `git rev-parse HEAD` through a fresh runner while
    it was being constructed, so asking what a release *would* do already ran
    something -- and the answer could go stale between the description and the
    execution.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, on_plan=lambda r: r.capture(["git", "rev-parse", "HEAD"]))

    with pytest.raises(GateError, match="plan"):
        command.execute()

    assert not runner.commands


def test_plan_construction_cannot_escape_by_building_its_own_runner(journal) -> None:
    """Sealing the command's runner is not enough, and this is how we know.

    `release.py` built a fresh `Runner(config.root)` inside `plan()` to capture
    `git rev-parse HEAD`. A seal that swapped `self._runner` never saw it: the
    dry run printed a real revision while the recording runner observed no
    commands at all -- the machine touched, and invisibly.

    So the seal is ambient rather than per-instance. Any runner, however it was
    obtained, refuses while a plan is being built.
    """
    from capsem_builder.gate.proc import Runner

    def _own_runner(_ignored) -> None:
        Runner(PROJECT_ROOT).capture(["git", "rev-parse", "HEAD"])

    command = _probe(RecordingRunner(PROJECT_ROOT), on_plan=_own_runner)

    with pytest.raises(GateError, match="plan"):
        command.execute()


@pytest.mark.parametrize("flag", ["dry_run", "graph"])
def test_inspection_issues_no_command_and_acquires_nothing(flag, capsys) -> None:
    """Asking must never become doing.

    `execute` consulted `reexec()` before it looked at these flags, so on macOS
    `capsem-gate candidate --dry-run` re-execed into a real `just test-clean`: a
    supposedly inert question starting a forty-minute destructive gate.
    """
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    resource = Recorder(log, "workspace")
    flags = {"dry_run": False, "graph": False, "timing": False, flag: True}
    command = _Probe(runner, argparse.Namespace(**flags))
    command.steps = (step("build", Run(["cargo", "build"])),)
    command.holdings = (resource,)
    # The exact shape of the regression: a command that would re-exec into
    # something long and destructive must not do so while being asked.
    command.replacement = ("caffeinate", "just", "test")

    command.execute()

    assert not runner.commands, "inspection ran a command"
    assert not log, "inspection acquired a resource"
    assert not resource.acquired
    # It must still answer the question: silence would pass the assertions
    # above for the wrong reason. `--graph` names the steps, `--dry-run` the
    # argv underneath them.
    printed = capsys.readouterr().out
    assert "build" in printed
    if flag == "dry_run":
        assert "cargo build" in printed


# ---------------------------------------------------------------------------
# Auto-logging: no call site is involved
# ---------------------------------------------------------------------------


def test_every_subprocess_is_recorded_once(journal) -> None:
    """The run log's whole purpose, and it had no production caller.

    Recorded here rather than at each call site, because sixteen call sites
    remembering is fifteen chances for one to stop.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step("one", Run(["cargo", "build"]), Run(["cargo", "clippy"])),
            step(
                "two",
                Script(CONFIG, "build_system/scripts/audit/check-source-syntax.py"),
            ),
        ),
    )

    command.execute()

    assert [entry["argv"][:2] for entry in journal.execs] == [
        ("cargo", "build"),
        ("cargo", "clippy"),
        ("uv", "run"),
    ]


def _recorded_command_policy(command: GateCommand, monkeypatch) -> str:
    """Drive a real command class through the funnel with one inert action."""
    runner = command._runner
    assert isinstance(runner, RecordingRunner)
    probe = Plan(command.name)
    probe.add(step("policy", Run(["doctor-policy-probe"])))
    monkeypatch.setattr(command, "plan", lambda: probe)
    monkeypatch.setattr(command, "resources", lambda _runner: ())
    # And declares none either: a command's `outside_egress` now produces an
    # `Egress` whether or not its `resources` mentions one, and a real egress
    # capability refuses to acquire inside an already-sandboxed process.
    monkeypatch.setattr(command, "outside_egress", False)
    monkeypatch.setattr(command, "reexec", lambda: None)
    monkeypatch.setattr(command, "exclusive", False)
    monkeypatch.setattr(command, "private_checkout", False)

    command.execute()

    return runner.commands[0].env[CONFIG.environment.command_sandbox_mode]


def test_candidate_overwrites_a_forged_ambient_sandbox_policy(journal, monkeypatch) -> None:
    """Doctor learns policy from the command, never from the invoking shell."""
    from capsem_builder.gate import candidate, sandbox
    from capsem_builder.gate.qualification import LocalQualification

    command = candidate.CandidateCommand(
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
        qualification=LocalQualification(bin_dir=CONFIG.modules.default_bin_dir),
    )
    monkeypatch.setenv(CONFIG.environment.command_sandbox_mode, sandbox.OFF.value)

    assert _recorded_command_policy(command, monkeypatch) == sandbox.ENFORCE.value


@pytest.mark.parametrize(
    ("requested", "ambient", "expected"),
    [
        (None, "enforce", "off"),
        ("enforce", "off", "enforce"),
    ],
)
def test_build_assets_exports_its_effective_sandbox_policy(
    journal, monkeypatch, requested: str | None, ambient: str, expected: str
) -> None:
    """Default build-assets stays open; an explicit enforcing override wins."""
    from capsem_builder.gate import imagebuild, sandbox

    parsed = None if requested is None else sandbox.SandboxMode(requested)
    command = imagebuild.BuildAssetsCommand(
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            dry_run=False,
            graph=False,
            timing=False,
            sandbox=parsed,
            profile=None,
            arch=None,
            template="all",
        ),
    )
    monkeypatch.setenv(CONFIG.environment.command_sandbox_mode, ambient)

    assert _recorded_command_policy(command, monkeypatch) == expected


def test_a_recorded_command_carries_what_a_diagnosis_needs(journal) -> None:
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(runner, steps=(step("one", Run(["cargo", "build"])),))

    command.execute()

    (recorded,) = journal.execs
    assert recorded["exit"] == 0
    assert recorded["cwd"] == str(PROJECT_ROOT)
    assert recorded["duration_ms"] >= 0


def test_a_failing_command_is_recorded_with_its_status(journal) -> None:
    """A run log that only holds successes is a log of the wrong runs."""
    runner = RecordingRunner(PROJECT_ROOT, failures=("cargo build",))
    command = _probe(runner, steps=(step("one", Run(["cargo", "build"])),))

    with pytest.raises(GateError):
        command.execute()

    assert [entry["exit"] for entry in journal.execs] == [1]


def test_the_recorded_environment_is_the_delta_not_the_machines(journal) -> None:
    """This file gets attached to bug reports, and a release machine's
    environment holds tokens."""
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner, steps=(step("one", Run(["cargo", "build"], env={"CAPSEM_MARK": "1"})),)
    )

    command.execute()

    (recorded,) = journal.execs
    # The commit is part of the delta and belongs in the report: a step that
    # authors release provenance is told which one it is proving rather than
    # resolving the tree it happens to sit in, so a bug report that omitted it
    # would omit which bytes the run was about. It is a revision, not a secret.
    assert recorded["env"] == {
        "CAPSEM_MARK": "1",
        CONFIG.environment.command_sandbox_mode: "off",
        CONFIG.environment.qualified_source_commit: qualified_commit(PROJECT_ROOT, None),
    }


# ---------------------------------------------------------------------------
# Isolation comes from what was acquired, not from what a call site remembered
# ---------------------------------------------------------------------------


def test_the_acquired_resources_environment_reaches_every_command(journal) -> None:
    """`Workspace.environment` existed and production never used it.

    `execute` built a bare `Context`, so every command advertised as isolated
    ran against the developer's own `~/.capsem` -- able to read and modify real
    service state, and able to pass or fail on machine state.
    """
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"),),
        steps=(
            step(
                "one",
                Run(["cargo", "build"]),
                Script(CONFIG, "build_system/scripts/build/x.py"),
            ),
        ),
    )

    command.execute()

    for issued in runner.commands:
        assert issued.env["CAPSEM_HOME"] == "/tmp/isolated"


def test_a_later_resource_wins_over_an_earlier_one(journal) -> None:
    """Acquisition order is the precedence order, as a stack would give."""
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(
            Recorder(log, "outer", CAPSEM_HOME="/tmp/outer", KEEP="yes"),
            Recorder(log, "inner", CAPSEM_HOME="/tmp/inner"),
        ),
        steps=(step("one", Run(["cargo", "build"])),),
    )

    command.execute()

    (issued,) = runner.commands
    assert issued.env["CAPSEM_HOME"] == "/tmp/inner"
    assert issued.env["KEEP"] == "yes"


def test_an_action_that_names_a_variable_still_wins(journal) -> None:
    """The narrower scope is the one that meant it."""
    log: list[str] = []
    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"),),
        steps=(step("one", Run(["cargo", "build"], env={"CAPSEM_HOME": "/tmp/step"})),),
    )

    command.execute()

    assert runner.commands[0].env["CAPSEM_HOME"] == "/tmp/step"


def test_a_failed_acquire_runs_no_command_at_all(journal) -> None:
    """A half-built scope must not execute anything in itself.

    Note what this does *not* prove. `held` yields the resources it acquired
    rather than the ones it was asked for, which reads as the safer shape --
    but the two differ only when an acquire raised, and then the body never
    runs, so no test can tell them apart. The shape is kept for what it says,
    not for a behaviour it defends; the behaviour worth defending is this one.
    """
    log: list[str] = []

    class Refuses(Resource, name="funnel-refuses"):
        def acquire(self) -> None:
            raise GateError("no")

        def release(self) -> None:
            log.append("release refuses")

        def environment(self) -> dict[str, str]:
            return {"CAPSEM_HOME": "/tmp/never"}

    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        holdings=(Recorder(log, "workspace", CAPSEM_HOME="/tmp/isolated"), Refuses()),
        steps=(step("one", Run(["cargo", "build"])),),
    )

    with pytest.raises(GateError, match="no"):
        command.execute()

    assert not runner.commands
    assert "release refuses" not in log


# ---------------------------------------------------------------------------
# What a plan must hold before a lock is taken
# ---------------------------------------------------------------------------


def test_a_step_may_not_claim_an_undeclared_exclusive(journal) -> None:
    """`[execution.exclusives]` carries the reason each one exists; a claim on
    something absent from it excludes nothing and reads as though it does."""
    from capsem_builder.gate.harnessschema import Exclusive

    runner = RecordingRunner(PROJECT_ROOT)
    command = _probe(
        runner,
        steps=(
            step(
                "one",
                Run(["cargo", "build"]),
                contends=(Exclusive(name="invented", reason="none"),),
            ),
        ),
    )

    with pytest.raises(GateError, match="invented"):
        command.execute()

    assert not runner.commands, "checked before the machine lock, not after"


def test_every_real_resource_satisfies_the_environment_protocol() -> None:
    """The double is not the thing.

    `Resource.environment()` is a method, and `Workspace.environment` was a
    property -- so `environment_of` raised `TypeError: 'dict' object is not
    callable` against the one resource every isolated command actually holds.
    The funnel tests never saw it, because they exercise a recorder written to
    match the protocol rather than the classes that have to implement it.

    Every concrete `Resource` in the package, then, not a stand-in.
    """
    import inspect

    from capsem_builder.gate import cli  # noqa: F401 - imports every module
    from capsem_builder.gate.lifecycle import Resource, environment_of

    concrete = [
        cls
        for cls in _descendants(Resource)
        if cls.__module__.startswith("capsem_builder.gate.") and not inspect.isabstract(cls)
    ]
    assert len(concrete) >= 4, f"scanned too few resources: {concrete}"

    for cls in concrete:
        assert callable(cls.environment), (
            f"{cls.__name__}.environment is not callable; `environment_of` would raise against it"
        )

    # And the one every isolated command holds, exercised for real.
    workspace = _workspace()
    assert set(environment_of((workspace,))) == set(workspace.environment())


def _descendants(root: type) -> list[type]:
    found = []
    for child in root.__subclasses__():
        found.append(child)
        found += _descendants(child)
    return found


def _workspace():
    from capsem_builder.gate.workspace import Workspace

    return Workspace(CONFIG)


def test_the_workspace_exports_the_four_variables_that_isolate_a_run() -> None:
    """Named individually, because losing one silently relocates part of a run
    back into the developer's own home."""
    exported = _workspace().environment()

    assert set(exported) == {
        "CAPSEM_HOME",
        "CAPSEM_RUN_DIR",
        "CAPSEM_BENCHMARK_OUTPUT_ROOT",
        "COVERAGE_FILE",
    }
    run_dir = exported.pop("CAPSEM_RUN_DIR")
    assert run_dir.startswith("/tmp/capsem-r-")
    for value in exported.values():
        assert str(CONFIG.root) in value, f"{value} is outside the checkout"

"""Citadel guard: the gate's cross-run state reaches whoever works next.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards a mistake of *reach*: a measurement that is computed
correctly, written correctly, and read by nobody.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.context import Context
from capsem_builder.gate.digestreport import RefreshDigest
from capsem_builder.gate.execution import step as make_step
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.runledger import LedgerRow, StepRow
from helpers.gate import RecordingRunner, gate_plan

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(PROJECT_ROOT)

SETTINGS = PROJECT_ROOT / ".claude" / "settings.json"
SCRIPT = PROJECT_ROOT / "scripts" / "print-gate-digest.py"
#: Every agent contract that has to name it, because three agents work here and
#: only one of them gets the hook.
CONTRACTS = ("AGENTS.md", "CLAUDE.md", "GEMINI.md")
DIGEST_STEP = "fast.digest"

DIGEST_ECHO_RATIONALE = """\
The gate's cross-run state must reach whoever works next, without being asked.

Every failure mode that costs real time here is longitudinal. A step that fails
one run in four is excused every single time it is seen alone. A phase that has
doubled since a change three days ago looks like an ordinary slow build. A
critical path made of queueing looks like work. None of these is visible in the
run you are looking at, which is the only run anyone looks at.

So the digest is computed from the ledger and pushed, not offered:

  written by every run       `fast.digest` early in the phase, and again at
                             `RunLog.close` so the finished run is included
  printed into each session   a session-start hook, so an agent that never
                             thought to ask still knows
  named in all three         AGENTS.md, CLAUDE.md and GEMINI.md, because Codex
  agent contracts            and Gemini do not get the hook

Each of those is one edit away from silently not happening, and the failure is
invisible by construction -- nothing breaks, the digest simply stops being read
and the intermittent failures go back to being bad luck. That is what this
guard is for.

Two rules about the hook itself, both learned the direct way. It must not
invoke `capsem-gate`: that takes the history lock and can block behind a
running gate, and a session that hangs on startup gets its hook deleted. And it
must not fail on a fresh checkout, where having no digest yet is the ordinary
state rather than an error.

See skills/citadel/SKILL.md and config/gate.toml [runlog.digest].
"""


def _script_module():
    """`print-gate-digest.py` imported as a module, hyphen and all."""
    spec = importlib.util.spec_from_file_location("print_gate_digest", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def hook_commands(settings: dict) -> list[str]:
    """Every command a session-start hook would run."""
    return [
        hook.get("command", "")
        for entry in settings.get("hooks", {}).get("SessionStart", [])
        for hook in entry.get("hooks", [])
    ]


def invokes_the_gate(command: str) -> bool:
    """Whether a hook command would take the gate's lock.

    Matched on the command name rather than on a list of known-bad spellings:
    `just test-clean`, `capsem-gate runs digest` and `uv run capsem-gate ...` all
    take the machine or history lock, and the point is that none of them may
    run before a session has started.
    """
    return any(token in command for token in ("capsem-gate", "just "))


def test_the_hook_is_registered() -> None:
    assert SETTINGS.is_file(), DIGEST_ECHO_RATIONALE + f"\n{SETTINGS} is missing"
    commands = hook_commands(json.loads(SETTINGS.read_text(encoding="utf-8")))
    assert commands, DIGEST_ECHO_RATIONALE + "\nno SessionStart hook is registered"
    assert any(SCRIPT.name in command for command in commands), (
        DIGEST_ECHO_RATIONALE + f"\nno SessionStart hook runs {SCRIPT.name}: {commands}"
    )


def test_the_hook_does_not_invoke_the_gate() -> None:
    commands = hook_commands(json.loads(SETTINGS.read_text(encoding="utf-8")))
    blocking = [command for command in commands if invokes_the_gate(command)]
    assert not blocking, (
        DIGEST_ECHO_RATIONALE + f"\na session-start hook would take the gate's lock: {blocking}"
    )


def test_the_script_reads_the_path_the_gate_writes() -> None:
    """The hook and the gate must agree about where the digest is.

    Compared by resolving both, not by comparing spellings. The script reads
    `config/gate.toml` with `tomllib` because a session hook cannot depend on
    the project environment, and that freedom is exactly what lets it drift.
    """
    assert SCRIPT.is_file(), DIGEST_ECHO_RATIONALE + f"\n{SCRIPT} is missing"
    resolved = _script_module().digest_path(PROJECT_ROOT)
    expected = PROJECT_ROOT / CONFIG.runlog.digest.path
    assert resolved == expected, (
        DIGEST_ECHO_RATIONALE + f"\nthe hook reads {resolved}, the gate writes {expected}"
    )


@pytest.mark.parametrize("state", ["no config at all", "config but no digest"])
def test_the_script_never_fails_a_session(
    state: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys
) -> None:
    """Both empty states exit zero and say what is missing.

    A hook that exits non-zero on a fresh clone is a hook everybody deletes,
    and deleting it is how the digest stops being read at all. Driven through
    `main` rather than asserted about the source, because returning zero is the
    behaviour and reading the code is not the same as running it.
    """
    module = _script_module()
    monkeypatch.setattr(module, "ROOT", tmp_path)
    if state == "config but no digest":
        (tmp_path / "config").mkdir()
        (tmp_path / "config" / "gate.toml").write_text(
            f'[runlog.digest]\npath = "{CONFIG.runlog.digest.path}"\n', encoding="utf-8"
        )

    assert module.main() == 0, DIGEST_ECHO_RATIONALE + f"\nthe hook failed with {state}"
    printed = capsys.readouterr().out
    assert printed.strip(), DIGEST_ECHO_RATIONALE + f"\nthe hook said nothing with {state}"


@pytest.mark.parametrize("name", CONTRACTS)
def test_the_agent_contracts_name_the_digest(name: str) -> None:
    text = (PROJECT_ROOT / name).read_text(encoding="utf-8")
    assert CONFIG.runlog.digest.path in text, (
        DIGEST_ECHO_RATIONALE
        + f"\n{name} does not name {CONFIG.runlog.digest.path}; the agents that "
        "do not get the hook would never learn it exists"
    )


def test_the_digest_is_rebuilt_in_the_fast_phase() -> None:
    """Early, or it reports on the run after the one you are waiting for."""
    plan = gate_plan("candidate")
    assert DIGEST_STEP in plan.labels, (
        DIGEST_ECHO_RATIONALE + f"\n{DIGEST_STEP} is not in the candidate plan"
    )


def test_interrogating_the_plan_does_not_rewrite_the_digest(tmp_path: Path) -> None:
    """Asking what the gate would do must not do any of it.

    Driven as a real one-step plan under `observing=True`, which is what every
    contract that interrogates the gate passes. `RecordSourceState` learned
    this the expensive way -- it overwrote the running gate's own state file
    with the recorder's empty output, and `source.verify` reported a HEAD
    change on an untouched tree at the end of a forty-minute run.

    Asserted against a sentinel rather than by comparing the file before and
    after. The digest is a pure function of the ledger, so a step that wrongly
    rewrites it produces byte-identical output -- the first version of this
    test passed with the `observing` check deleted, which is the whole reason
    the rule is to break a guard once and watch it go red.
    """
    (tmp_path / "config").mkdir()
    shutil.copy(PROJECT_ROOT / "config" / "gate.toml", tmp_path / "config" / "gate.toml")
    config = gate_config.load(tmp_path)

    digest = tmp_path / config.runlog.digest.path
    digest.parent.mkdir(parents=True, exist_ok=True)
    digest.write_text("untouched", encoding="utf-8")

    plan = Plan("observe-the-digest")
    plan.add(make_step("digest", RefreshDigest()))
    plan.run(Context(RecordingRunner(tmp_path), config, observing=True))

    assert digest.read_text(encoding="utf-8") == "untouched", (
        DIGEST_ECHO_RATIONALE + "\ninterrogating the plan rewrote the digest; RefreshDigest must "
        "return early when context.observing is set"
    )


def test_the_ledger_row_round_trips() -> None:
    """The ledger outlives every run directory, so it must survive reload.

    Cheap to assert and the whole premise of keeping it: a row written today
    is read months from now, by which time the only thing guaranteeing it is
    still readable is that nothing changed the shape without noticing.
    """
    row = LedgerRow(
        row_schema=CONFIG.runlog.ledger.row_schema,
        run_id="20260101-000000-abcdef-candidate",
        command="candidate",
        head="0" * 40,
        status="ok",
        total_ms=1.0,
        identity="deadbeef",
        critical_path=("fast.digest",),
        steps={"fast.digest": StepRow(duration_ms=1.0, status="ok")},
    )
    assert LedgerRow.model_validate_json(row.model_dump_json()) == row


# -- adversarial: the guard has to fail when the wiring is removed ----------


def test_a_settings_file_without_the_hook_is_caught() -> None:
    assert hook_commands({}) == []
    assert hook_commands({"hooks": {"SessionStart": [{"hooks": []}]}}) == []


@pytest.mark.parametrize(
    "command",
    [
        "uv run capsem-gate runs digest",
        "capsem-gate runs digest",
        "just test-clean",
        "cd /repo && uv run capsem-gate runs digest || true",
    ],
)
def test_a_hook_that_would_take_the_lock_is_caught(command: str) -> None:
    assert invokes_the_gate(command)


def test_the_registered_hook_is_not_itself_caught() -> None:
    """The rule must not reject the thing it is protecting."""
    commands = hook_commands(json.loads(SETTINGS.read_text(encoding="utf-8")))
    assert commands and not any(invokes_the_gate(command) for command in commands)


def _load_hook():
    """Import the hook as a module, fresh each time.

    Fresh because the tests below replace `trunk_runs` to drive the rendering
    without a network; a shared module would leak that stub into the next test.
    """
    spec = importlib.util.spec_from_file_location("gate_digest_hook", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# -- trunk CI reaches the session too ----------------------------------------

CI_BLINDNESS_RATIONALE = """\
The digest described local gate runs and nothing else, and that gap had a cost.

Trunk sat red for fourteen consecutive CI runs on a single root cause -- a
published profile that no longer satisfied a rule the source had grown -- while
this hook printed a healthy-looking local picture. Every instrument was
correct. None of them was looking at the thing that was broken.

So the hook asks GitHub, and it asks *here* rather than in the gate, because
the gate cannot: candidate and the release lanes run inside the kernel network
boundary, so a digest written during a run could never reach it. The section
would have gone missing exactly when a run was happening.

Three properties are load-bearing and each is asserted below. It degrades to
"unknown" rather than to silence, because a missing section reads as green. It
is bounded by a timeout, because a session-start hook that hangs is one
somebody deletes. And when trunk is red it demands a statement -- fix it, or
say out loud that you are leaving it red -- because a failure rate nobody is
required to acknowledge is one everybody learns to route around.

See scripts/print-gate-digest.py.
"""


def test_the_hook_reports_trunk_ci(monkeypatch: pytest.MonkeyPatch) -> None:
    hook = _load_hook()
    seen: dict[str, object] = {}

    def run(argv, **kwargs):
        seen.update(argv=argv, **kwargs)
        return hook.subprocess.CompletedProcess(
            argv,
            0,
            '[{"status":"completed","conclusion":"success"}]',
            "",
        )

    monkeypatch.setattr(hook.subprocess, "run", run)
    assert hook.trunk_runs() == [{"status": "completed", "conclusion": "success"}]
    assert seen["argv"] == [
        "gh",
        "run",
        "list",
        "--workflow",
        hook.CI_WORKFLOW,
        "--branch",
        hook.CI_BRANCH,
        "--limit",
        str(hook.CI_RUNS),
        "--json",
        "status,conclusion,displayTitle,url,createdAt",
    ]
    assert seen["timeout"] == hook.CI_TIMEOUT_SECONDS


def test_a_red_trunk_demands_a_statement(monkeypatch: pytest.MonkeyPatch) -> None:
    """Red must ask for a decision, not merely report a colour."""
    hook = _load_hook()
    red = [{"status": "completed", "conclusion": "failure", "displayTitle": "x", "url": "u"}] * 3
    monkeypatch.setattr(hook, "trunk_runs", lambda: red)
    section = hook.ci_section()

    assert "RED" in section and "3 consecutive" in section, section
    for demand in ("fix it", "unrelated"):
        assert demand in section.lower(), (
            CI_BLINDNESS_RATIONALE + f"\nred no longer asks the reader to {demand!r}"
        )


def test_a_cancelled_run_neither_counts_nor_clears_the_streak() -> None:
    """A cancellation says nothing about the code.

    Counting one inflates the streak; letting one break it makes a wall of red
    read as an isolated blip. Seven of the last twenty-five trunk runs were
    cancellations, so this is the ordinary case rather than an edge one.
    """
    hook = _load_hook()
    runs = [
        {"status": "completed", "conclusion": "failure"},
        {"status": "completed", "conclusion": "cancelled"},
        {"status": "completed", "conclusion": "failure"},
        {"status": "completed", "conclusion": "success"},
        {"status": "completed", "conclusion": "failure"},
    ]
    assert len(hook.consecutive_failures(runs)) == 2


def test_a_running_job_cannot_hide_the_last_completed_red_run(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A queued replacement is not evidence that the broken trunk recovered."""
    hook = _load_hook()
    runs = [
        {"status": "in_progress", "conclusion": ""},
        {"status": "completed", "conclusion": "failure"},
        {"status": "completed", "conclusion": "cancelled"},
        {"status": "completed", "conclusion": "failure"},
        {"status": "completed", "conclusion": "success"},
    ]
    monkeypatch.setattr(hook, "trunk_runs", lambda: runs)

    section = hook.ci_section()
    assert "RED" in section and "2 consecutive" in section, section


def test_green_trunk_says_so_briefly(monkeypatch: pytest.MonkeyPatch) -> None:
    hook = _load_hook()
    monkeypatch.setattr(
        hook,
        "trunk_runs",
        lambda: [{"status": "completed", "conclusion": "success"}],
    )
    section = hook.ci_section()
    assert "Green" in section and "RED" not in section
    assert len(section.splitlines()) < 6, "a healthy trunk must not cost screen space"


def test_an_unknown_answer_never_reads_as_green(monkeypatch: pytest.MonkeyPatch) -> None:
    hook = _load_hook()
    monkeypatch.setattr(hook, "trunk_runs", lambda: None)
    section = hook.ci_section()
    assert "Unknown" in section
    assert "Green" not in section, CI_BLINDNESS_RATIONALE + "\nunknown rendered as green"

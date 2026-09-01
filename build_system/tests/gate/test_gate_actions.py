"""The smallest reusable units of gate work.

An action knows two things: what it would do, and how to do it.

The first is why `--dry-run` can print real argv rather than a list of step
names -- a plan whose steps are opaque callables can only say "build assets",
which is a summary, not something you can check before spending forty minutes.

The second is why nothing in this package writes `shutil.copytree` twice. The
justfile had eleven hand-written copies of one storage command and four `case`
blocks over architecture names; the first draft of the Python replacement was
already growing its own second copy of "unpack, inject, repack atomically".
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Action, Run, Script, Shell
from capsem_builder.gate.context import Context, NullJournal
from capsem_builder.gate.errors import GateError
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def runner() -> RecordingRunner:
    return RecordingRunner(PROJECT_ROOT)


@pytest.fixture
def context(runner: RecordingRunner) -> Context:
    return Context(runner, gate_config.load(PROJECT_ROOT))


# ---------------------------------------------------------------------------
# Rendering is what a dry run prints
# ---------------------------------------------------------------------------


def test_run_renders_the_argv_it_would_execute(context: Context) -> None:
    """A dry run that says "build assets" without saying what it invokes is a
    summary. The point is to be able to check the plan before spending on it."""
    action = Run(["cargo", "build", "--release"])

    assert action.render() == "cargo build --release"


def test_rendering_executes_nothing(context: Context, runner: RecordingRunner) -> None:
    """A dry run with side effects is not a dry run."""
    Run(["docker", "system", "prune", "-af"]).render()
    Script(context.config, "build_system/scripts/build/clean_stale.py").render()
    Shell("rm -rf cache/target/*").render()

    assert runner.commands == []


def test_render_quotes_what_a_shell_would_need_quoted() -> None:
    """The rendering is meant to be runnable by hand from the dry run."""
    action = Run(["git", "commit", "-m", "two words"])

    assert action.render() == "git commit -m 'two words'"


def test_render_shows_the_environment_an_action_adds() -> None:
    """An invocation that only behaves that way because of one variable is one
    the reader has to be told about."""
    action = Run(["just", "_test-candidate-run"], env={"CAPSEM_TEST_MODULE": "fast"})

    assert action.render() == "CAPSEM_TEST_MODULE=fast just _test-candidate-run"


# ---------------------------------------------------------------------------
# Performing
# ---------------------------------------------------------------------------


def test_run_invokes_through_the_runner(context: Context, runner: RecordingRunner) -> None:
    """Every invocation goes through the one funnel, so the run log sees it."""
    Run(["cargo", "build"]).perform(context)

    assert runner.rendered == ["cargo build"]


def test_a_failing_command_raises_and_names_itself(context: Context) -> None:
    """An exit status nobody reads is the defect this replaces: the asset lanes
    collected each lane's status into a variable and never checked it."""
    context.runner.fail_on("cargo build")

    with pytest.raises(GateError, match="cargo build"):
        Run(["cargo", "build"]).perform(context)


def test_an_unchecked_command_tolerates_failure(context: Context) -> None:
    """Cleanup runs against whatever state the failure left behind, so its own
    exit status is not evidence of anything."""
    context.runner.fail_on("docker rm")

    Run(["docker", "rm", "-f", "stale"], check=False).perform(context)


def test_the_context_environment_reaches_the_command(
    runner: RecordingRunner,
) -> None:
    """A workspace exports CAPSEM_HOME once; every action inside it inherits
    that rather than each one remembering to pass it."""
    context = Context(runner, gate_config.load(PROJECT_ROOT)).with_env(
        CAPSEM_HOME="/tmp/isolated"
    )

    Run(["capsem", "status"]).perform(context)

    assert "CAPSEM_HOME=/tmp/isolated" in runner.rendered[0]


def test_an_action_can_override_the_context_environment(
    runner: RecordingRunner,
) -> None:
    """The narrower scope wins, so one lane can differ without a new context."""
    context = Context(runner, gate_config.load(PROJECT_ROOT)).with_env(RUST_LOG="info")

    Run(["capsem-service"], env={"RUST_LOG": "debug"}).perform(context)

    assert "RUST_LOG=debug" in runner.rendered[0]


def test_with_env_leaves_the_parent_context_alone(runner: RecordingRunner) -> None:
    """Two concurrent steps share a parent context; one adding a variable must
    not change what the other sees."""
    parent = Context(runner, gate_config.load(PROJECT_ROOT))

    child = parent.with_env(CAPSEM_HOME="/tmp/isolated")

    assert parent.env == {}
    assert child.env == {"CAPSEM_HOME": "/tmp/isolated"}


# ---------------------------------------------------------------------------
# Scripts and shell
# ---------------------------------------------------------------------------


def test_a_script_runs_through_the_projects_environment(
    context: Context, runner: RecordingRunner
) -> None:
    """A raw `python3 tool.py` picks up whatever interpreter is on PATH, which
    on a release runner is not the one the lockfile pins."""
    Script(context.config, "build_system/scripts/build/clean_stale.py", "--force").perform(context)

    assert runner.rendered[0].startswith("uv run --project build_system --frozen python ")
    assert runner.rendered[0].endswith("clean_stale.py --force")


def test_a_script_renders_the_relative_path_it_was_given(context: Context) -> None:
    """The absolute path is noise in a dry run; the checkout is implied."""
    assert Script(context.config, "build_system/scripts/build/clean_stale.py").render() == (
        "uv run --project build_system --frozen python build_system/scripts/build/clean_stale.py"
    )


def test_shell_is_for_fragments_where_the_shell_is_the_point(
    context: Context, runner: RecordingRunner
) -> None:
    Shell("docker images -q | xargs -r docker rmi").perform(context)

    assert runner.commands[0].argv[:2] == ("bash", "-c")


# ---------------------------------------------------------------------------
# The subclass contract
# ---------------------------------------------------------------------------


def test_an_action_subclass_must_name_itself() -> None:
    """The name is what the run log records the action as."""
    with pytest.raises(TypeError, match="name"):
        # The omission is the subject of the test.
        class Unnamed(Action):  # ty: ignore[missing-argument]
            def render(self) -> str:
                return ""

            def perform(self, context: Context) -> None: ...


def test_every_action_renders_and_performs() -> None:
    """Both halves are abstract: an action that cannot describe itself breaks
    the dry run, and one that cannot run breaks the gate."""
    class Halfway(Action, name="halfway"):
        def render(self) -> str:
            return ""

    with pytest.raises(TypeError, match="perform"):
        Halfway()  # ty: ignore[invalid-argument-type]


def test_the_context_resolves_paths_against_the_checkout(context: Context) -> None:
    """So an action is handed a resolved path and never joins one itself."""
    assert context.root == PROJECT_ROOT
    assert context.path("config") == PROJECT_ROOT / "config"


def test_a_null_journal_absorbs_what_an_action_reports() -> None:
    """So an action can be exercised without a run behind it."""
    journal = NullJournal()

    journal.note("nothing is listening")
    journal.artifact(Path("vmlinuz"), digest="cafe", size=1)

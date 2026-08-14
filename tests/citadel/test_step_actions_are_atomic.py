"""Citadel guard: a step may not hide a compiler behind its name.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one records a measurement failure rather than a correctness one,
which is why it survived two months of green runs.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest
from helpers.gate import gate_plan

from capsem.gate.execution import Kind
from capsem.gate.shellnodes import Command, arm_named, commands
from capsem.gate.shellparse import parse

ROOT = Path(__file__).resolve().parents[2]

#: The commands whose steps must hold. `candidate` is the whole gate; the fast
#: lane is where a hidden build costs the most, because speed is its promise.
COMMANDS = ("candidate", "test-fast", "test-static")

#: The claim a step takes when it drives cargo. Cargo locks its target
#: directory, so two invocations serialise whether or not anyone declared it.
WORKSPACE_CLAIM = "workspace_binaries"

#: Cargo subcommands that build. `fmt` and `--version` do not, and a step that
#: only formats has no business holding the target directory.
BUILDS = frozenset({"build", "run", "test", "clippy", "check", "nextest", "llvm-cov", "install"})

#: Kinds that assert no build happens. A step reaching a compiler may hold any
#: other kind -- an end-to-end proof that builds what it proves is honestly
#: `E2E` -- but not one of these.
BUILDS_NOTHING = frozenset({Kind.LINT, Kind.STATIC_TEST})

ATOMICITY_RATIONALE = """\
A step is the unit the gate measures, schedules, declares and reports. When one
action shells into a script that also runs a compiler, every one of those
instruments goes blind at the same moment: the timing report prints one line,
the declaration describes whatever the label says, and the scheduler holds the
step's claims for a duration nobody can attribute.

`web.release-site` ran for one minute fifty-nine. Astro's type-check took no
measurable time and vitest took one second; the rest was
`cargo run -p capsem-admin` building a binary the fast lane does not otherwise
build. Four instruments looked straight at it. The label said "web". The timing
summary showed a single opaque line. `slow_action_seconds` flagged it every
run, so it read as furniture. And nothing declared what the step was, so there
was nothing for the number to contradict.

What makes it a Citadel entry rather than a bug is that no instrument was
broken. Each reported exactly what it measured. The step simply was not the
unit anyone thought it was.

What is checked is the *claim*. `web.release-site` did declare `COMPILE` -- by
accident, from a comprehension that declared it for four surfaces at once -- so
a rule about `kind` would have passed it. What it could not do honestly was
hold `astro_build` alone while driving cargo against the workspace target
directory, serialising against every other build on the machine through a lock
nothing had declared.

Read with a parser, not a pattern. Three earlier versions of this guard each
reported a clean tree: one searched only the shell and the cargo call was in a
Python script the shell invoked; one checked `kind`, which was already right by
accident; one matched `cargo` textually and could not tell a command from a
filename, a comment, or the left side of an assignment.

See src/capsem/gate/shellparse.py and skills/dev-gate/SKILL.md.
"""


def cargo_builds(argv: tuple[str, ...]) -> bool:
    """Whether this argv is a cargo invocation that builds something.

    For a plain argv -- a step's own command, or a Python list literal -- where
    there is no shell to strip assignments and wrappers.
    """
    return len(argv) > 1 and argv[0] == "cargo" and argv[1] in BUILDS


def command_builds(command: Command) -> bool:
    """The same question of a parsed command, which knows more.

    `CARGO_TARGET_DIR=/tmp env cargo build` has `cargo` at neither argv[0] nor
    argv[1]; the parser already resolves the assignment prefix and the wrapper,
    so asking it beats re-deriving that here and getting it wrong.
    """
    return command.program == "cargo" and command.subcommand() in BUILDS


def python_argvs(source: str) -> list[tuple[str, ...]]:
    """Every list-of-strings literal in a Python file, as argv.

    The form the original bug was in: `check-web-surface.sh release-site` named
    no compiler at all, and the `["cargo", "run", ...]` that cost two minutes
    was built inside the Python script it called. Read with `ast` for the same
    reason the shell is read with a parser -- a string in a list literal is not
    the same thing as the word `cargo` appearing in a file.
    """
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return []
    found = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.List | ast.Tuple):
            continue
        words = [
            item.value
            for item in node.elts
            if isinstance(item, ast.Constant) and isinstance(item.value, str)
        ]
        if words:
            found.append(tuple(words))
    return found


def reaches_compiler(path: Path, arm: str | None, seen: frozenset[Path]) -> bool:
    """Whether running `path` (optionally one arm of it) builds Rust.

    Follows hand-offs: a step names a dispatcher, the dispatcher names a
    program, the program builds. Stopping at the first file is what let a
    two-minute Rust build read as a web check.
    """
    source = path.read_text(encoding="utf-8")
    if path.suffix == ".py":
        argvs = python_argvs(source)
        return any(cargo_builds(argv) for argv in argvs)

    tree = parse(source)
    body = tree if arm is None else arm_named(tree, arm)
    if body is None:
        return False
    found: list[Command] = commands(body)
    if any(command_builds(command) for command in found):
        return True
    for command in found:
        for word in command.argv:
            nested = ROOT / word
            if nested in seen or nested.suffix not in {".sh", ".py"} or not nested.is_file():
                continue
            if reaches_compiler(nested, None, seen | {nested}):
                return True
    return False


def argv_of(action) -> tuple[str, ...]:
    """Every action funnels through a `Command`; its argv is what runs."""
    command = getattr(action, "_command", None)
    argv = getattr(command, "argv", ())
    return tuple(str(item) for item in argv)


def script_of(argv: tuple[str, ...]) -> tuple[Path, str | None] | None:
    for index, token in enumerate(argv):
        candidate = ROOT / token
        if token.endswith((".sh", ".py")) and candidate.is_file():
            return candidate, (argv[index + 1] if index + 1 < len(argv) else None)
    return None


def hidden_builds(command: str) -> list[str]:
    offenders = []
    plan = gate_plan(command)
    for label in plan.labels:
        step = plan.step_named(label)
        for action in step.actions:
            argv = argv_of(action)
            if not argv:
                continue
            where = f"{label}: {' '.join(argv[:3])}"
            if cargo_builds(argv):
                pass  # cargo on the command line, as `fast.clippy` runs it
            elif (found := script_of(argv)) is not None:
                path, arm = found
                if not reaches_compiler(path, arm, frozenset({path})):
                    continue
                where = f"{label}: {path.name}" + (f" {arm}" if arm else "")
            else:
                continue
            if step.kind in BUILDS_NOTHING:
                offenders.append(
                    f"{where} reaches cargo but declares kind={step.kind.value}, "
                    "which asserts it builds nothing"
                )
            if not any(claim.name == WORKSPACE_CLAIM for claim in step.contends):
                offenders.append(f"{where} reaches cargo without claiming {WORKSPACE_CLAIM}")
    return offenders


@pytest.mark.parametrize("command", COMMANDS)
def test_a_step_that_compiles_rust_declares_it(command: str) -> None:
    offenders = hidden_builds(command)
    assert not offenders, ATOMICITY_RATIONALE + "\n" + "\n".join(offenders)


def test_the_guard_can_tell_a_command_from_a_mention() -> None:
    """The distinctions a pattern could not make, asserted directly.

    Each of these once produced a wrong answer: a variable named `cargo`, a
    build named in a comment, a filename containing the word, and a
    subcommand that compiles nothing.
    """
    assert cargo_builds(("cargo", "build", "-p", "x"))
    assert not cargo_builds(("cargo", "fmt", "--check")), "formatting builds nothing"
    assert not cargo_builds(("pytest", "tests/test_cargo_build.py")), "a filename is not a build"

    def builds(source: str) -> bool:
        return any(command_builds(item) for item in commands(parse(source)))

    assert builds("cargo build -p x")
    assert builds("(cd $ROOT && cargo run -p capsem-admin)"), "a subshell still compiles"
    assert builds("CARGO_TARGET=/tmp env cargo build"), "prefixes and wrappers are stepped over"
    assert not builds("# cargo build -p x"), "a comment is not a build"
    assert not builds('cargo="$(command -v cargo)"'), "an assignment is not a build"
    assert not builds('echo "cargo build"'), "a quoted argument is not a build"


def test_the_guard_catches_the_bug_it_was_written_for() -> None:
    """The exact arrangement of `web.release-site`, end to end.

    Worth its own test because three earlier versions of this guard missed it,
    each for a different reason and each reading as clean. A guard that misses
    its own founding case is decoration.
    """
    channel = ROOT / "scripts" / "build-complete-release-channel.py"
    assert channel.is_file(), "the script the original bug hid behind has moved"
    assert any(cargo_builds(argv) for argv in python_argvs(channel.read_text())), (
        "cargo built as a Python argv list is no longer detected"
    )

    surface = ROOT / "scripts" / "check-web-surface.sh"
    arm = arm_named(parse(surface.read_text()), "release-channel")
    assert arm is not None, "the arm that carries the parity proof is gone"
    assert not any(command_builds(item) for item in commands(arm)), (
        "the shell arm itself never named cargo -- that was the whole problem"
    )
    assert reaches_compiler(surface, "release-channel", frozenset({surface})), (
        "the guard must follow the hand-off into Python to find it"
    )

    assert Kind.COMPILE not in BUILDS_NOTHING, (
        "the original step declared COMPILE and was still wrong; if the kind "
        "rule ever becomes the whole guard, it stops catching this"
    )


def test_an_arm_the_step_does_not_select_is_not_charged_to_it() -> None:
    """A dispatcher holds many steps' work; a step answers for its own arm.

    Charging `web.release-site` for the cargo in the `release-channel` arm
    would make the guard unactionable, and an unactionable guard gets deleted.
    """
    surface = ROOT / "scripts" / "check-web-surface.sh"
    assert not reaches_compiler(surface, "release-site", frozenset({surface}))
    assert not reaches_compiler(surface, "docs", frozenset({surface}))

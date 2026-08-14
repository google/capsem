"""Citadel guard: a step may not hide a compiler behind its name.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one records a measurement failure rather than a correctness one,
which is why it took two months to notice.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from helpers.gate import gate_plan

from capsem.gate.execution import Kind

ROOT = Path(__file__).resolve().parents[2]

#: The commands whose steps must hold. `candidate` is the whole gate; the fast
#: lane is where a hidden build costs the most, because speed is its promise.
COMMANDS = ("candidate", "test-fast", "test-static")

#: The claim a step takes when it drives cargo. Cargo locks its target
#: directory, so two invocations serialise whether or not anybody declared it.
WORKSPACE_CLAIM = "workspace_binaries"

ATOMICITY_RATIONALE = """\
A step is the unit the gate measures, schedules, declares and reports. When one
action shells into a script that also runs a compiler, every one of those
instruments goes blind at the same moment: the timing report prints one line,
the declaration describes whatever the label says, and the scheduler holds the
step's claims for a duration nobody can attribute.

`web.release-site` ran for one minute fifty-nine. Astro's type-check took no
measurable time and vitest took one second; the rest was
`cargo run -p capsem-admin` building a binary the fast lane does not otherwise
build. Four separate instruments looked straight at it. The label said "web".
The timing summary showed a single opaque line. `slow_action_seconds` flagged
it every run, so it read as furniture. And nothing declared what the step was,
so there was nothing for the number to contradict.

What makes it a Citadel entry rather than a bug is that no instrument was
broken. Each reported exactly what it measured. The step was simply not the
unit anybody thought it was, and a measurement of the wrong unit is not wrong
-- it is unfalsifiable.

So the declaration is checked against the script the action actually runs, and
what is checked is the *claim*. `web.release-site` did declare `COMPILE` -- by
accident, from a comprehension that declared it for four surfaces at once -- so
a rule about `kind` would have passed it. What it could not do honestly was
hold `astro_build` alone while driving cargo against the workspace target
directory. Cargo locks that directory, so the step was serialising against
every other build on the machine through a lock nothing had declared, and the
wait was charged to its own execution time.

A step that reaches a compiler therefore claims the workspace, and may not
declare a `kind` that asserts it builds nothing.

See src/capsem/gate/audits.py and skills/dev-gate/SKILL.md.
"""

#: Kinds that assert no build happens. A step reaching a compiler may hold any
#: other kind -- an end-to-end proof that builds what it proves is honestly
#: `E2E` -- but not one of these.
BUILDS_NOTHING = frozenset({Kind.LINT, Kind.STATIC_TEST})

#: `cargo` as a shell command, not as a substring of a path or a comment.
CARGO = re.compile(r"(?:^|[\s(|&;])cargo\s+(?P<sub>[a-z-]+)")

#: `cargo` as the head of an argv list in Python: `["cargo", "run", ...]`,
#: usually spread over as many lines. This is not a refinement -- it is the
#: form the original bug was in. `check-web-surface.sh release-site` held no
#: `cargo` at all; it ran a Python script that built the argv, and a guard
#: reading only the shell would have called that step clean.
CARGO_ARGV = re.compile(r"""["']cargo["']\s*,\s*["'](?P<sub>[a-z-]+)["']""", re.S)

#: A script one script hands off to. Followed so the guard sees through a
#: dispatcher into the program that does the work.
HANDOFF = re.compile(r"(?P<path>scripts/[\w.-]+\.(?:sh|py))")

#: Cargo subcommands that build something. `cargo fmt` and `cargo --version`
#: do not, and a step that only formats has no business claiming the target
#: directory.
BUILDS = frozenset({"build", "run", "test", "clippy", "check", "nextest", "llvm-cov", "install"})


def script_arm(body: str, arm: str | None) -> str:
    """The one `case` branch a step selects, or the whole file.

    A dispatcher script is many steps' worth of work in one file; charging a
    step for a compiler in a branch it never takes would make this guard
    unactionable, which is how guards get deleted.
    """
    if not arm:
        return body
    match = re.search(rf"^\s*{re.escape(arm)}\)\n(.*?)^\s*;;", body, re.S | re.M)
    return match.group(1) if match else body


def uncommented(body: str) -> str:
    return "\n".join(
        line for line in body.splitlines() if not line.strip().startswith("#")
    )


def compiles(body: str) -> bool:
    """Whether this text runs a cargo subcommand that builds something."""
    text = uncommented(body)
    return any(
        hit.group("sub") in BUILDS
        for pattern in (CARGO, CARGO_ARGV)
        for hit in pattern.finditer(text)
    )


def reaches_compiler(body: str, seen: frozenset[Path] = frozenset()) -> bool:
    """`compiles`, following the scripts this text hands off to.

    A step's action names a dispatcher; the dispatcher names a program; the
    program builds. Stopping at the first file is what made the original
    two-minute Rust build look like a web check, so the guard walks the chain.
    `seen` keeps a script that invokes itself from recursing.
    """
    if compiles(body):
        return True
    for hit in HANDOFF.finditer(uncommented(body)):
        path = ROOT / hit.group("path")
        if path in seen or not path.is_file():
            continue
        if reaches_compiler(path.read_text(), seen | {path}):
            return True
    return False


def script_of(argv: tuple[str, ...]) -> tuple[Path, str | None] | None:
    """The script an action shells into, and the sub-command it passes."""
    for index, token in enumerate(argv):
        candidate = ROOT / token
        if token.endswith(".sh") and candidate.is_file():
            arm = argv[index + 1] if index + 1 < len(argv) else None
            return candidate, arm
    return None


def argv_of(action) -> tuple[str, ...]:
    """Every action funnels through a `Command`; its argv is what runs."""
    command = getattr(action, "_command", None)
    argv = getattr(command, "argv", ())
    return tuple(str(item) for item in argv)


def hidden_builds(command: str) -> list[str]:
    plan = gate_plan(command)
    offenders = []
    for label in plan.labels:
        step = plan.step_named(label)
        for action in step.actions:
            found = script_of(argv_of(action))
            if found is None:
                continue
            path, arm = found
            if not reaches_compiler(script_arm(path.read_text(), arm), frozenset({path})):
                continue
            where = f"{label}: {path.name}"
            if arm:
                where += f" {arm}"
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


def test_the_guard_can_see_a_compiler() -> None:
    """Break it here, so a refactor that blinds it fails rather than passes.

    Every part of the detection is exercised against text: the branch split,
    the comment skip, and the build/non-build subcommand division. A guard
    whose extractor silently returns nothing reports a clean tree, and this
    one's extractor is the whole guard.
    """
    dispatcher = "a)\n    cargo build -p x\n    ;;\nb)\n    echo hello\n    ;;\n"
    assert compiles(script_arm(dispatcher, "a"))
    assert not compiles(script_arm(dispatcher, "b")), "a branch the step never takes"
    assert compiles(script_arm(dispatcher, None)), "no arm named: charge the whole file"
    assert not compiles("# cargo build -p x"), "a mention in a comment is not a build"
    assert not compiles("cargo fmt --check"), "formatting builds nothing"
    assert compiles("(cd $ROOT && cargo run -p capsem-admin)"), "a subshell still compiles"
    assert not compiles('cargo = ROOT / "Cargo.toml"'), "a variable named cargo builds nothing"
    assert compiles('command = [\n    "cargo",\n    "run",\n]'), "argv form counts"


def test_the_guard_catches_the_bug_it_was_written_for() -> None:
    """The exact arrangement of `web.release-site`, end to end.

    Worth its own test because the first two versions of this guard missed it,
    each for a different reason, and both would have read as clean.

    The first read only the shell. `check-web-surface.sh release-site`
    contained no `cargo` -- it ran a Python script that built the argv list --
    so the two minutes stayed invisible.

    The second checked that a compiling step declares `kind=COMPILE`, and
    `web.release-site` already did: the comprehension that built four surfaces
    declared it for all of them, and it happened to be right here for the wrong
    reason. The detectable lie was never the kind. It was holding `astro_build`
    -- a lock on Astro's staging directory -- while the real contention was the
    cargo target directory nobody had claimed.

    A guard that misses its own founding case is decoration.
    """
    channel = ROOT / "scripts" / "build-complete-release-channel.py"
    assert channel.is_file(), "the script the original bug hid behind has moved"
    assert compiles(channel.read_text()), "cargo in argv form is no longer detected"

    arm = f"require_astro\nuv run python {channel.relative_to(ROOT)} --out-dir x\n"
    assert not compiles(arm), "the shell arm itself never named cargo -- that was the problem"
    assert reaches_compiler(arm), "the guard must follow the hand-off to find it"

    assert Kind.COMPILE not in BUILDS_NOTHING, (
        "the original step declared COMPILE and was still wrong; if the kind "
        "rule ever becomes the whole guard, it stops catching this"
    )

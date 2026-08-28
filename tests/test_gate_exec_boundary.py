"""`just exec` runs one command in a fresh VM, and only in the VM.

Three independent defects met here, and each was invisible from inside its own
layer.

The CLI subparser stored the subcommand name in `command`, and `ExecCommand`
stored its payload there too, so argparse overwrote the name with a list and
registry dispatch raised `TypeError: cannot use 'list' as a dict key`. The
public command could not run at all.

The recipe interpolated `{{CMD}}` unquoted, so a semicolon, pipe, redirection
or command substitution meant for the guest became host shell syntax first.
`just --dry-run exec 'echo guest; echo HOST'` rendered a second host command.
This is the serious one: text a user believes is going into a sandbox executes
outside it.

And it invoked `capsem exec`, which the Rust CLI defines as "execute a command
in a running session" and which takes a session argument. The one-shot
fresh-VM operation `just exec` documents is `capsem run`.

The payload crosses three layers, so these assert at each boundary rather than
mocking before the risky handoff -- which is what let all three sit here.
"""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import cli
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

#: Every one of these is ordinary in a guest and dangerous in a host shell.
PAYLOADS = [
    "echo hello",
    "echo 'single quoted'",
    'echo "double quoted"',
    "echo one; echo two",
    "cat /etc/hostname | wc -l",
    "echo out > /tmp/guest-file",
    "echo $(whoami)",
    "echo `hostname`",
    "grep -r 'needle' /workspace && echo found",
    "python3 -c 'print(1 + 1)'",
]


def _planned(payload: str) -> RecordingRunner:
    """Drive the real command through the real parser, as the recipe does."""
    runner = RecordingRunner(PROJECT_ROOT)
    # `--` exactly as the recipe passes it: without it a payload beginning with
    # a dash is claimed by argparse, and `just exec --help` prints the gate's
    # own help instead of running anything in the guest.
    args = cli.build_parser().parse_args(["exec", "--", payload])
    GateCommand.registry[args.gate_command](runner, args).plan().run(
        _context(runner)
    )
    return runner


def _context(runner: RecordingRunner):
    from capsem_builder.gate.context import Context

    return Context(runner, CONFIG)


# ---------------------------------------------------------------------------
# The parser
# ---------------------------------------------------------------------------


def test_the_subcommand_name_survives_a_commands_own_arguments() -> None:
    """`dest="command"` collided with `ExecCommand`'s positional.

    Registry lookup then indexed a dict with a list. Nothing about the failure
    pointed at the parser, which is why it survived.
    """
    args = cli.build_parser().parse_args(["exec", "echo hello"])

    assert args.gate_command == "exec"
    assert GateCommand.registry[args.gate_command] is not None


def test_no_command_stores_an_argument_where_the_subcommand_name_goes() -> None:
    """The general form, so the next command cannot repeat it."""
    for name, command in sorted(GateCommand.registry.items()):
        parser = argparse.ArgumentParser()
        command.add_arguments(parser)
        stored = {action.dest for action in parser._actions}

        assert "gate_command" not in stored, (
            f"{name} stores an argument in the subcommand slot"
        )


# ---------------------------------------------------------------------------
# The payload, unchanged, all the way to argv
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("payload", PAYLOADS)
def test_the_guest_command_reaches_the_vm_exactly_as_written(payload: str) -> None:
    runner = _planned(payload)

    (issued,) = runner.commands
    assert issued.argv[1:] == ("run", payload), (
        f"the payload was reshaped on the way: {issued.argv!r}"
    )


@pytest.mark.parametrize("payload", PAYLOADS)
def test_no_host_shell_ever_receives_the_payload(payload: str) -> None:
    """A `bash -c` anywhere on this path is the vulnerability itself."""
    runner = _planned(payload)

    for issued in runner.commands:
        assert issued.argv[0] not in {"bash", "sh", "zsh"}
        assert "-c" not in issued.argv[:2]


def test_it_runs_a_fresh_session_rather_than_an_existing_one() -> None:
    """`capsem exec` needs a session argument and means something else.

    `just exec` documents a one-shot temporary VM; that is `capsem run`
    (crates/capsem/src/main.rs). Invoking `exec` with no session would have
    consumed the payload as the session name.
    """
    runner = _planned("echo hello")

    (issued,) = runner.commands
    assert issued.argv[0].endswith(Path(CONFIG.logs.cli).name)
    assert issued.argv[1] == "run"


def test_a_payload_that_looks_like_a_flag_is_still_a_payload() -> None:
    """Otherwise argparse claims it and the user gets the gate's own help."""
    runner = _planned("--help")

    (issued,) = runner.commands
    assert issued.argv[1:] == ("run", "--help")


# ---------------------------------------------------------------------------
# The just boundary, which is where the quoting is actually decided
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "payload",
    ["echo one; echo two", "echo $(whoami)", "cat /etc/hostname | wc -l"],
)
def test_the_recipe_hands_the_payload_over_as_one_argument(payload: str) -> None:
    """Asserted through `just` itself, not by reading the recipe text.

    The defect lived in how `just` interpolates, so a test that inspects the
    source and not the interpolation would have passed throughout.
    """
    rendered = subprocess.run(
        ["just", "--dry-run", "exec", payload],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    line = next(
        line
        for line in (rendered.stderr + rendered.stdout).splitlines()
        if "capsem-gate exec" in line
    )

    _prefix, _, tail = line.partition("capsem-gate exec ")
    assert tail.strip() in {f"-- '{payload}'", f'-- "{payload}"'}, (
        f"the payload reached the host shell unquoted: {line}"
    )

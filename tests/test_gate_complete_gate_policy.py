"""Everything that contains the complete gate shares its lifecycle policy.

Keep-awake was `CandidateCommand`'s, because the gate was candidate's. Both
release commands used to reach it by launching `just test`; deleting that child
command was right, but it left them owning the same forty-minute qualification
with none of the wrapper. On macOS an unattended release could sleep in the
middle of it -- during qualification, or during publication.

The guard that existed said "only candidate re-execs", which described the
implementation rather than the policy, so it passed while the hole opened.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import candidate, cli  # noqa: F401 - importing registers every command
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

#: Every command whose plan contains the complete qualification gate.
COMPLETE_GATE = {
    "candidate": {},
    "release-binaries": {"channel": "nightly"},
    "release-profile": {"channel": "nightly", "profile": "code"},
}


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )


@pytest.fixture
def macos(monkeypatch):
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/caffeinate")
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_every_complete_gate_command_keeps_the_host_awake(name, macos) -> None:
    replacement = _command(name, **COMPLETE_GATE[name]).reexec()

    assert replacement is not None, f"{name} runs the complete gate but lets macOS sleep through it"
    assert list(replacement[: len(CONFIG.candidate.keep_awake_command)]) == list(
        CONFIG.candidate.keep_awake_command
    )


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_the_wrapper_preserves_the_operators_exact_invocation(name, macos) -> None:
    """A wrapper wraps what it was given. Substituting a recipe drops flags.

    The *arguments*, not `sys.argv` whole. `capsem-gate` re-execs itself with
    `-m capsem.gate` to get an isolated bytecode cache, so `sys.argv[0]` here
    is the path of `__main__.py`. Asserting the tail equalled `sys.argv`
    endorsed passing that to `caffeinate`, which cannot execute a `.py` file --
    `env: __main__.py: Permission denied`, three seconds into a gate.
    """
    replacement = list(_command(name, **COMPLETE_GATE[name]).reexec())

    arguments = sys.argv[1:]
    assert replacement[-len(arguments) :] == arguments

    # Everything before the operator's own arguments: the keep-awake prefix,
    # then this interpreter running this module. Checked as a slice rather
    # than by scanning the whole line, because under pytest the arguments are
    # themselves a list of `.py` paths.
    wrapper = replacement[: -len(arguments)]
    assert wrapper[-3:] == [sys.executable, "-m", candidate.MODULE]
    assert not any(part.endswith(".py") for part in wrapper), (
        f"a re-exec must name a program, not a source file: {wrapper}"
    )


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_the_wrapper_is_applied_exactly_once(name, macos, monkeypatch) -> None:
    """The marker the wrapper exports stops the child wrapping itself."""
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")

    assert _command(name, **COMPLETE_GATE[name]).reexec() is None


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_linux_needs_no_wrapper(name, monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")

    assert _command(name, **COMPLETE_GATE[name]).reexec() is None


def test_the_policy_is_stated_as_policy_not_as_one_command_name() -> None:
    """Anything else re-execing is still a thing to justify."""
    replacing = sorted(
        name
        for name, cls in GateCommand.registry.items()
        if "reexec" in vars(cls) and cls.__module__.startswith("capsem.gate.")
    )

    assert set(replacing) <= set(COMPLETE_GATE), (
        f"{sorted(set(replacing) - set(COMPLETE_GATE))} replace themselves "
        "without containing the complete gate"
    )

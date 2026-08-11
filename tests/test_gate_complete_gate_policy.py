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

from capsem.gate import candidate, cli, sandbox  # noqa: F401 - importing registers commands
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


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_every_complete_gate_keeps_the_enforcing_policy(name: str) -> None:
    """Candidate and both macOS/Linux release wrappers share one declaration."""
    assert GateCommand.registry[name].sandboxed is sandbox.ENFORCE


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )


@pytest.fixture
def macos(monkeypatch):
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/caffeinate")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: False)
    monkeypatch.delenv(CONFIG.candidate.keep_awake_marker, raising=False)


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_every_complete_gate_command_keeps_the_host_awake(name, macos) -> None:
    replacement = _command(name, **COMPLETE_GATE[name]).reexec()

    assert replacement is not None, f"{name} runs the complete gate but lets macOS sleep through it"
    # Present, not first. `candidate` now declares an enforcing sandbox, so
    # `sandbox.applied` wraps the keep-awake argv rather than the other way
    # round -- a profile is inherited and irrevocable, so it has to be the
    # outermost thing or the process that adopts it is not the one that runs
    # the gate. What matters is that the machine still cannot sleep through an
    # unattended run, which is the argv containing the command, wherever it is.
    assert list(CONFIG.candidate.keep_awake_command) == [
        part
        for part in replacement
        if part in set(CONFIG.candidate.keep_awake_command)
    ][: len(CONFIG.candidate.keep_awake_command)], (
        f"{name} lost its keep-awake wrapper: {replacement}"
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

    # The interpreter, the module, then whatever the operator passed. Built as
    # one tail rather than sliced by `len(arguments)`: under `xdist` a worker's
    # `sys.argv[1:]` is empty, and `replacement[-0:]` is the *whole list*, so
    # the assertion compared everything to nothing and this failed only in the
    # broad parallel suite.
    tail = [sys.executable, "-m", candidate.MODULE, *sys.argv[1:]]
    assert replacement[-len(tail) :] == tail

    # Everything before it is the keep-awake prefix. Checked as a slice rather
    # than by scanning the whole line, because under pytest the operator's own
    # arguments are themselves a list of `.py` paths.
    wrapper = replacement[: -len(tail)]
    assert not any(part.endswith(".py") for part in wrapper), (
        f"a re-exec must name a program, not a source file: {wrapper}"
    )


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_the_wrapper_is_applied_exactly_once(name, macos, monkeypatch) -> None:
    """Kernel state, not a forgeable environment marker, stops rewrapping."""
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: True)

    assert _command(name, **COMPLETE_GATE[name]).reexec() is None


def test_forging_the_keep_awake_marker_cannot_disable_the_macos_sandbox(
    macos, monkeypatch
) -> None:
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")

    replacement = _command("candidate").reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.command


def test_linux_candidate_gets_the_kernel_enforced_network_wrapper(monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: False)

    replacement = _command("candidate").reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.linux_command
    assert "--unshare-net" in replacement


@pytest.mark.parametrize("name", ["release-binaries", "release-profile"])
def test_linux_release_qualification_gets_the_kernel_wrapper(name, monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: False)
    monkeypatch.setattr("capsem.gate.sandbox.prepare_egress", lambda *_args: None)

    replacement = _command(name, **COMPLETE_GATE[name]).reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.linux_command
    assert "--unshare-net" in replacement


@pytest.mark.parametrize(
    "name",
    [
        "test-fast",
        "test-static",
        "test-artifacts",
        "test-functional",
        "test-glowup",
    ],
)
def test_release_ci_modules_declare_the_same_kernel_boundary(name) -> None:
    assert GateCommand.registry[name].sandboxed == sandbox.ENFORCE


def test_linux_kernel_wrapper_is_applied_exactly_once(monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: True)

    assert _command("candidate").reexec() is None


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

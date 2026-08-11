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
from capsem.gate.errors import GateError
from capsem.gate.qualification import LocalQualification

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

#: Every command whose plan contains the complete qualification gate.
COMPLETE_GATE = {
    "candidate": {},
    "release-binaries": {"channel": "nightly"},
    "release-profile": {"channel": "nightly", "profile": "code"},
}

PRIVATE_MODULES = (
    "test-fast",
    "test-static",
    "test-artifacts",
    "test-functional",
    "test-glowup",
    "test-release-contracts",
)


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_every_complete_gate_keeps_the_enforcing_policy(name: str) -> None:
    """Candidate and both macOS/Linux release wrappers share one declaration."""
    command = GateCommand.registry[name]

    assert issubclass(command, candidate.CompleteGate)
    assert command.complete_qualification is True
    assert command.sandboxed is sandbox.ENFORCE


def test_the_complete_qualification_declaration_inventory_is_exact() -> None:
    """The declaration and mixin are one policy, not two drifting lists."""
    declared = {
        name for name, command in GateCommand.registry.items() if command.complete_qualification
    }
    composed = {
        name
        for name, command in GateCommand.registry.items()
        if issubclass(command, candidate.CompleteGate)
    }

    assert declared == composed == set(COMPLETE_GATE)


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
        qualification=LocalQualification(bin_dir=CONFIG.modules.default_bin_dir),
    )


@pytest.mark.parametrize("mode", [sandbox.OFF, sandbox.REPORT])
@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_complete_qualification_refuses_unenforced_sandbox_before_planning(
    name: str,
    mode: sandbox.SandboxMode,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Permissive measurement can never leave evidence named qualification."""
    reached: list[str] = []
    command = _command(name, sandbox=mode, **COMPLETE_GATE[name])

    def unexpected(label: str):
        def reached_boundary(*_args, **_kwargs):
            reached.append(label)
            raise AssertionError(f"{label} ran before the sandbox refusal")

        return reached_boundary

    monkeypatch.setattr(command, "_describe", unexpected("plan"))
    monkeypatch.setattr(command, "reexec", unexpected("reexec"))
    monkeypatch.setattr(command, "resources", unexpected("resources"))

    with pytest.raises(GateError, match=rf"{name}.*complete qualification.*enforce"):
        command.execute()

    assert reached == []
    assert command._runner.commands == []


def test_complete_qualification_defaults_to_enforcement() -> None:
    """The safe mode is declaration-owned, not an operator convention."""
    commands = tuple(_command(name, **arguments) for name, arguments in COMPLETE_GATE.items())

    assert all(command.complete_qualification for command in commands)
    assert all(command._sandbox_mode is sandbox.ENFORCE for command in commands)


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_complete_qualification_accepts_explicit_enforcement(
    name: str,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The refusal distinguishes enforcement from both permissive modes."""
    command = _command(name, sandbox=sandbox.ENFORCE, **COMPLETE_GATE[name])

    class PlanReached(Exception):
        pass

    def reached_plan():
        raise PlanReached

    monkeypatch.setattr(command, "_describe", reached_plan)

    with pytest.raises(PlanReached):
        command.execute()

    assert command._runner.commands == []


@pytest.mark.parametrize("name", ["test-fast", "test-static"])
@pytest.mark.parametrize("mode", [sandbox.OFF, sandbox.REPORT])
def test_a_module_can_still_measure_without_claiming_complete_qualification(
    name: str,
    mode: sandbox.SandboxMode,
) -> None:
    """Report/off remain diagnostic modes for explicitly incomplete evidence."""
    command = _command(name, sandbox=mode)

    assert command.complete_qualification is False
    assert command._sandbox_mode is mode
    sandbox.require_complete_qualification(
        command.name, command._sandbox_mode, command.complete_qualification
    )
    command._describe()


def _just_recipe(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = next(index for index, line in enumerate(lines) if line.startswith(f"{name}:"))
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index] and not lines[index][0].isspace()
        ),
        len(lines),
    )
    return "\n".join(lines[start:end])


def test_private_just_module_entrypoints_do_not_override_the_sandbox() -> None:
    for module in PRIVATE_MODULES:
        recipe = _just_recipe(f"_{module}")
        assert f"capsem-gate {module}" in recipe
        assert "--sandbox" not in recipe, f"_{module} overrides its command-owned boundary"


def test_ci_module_entrypoints_do_not_override_the_sandbox() -> None:
    inspected: list[str] = []
    for workflow in sorted((PROJECT_ROOT / ".github" / "workflows").glob("*.yaml")):
        source = workflow.read_text(encoding="utf-8")
        if not any(
            f"just _{module}" in source or f"capsem-gate {module}" in source
            for module in PRIVATE_MODULES
        ):
            continue
        inspected.append(workflow.name)
        assert "--sandbox" not in source, (
            f"{workflow.name} overrides the command-owned module sandbox boundary"
        )

    assert inspected, "no workflow module entrypoints were inspected"


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


@pytest.mark.parametrize("name", PRIVATE_MODULES)
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

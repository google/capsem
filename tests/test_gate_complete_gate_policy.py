"""Qualification producers and consumers both require kernel enforcement.

Only candidate executes the complete graph and owns its keep-awake lifecycle.
Release is a short consumer, but still handles evidence that may authorize
publication, so permissive diagnostic sandbox modes are equally forbidden.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import pytest
import variables
from capsem_builder.gate import candidate, cli, sandbox  # noqa: F401 - importing registers commands
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.command import GateCommand
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.qualification import LocalQualification
from capsem_builder.gate.sourcecommit import SourceCommit
from capsem_builder.gate.sourcestate import RequireSourceUnchanged
from capsem_builder.gate.timingratchet import EnforceTimingRegression, TimingBoundary
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
SOURCE_COMMIT = SourceCommit("0" * 40)

#: Every command whose plan contains the complete qualification gate.
COMPLETE_GATE = {
    "candidate": {},
}
#: Both release commands dispatch self-qualifying hosted lanes.
RELEASES = {
    "release-binaries": {"channel": "stable", "source_commit": SOURCE_COMMIT},
    "release-profile": {
        "channel": "stable",
        "profile": "code",
        "source_commit": SOURCE_COMMIT,
    },
}
ENFORCED = {**COMPLETE_GATE, **RELEASES}

#: (recipe, gate command). The recipe names describe what a module does --
#: source checks versus compiled checks -- while the command names are the
#: gate's own and did not move.
PRIVATE_MODULES = (
    ("_test-source-checks", "test-fast"),
    ("_test-compiled-checks", "test-static"),
    ("_test-artifacts", "test-artifacts"),
    ("_test-functional", "test-functional"),
    ("_test-glowup", "test-glowup"),
    ("_test-release-contracts", "test-release-contracts"),
)

#: What CI is allowed to call. Workflows reach these and nothing else; see
#: `tests/citadel/test_ci_calls_only_public_recipes.py`.
PUBLIC_CI_VERBS = (
    variables.FAST_TEST,
    variables.QUALIFY_ASSETS,
    variables.QUALIFY_BINARIES,
    variables.BUILD_ASSETS,
)


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_every_complete_gate_keeps_the_enforcing_policy(name: str) -> None:
    """Only the command that executes every step claims complete qualification."""
    command = GateCommand.registry[name]

    assert issubclass(command, candidate.CompleteGate)
    assert command.complete_qualification is True
    assert command.sandboxed is sandbox.ENFORCE


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_every_release_dispatches_qualification_under_enforcement(name: str) -> None:
    from capsem_builder.gate.qualificationevidence import QualificationPolicy

    command = GateCommand.registry[name]

    assert not issubclass(command, candidate.CompleteGate)
    assert command.complete_qualification is False
    assert command.qualification_policy is QualificationPolicy.NONE
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
    parsed = {"dry_run": False, "graph": False, "timing": False}
    parsed.update(args)
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(**parsed),
        qualification=LocalQualification(bin_dir=CONFIG.modules.default_bin_dir),
    )


@pytest.mark.parametrize("mode", [sandbox.OFF, sandbox.REPORT])
@pytest.mark.parametrize("name", sorted(ENFORCED))
def test_complete_qualification_refuses_unenforced_sandbox_before_planning(
    name: str,
    mode: sandbox.SandboxMode,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Permissive measurement can never leave evidence named qualification."""
    reached: list[str] = []
    command = _command(name, sandbox=mode, **ENFORCED[name])

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
def test_timing_ratcheting_precedes_every_publication_boundary(name: str) -> None:
    plan = _command(name, **COMPLETE_GATE[name])._describe()
    ordered = list(plan.labels)
    ratchet = ordered.index(TimingBoundary.QUALIFICATION.value)
    actions = plan.step_named(TimingBoundary.QUALIFICATION.value).actions

    assert isinstance(actions[0], RequireSourceUnchanged)
    assert isinstance(actions[-1], EnforceTimingRegression)
    assert ratchet == len(ordered) - 1


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_release_dispatches_lanes_instead_of_composing_a_local_gate(name: str) -> None:
    plan = _command(name, **RELEASES[name])._describe()
    ordered = list(plan.labels)

    assert ordered[0] == "source.worktree-clean"
    assert "source.record" not in ordered
    assert TimingBoundary.QUALIFICATION.value not in ordered
    assert "qualification.accept" not in ordered
    assert ordered.index("source.publish-ref") < ordered.index("release")


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


def test_a_local_resume_still_refuses_a_step_its_plan_does_not_have() -> None:
    """A diagnostic continuation names the bad frontier and its plan."""
    command = _command(
        "candidate",
        dry_run=True,
        prefix=None,
        resume_from="no-such-step",
    )

    with pytest.raises(GateError, match="no step named 'no-such-step'"):
        command.execute()

    assert command._runner.commands == []


def test_candidate_continuation_remains_an_explicit_diagnostic(capsys) -> None:
    command = _command(
        "candidate",
        dry_run=True,
        prefix=None,
        resume_from="artifacts.build-chain",
    )

    command.execute()

    assert "carried" in capsys.readouterr().out
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
    for recipe_name, module in PRIVATE_MODULES:
        recipe = _just_recipe(recipe_name)
        assert f"capsem-gate {module}" in recipe
        assert "--sandbox" not in recipe, f"{recipe_name} overrides its command-owned boundary"


def test_ci_module_entrypoints_do_not_override_the_sandbox() -> None:
    """CI reaches public verbs now, so this asks about those.

    It used to look for `just _<module>`, which no workflow contains any more:
    the inventory went empty and the guard would have passed by finding nothing
    if it did not assert what it inspected.
    """
    inspected: list[str] = []
    for workflow in sorted((PROJECT_ROOT / ".github" / "workflows").glob("*.yaml")):
        source = workflow.read_text(encoding="utf-8")
        if not any(f"just {verb}" in source for verb in PUBLIC_CI_VERBS):
            continue
        inspected.append(workflow.name)
        assert "--sandbox" not in source, (
            f"{workflow.name} overrides the command-owned module sandbox boundary"
        )

    assert inspected, "no workflow module entrypoints were inspected"


@pytest.mark.parametrize("name", sorted(RELEASES))
def test_an_unqualified_channel_still_publishes_under_enforcement(name: str) -> None:
    """Nightly consumes no journal, and must still be sandbox-enforced.

    Enforcement keyed on the qualification policy alone would let the channel
    with *less* human scrutiny publish from a permissive sandbox.
    """
    from capsem_builder.gate.qualificationevidence import QualificationPolicy

    arguments = {**RELEASES[name], "channel": "nightly"}
    command = _command(name, **arguments)

    assert command.publishes is True
    assert command.qualification_policy is not QualificationPolicy.REQUIRE
    assert "qualification.accept" not in list(command._describe().labels)

    with pytest.raises(GateError, match=rf"{name}.*complete qualification.*enforce"):
        _command(name, sandbox=sandbox.OFF, **arguments).execute()


@pytest.fixture
def macos(monkeypatch):
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/caffeinate")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)
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
    assert (
        list(CONFIG.candidate.keep_awake_command)
        == [part for part in replacement if part in set(CONFIG.candidate.keep_awake_command)][
            : len(CONFIG.candidate.keep_awake_command)
        ]
    ), f"{name} lost its keep-awake wrapper: {replacement}"


@pytest.mark.parametrize("name", sorted(COMPLETE_GATE))
def test_the_wrapper_preserves_the_operators_exact_invocation(name, macos) -> None:
    """A wrapper wraps what it was given. Substituting a recipe drops flags.

    The *arguments*, not `sys.argv` whole. `capsem-gate` re-execs itself with
    `-m capsem_builder.gate` to get an isolated bytecode cache, so `sys.argv[0]` here
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
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: True)

    assert _command(name, **COMPLETE_GATE[name]).reexec() is None


def test_forging_the_keep_awake_marker_cannot_disable_the_macos_sandbox(macos, monkeypatch) -> None:
    monkeypatch.setenv(CONFIG.candidate.keep_awake_marker, "1")

    replacement = _command("candidate").reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.command


def test_linux_candidate_gets_the_kernel_enforced_network_wrapper(monkeypatch) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)

    replacement = _command("candidate").reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.linux_command
    assert "--unshare-net" in replacement


@pytest.mark.parametrize("name", ["release-binaries", "release-profile"])
def test_linux_release_dispatch_gets_the_kernel_wrapper(name, monkeypatch) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)
    monkeypatch.setattr("capsem_builder.gate.sandbox.prepare_egress", lambda *_args: None)

    replacement = _command(name, **RELEASES[name]).reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.linux_command
    assert "--unshare-net" in replacement


@pytest.mark.parametrize("name", [command for _recipe, command in PRIVATE_MODULES])
def test_release_ci_modules_declare_the_same_kernel_boundary(name) -> None:
    assert GateCommand.registry[name].sandboxed == sandbox.ENFORCE


def test_linux_kernel_wrapper_is_applied_exactly_once(monkeypatch) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: True)

    assert _command("candidate").reexec() is None


def test_the_policy_is_stated_as_policy_not_as_one_command_name() -> None:
    """Anything else re-execing is still a thing to justify."""
    replacing = sorted(
        name
        for name, cls in GateCommand.registry.items()
        if "reexec" in vars(cls) and cls.__module__.startswith("capsem_builder.gate.")
    )

    assert set(replacing) <= set(COMPLETE_GATE), (
        f"{sorted(set(replacing) - set(COMPLETE_GATE))} replace themselves "
        "without containing the complete gate"
    )


@pytest.mark.parametrize("name", ["release-binaries", "release-profile"])
def test_a_public_release_cannot_carry_its_publication_prerequisites(name: str) -> None:
    """Only candidate qualification has recursively verified resume evidence.

    A public release plan is a short, fresh consumer. Allowing ``--from`` here
    derived authority from graph shape alone and could carry
    ``source.remote-main``, ``precheck`` and the mutable ``channel-source``
    fetch without any prior-attempt evidence.
    """
    command = _command(
        name,
        dry_run=True,
        prefix=None,
        resume_from="source.publish-ref",
        **RELEASES[name],
    )
    with pytest.raises(GateError, match="--from cannot be used while qualifying a release"):
        command.execute()

    assert command._runner.commands == []

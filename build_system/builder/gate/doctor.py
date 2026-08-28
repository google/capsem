"""Is this checkout's gate installed and coherent?

The gate is one Python package with one console script. That is easy to say and
easy to have wrong: `uv sync` can succeed while `capsem-gate` resolves to a
stale wheel from a previous checkout, `config/gate.toml` can name a rail the
storage policy no longer declares, and a recipe can dispatch to a subcommand
that was renamed. Each of those fails deep inside a run, and reads as a product
defect rather than an installation one.

These checks are cheap, run before anything expensive, and answer one question:
would the gate work if we started now? They are wired into `just doctor` and
into the bootstrap, so an operator meets them at setup rather than forty
minutes into a release.
"""

from __future__ import annotations

import shutil
import tomllib
from dataclasses import dataclass

from . import config as gate_config
from .actions import Call
from .command import GateCommand
from .errors import GateError
from .execution import Kind, Speed, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .proc import Runner


@dataclass(frozen=True)
class Finding:
    check: str
    detail: str


def _installed_entry_points(root, runner: Runner) -> list[Finding]:
    """The console scripts pyproject declares must actually be runnable."""
    findings = []
    declared = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))["project"][
        "scripts"
    ]

    for name in sorted(declared):
        if shutil.which(name) is None and not _runs_through_uv(runner, name):
            findings.append(
                Finding(
                    f"entry point {name}",
                    f"declared in pyproject but not runnable; run `uv sync` in {root}",
                )
            )
    return findings


def _runs_through_uv(runner: Runner, name: str) -> bool:
    """Through the runner, so the probe is recorded like any other command.

    This called `subprocess.run` directly, which is the one thing the harness
    exists to own -- a doctor that reaches past it can report on a machine the
    run log never saw it touch.
    """
    return runner.succeeds(["uv", "run", name, "--help"])


def _storage_rails(config: gate_config.GateConfig) -> list[Finding]:
    """Every storage phase must name a rail the policy declares.

    A phase pointing at a rail that does not exist releases nothing, and the
    next build fails on ENOSPC somewhere unrelated.
    """
    policy_path = config.path(config.storage.policy_file)
    try:
        rails = set(tomllib.loads(policy_path.read_text(encoding="utf-8"))["rails"])
    except (OSError, KeyError) as error:
        return [Finding("storage policy", f"cannot read rails from {policy_path}: {error}")]

    return [
        Finding(
            f"storage phase {name}",
            f"names rail {phase.rail!r}, which {policy_path.name} does not declare",
        )
        for name, phase in sorted(config.storage.phases.items())
        if phase.rail not in rails
    ]


def _dispatched_subcommands(config: gate_config.GateConfig, runner: Runner) -> list[Finding]:
    """Every `capsem-gate` subcommand the justfile calls must exist.

    A renamed subcommand leaves a recipe dispatching into an argparse error,
    which surfaces as a recipe failure with no obvious cause.
    """
    from .cli import build_parser

    justfile = config.path("justfile").read_text(encoding="utf-8")
    known: set[str] = set()
    for action in build_parser()._actions:
        choices = getattr(action, "choices", None)
        if choices:
            known.update(str(choice) for choice in choices)

    findings = []
    for line in justfile.splitlines():
        marker = "capsem-gate "
        if marker not in line:
            continue
        # A comment calls nothing. Naming a subcommand in prose -- "`capsem-gate
        # linux-rust` names this recipe when the image is missing" -- was read
        # as a dispatch of ``linux-rust` ``, trailing backtick included, and
        # reported as an unknown subcommand. The check is about what the
        # justfile *runs*.
        if line.lstrip().startswith("#"):
            continue
        called = line.split(marker, 1)[1].split()
        if called and called[0] not in known:
            findings.append(
                Finding(
                    f"recipe dispatch {called[0]!r}",
                    f"the justfile calls `capsem-gate {called[0]}`, which is not a "
                    f"subcommand; known: {', '.join(sorted(known))}",
                )
            )
    return findings


def check(runner: Runner) -> list[Finding]:
    """Everything that must hold before the gate can run at all."""
    config = gate_config.for_root(runner.root)
    return [
        *_installed_entry_points(config.root, runner),
        *_storage_rails(config),
        *_dispatched_subcommands(config, runner),
    ]


class DoctorCommand(
    GateCommand,
    name="doctor",
    help="check that this checkout's gate is installed and coherent",
):
    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "check",
                Call(
                    "would the gate work if we started now",
                    report,
                    justification=CallJustification(
                        kind=OpaqueKind.PURE_INSPECTION,
                        reason="reports every wiring problem it can find and changes nothing at all",
                        effects=machine_effects(Effect.PROCESS),
                    ),
                ),
                kind=Kind.STATIC_TEST,
                speed=Speed.FAST,
            )
        )
        return plan


def report(context) -> None:
    """Public: `imagebuild` runs the same check before a build."""
    findings = check(context.runner)
    if not findings:
        context.runner.note("gate: configuration valid, entry points installed, dispatch intact")
        return
    raise GateError(
        "the gate is not ready:\n"
        + "\n".join(f"  {finding.check}: {finding.detail}" for finding in findings)
    )

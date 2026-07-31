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

import argparse
import shutil
import subprocess
import tomllib
from dataclasses import dataclass

from . import config as gate_config
from .errors import GateError
from .proc import Runner


@dataclass(frozen=True)
class Finding:
    check: str
    detail: str


def _installed_entry_points(root) -> list[Finding]:
    """The console scripts pyproject declares must actually be runnable."""
    findings = []
    declared = tomllib.loads(
        (root / "pyproject.toml").read_text(encoding="utf-8")
    )["project"]["scripts"]

    for name in sorted(declared):
        if shutil.which(name) is None and not _runs_through_uv(root, name):
            findings.append(
                Finding(
                    f"entry point {name}",
                    f"declared in pyproject but not runnable; run `uv sync` in {root}",
                )
            )
    return findings


def _runs_through_uv(root, name: str) -> bool:
    return (
        subprocess.run(
            ["uv", "run", name, "--help"],
            cwd=root,
            capture_output=True,
            text=True,
        ).returncode
        == 0
    )


def _storage_rails(config: gate_config.GateConfig) -> list[Finding]:
    """Every storage phase must name a rail the policy declares.

    A phase pointing at a rail that does not exist releases nothing, and the
    next build fails on ENOSPC somewhere unrelated.
    """
    policy_path = config.path(config.doctor.storage_policy)
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
        *_installed_entry_points(config.root),
        *_storage_rails(config),
        *_dispatched_subcommands(config, runner),
    ]


def register(subparsers: argparse._SubParsersAction) -> None:
    parser = subparsers.add_parser(
        "doctor", help="check that this checkout's gate is installed and coherent"
    )
    parser.set_defaults(handler=_command)


def _command(args: argparse.Namespace, runner: Runner) -> int:
    findings = check(runner)
    if not findings:
        runner.note("gate: configuration valid, entry points installed, dispatch intact")
        return 0
    raise GateError(
        "the gate is not ready:\n"
        + "\n".join(f"  {finding.check}: {finding.detail}" for finding in findings)
    )

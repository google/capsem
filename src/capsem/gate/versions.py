"""The workspace version, read from its one authority and fanned out.

Four recipes each read the version with `grep '^version' Cargo.toml | head -1 |
sed ...`, which is a guess about file layout rather than a parse: it matches the
first line starting with `version` anywhere in the file, so a `[dependencies]`
table gaining such a line silently changes what the release calls itself.

The version is a human decision recorded in `Cargo.toml`. Only a person knows
whether a change is a fix, a feature, or a break, which is the entire point of
semver and the reason `min_capsem_version` can mean anything. Nothing here
invents a version, and in particular nothing here derives one from the clock: a
previous scheme appended `$(date +%s)`, which ordered releases but described
none of them, and left every version above every compatibility floor by
accident. `tests/test_retired_version_formats.py` scans this package for that
shape.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

from . import config as gate_config
from .actions import Call
from .command import GateCommand
from .errors import GateError
from .execution import step
from .fileactions import write_text
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .proc import Runner

# Semver's numeric identifiers carry no leading zeros, which is not pedantry
# here: it is what separates `2026.0730.16` -- the retired date-derived asset
# version -- from a real `MAJOR.MINOR.PATCH`. A plain `\d+\.\d+\.\d+` accepts
# the date, and accepting the date is how a version ended up above every
# compatibility floor it was supposed to be compared against.
SEMVER = re.compile(r"^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$")

# How each declared file spells its copy of the version. `[[versions.stamped]]`
# says which files and which key; this says what that looks like in each
# format. `Cargo.lock` and `uv.lock` are absent from both on purpose: their
# copies are refreshed by the tools that own them, not by substitution.
SEMVER_PATTERN = r"\d+\.\d+\.\d+"
FORMATS = {
    "json_key": (
        lambda key: re.compile(rf'"{key}": "{SEMVER_PATTERN}"'),
        lambda key: f'"{key}": "{{version}}"',
    ),
    "toml_key": (
        lambda key: re.compile(rf"^{key} = \"{SEMVER_PATTERN}\"$", re.M),
        lambda key: f'{key} = "{{version}}"',
    ),
}


def require_semver(version: str, *, source: str) -> str:
    """The version, if it is strict `MAJOR.MINOR.PATCH`."""
    if not SEMVER.match(version):
        raise GateError(f"{source} version is not semver MAJOR.MINOR.PATCH: {version}")
    return version


def workspace_version(root: Path) -> str:
    """The version every Capsem artifact in this checkout is stamped with."""
    settings = gate_config.for_root(root).versions
    cargo = Path(root) / settings.cargo_manifest
    try:
        declared = tomllib.loads(cargo.read_text(encoding="utf-8"))["workspace"]["package"][
            "version"
        ]
    except (KeyError, tomllib.TOMLDecodeError) as exc:
        raise GateError(f"{cargo} declares no [workspace.package] version: {exc}") from None
    return require_semver(declared, source=settings.cargo_manifest)


def _substitute(path: Path, pattern: re.Pattern[str], template: str, version: str) -> None:
    """Rewrite the one place a file spells the version.

    `sed` was happy to match nothing, so a renamed key would have left a stale
    version behind and reported success. Requiring exactly one match turns that
    into a failure at the point the assumption broke.
    """
    text = path.read_text(encoding="utf-8")
    replaced, count = pattern.subn(template.format(version=version), text)
    if count != 1:
        raise GateError(f"{path} should spell the version exactly once, matched {count} times")
    write_text(path, replaced)


def stamp(root: Path, runner: Runner) -> str:
    """Propagate `Cargo.toml`'s version across the release cohort.

    Refusing an already-tagged version is what keeps the bump deliberate: the
    release stops until someone chooses the next MAJOR.MINOR.PATCH.
    """
    root = Path(root)
    settings = gate_config.for_root(root).versions
    version = workspace_version(root)

    tag = f"{settings.tag_prefix}{version}"
    if runner.succeeds(["git", "rev-parse", "-q", "--verify", tag]):
        raise GateError(
            f"{tag} is already tagged. Bump the version in "
            f"{settings.cargo_manifest} to the next semver MAJOR.MINOR.PATCH "
            "for this change, then re-run."
        )

    runner.note(f"Stamping release cohort at {version}")
    for stamped in settings.stamped:
        pattern, template = FORMATS[stamped.kind]
        _substitute(root / stamped.path, pattern(stamped.key), template(stamped.key), version)

    # Cargo refreshes workspace package versions in place while preserving the
    # already locked dependency graph.
    runner.run(["cargo", "update", "--workspace", "--offline"])
    # Keep the editable project metadata in the frozen lockfile on the release
    # version before release-binaries creates its commit and tag.
    runner.run(["uv", "lock", "--offline"])
    return version


class StampCommand(
    GateCommand,
    name="stamp-version",
    help="propagate Cargo.toml's version across the release cohort",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "stamp",
                Call(
                    "stamp the workspace version into every file that carries it",
                    _stamp,
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="every file carrying the version is rewritten from the workspace value it reads",
                        effects=machine_effects(Effect.FILESYSTEM),
                    ),
                ),
            )
        )
        return plan


def _stamp(context) -> None:
    stamp(context.root, context.runner)


class VersionCommand(GateCommand, name="version", help="print the workspace version"):
    records = False
    """Only reads runs; creating one would answer with the question."""

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "read",
                Call(
                    "read the version from its one authority",
                    lambda ctx: print(workspace_version(ctx.root)),
                    justification=CallJustification(
                        kind=OpaqueKind.PURE_INSPECTION,
                        reason="prints the workspace version from its one authority and writes nothing",
                        effects=machine_effects(),
                    ),
                ),
            )
        )
        return plan

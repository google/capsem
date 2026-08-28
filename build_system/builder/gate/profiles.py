"""Which profiles a functional proof runs against, and in what order.

The selection is an intersection: the profiles materialized into this
checkout, and the profiles the manifest under test declares active. They have
to agree -- a materialized catalog that differs from the manifest means the
gate would prove a pairing nobody is shipping -- so a mismatch is an error
rather than a smaller set.

The base profile goes first and takes the broad suite; the rest are the
compatibility axis and run the VM-owned suites again behind it. That ordering
used to be a lambda comparing against the string `"code"` inside a script,
which is a product decision spelled in a sort key.
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

from .config import GateConfig
from .errors import GateError


def materialized(profiles_dir: Path) -> list[str]:
    """The profiles present in this checkout, each agreeing with its own id."""
    if not profiles_dir.is_dir():
        raise GateError(f"materialized profile directory is missing: {profiles_dir}")

    found: list[str] = []
    for profile in sorted(profiles_dir.glob("*/profile.toml")):
        identity = profile.parent.name
        try:
            document = tomllib.loads(profile.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise GateError(f"invalid materialized profile {profile}: {error}") from None
        if document.get("id") != identity:
            raise GateError(f"materialized profile {profile} declares an id that is not {identity}")
        found.append(identity)

    if not found:
        raise GateError(f"no materialized profiles found under {profiles_dir}")
    return found


def declared(manifest: Path) -> list[str] | None:
    """The active profiles the manifest names, or None if it names none.

    A manifest with no `profiles` key predates the profile axis; the caller
    falls back to whatever is materialized rather than failing, because that
    is what such a manifest means.
    """
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GateError(f"invalid release test manifest {manifest}: {error}") from None
    if not isinstance(document, dict):
        raise GateError(f"{manifest} must be a JSON object")

    profiles = document.get("profiles")
    if profiles is None:
        return None
    if not isinstance(profiles, dict) or not profiles:
        raise GateError(f"{manifest} profiles must be a non-empty object")

    active = sorted(
        identity
        for identity, profile in profiles.items()
        if isinstance(profile, dict) and profile.get("status") != "revoked"
    )
    if not active:
        raise GateError(f"{manifest} has no active profiles")
    return active


def selected(config: GateConfig) -> list[str]:
    """The profile axis for a functional proof, base profile first.

    From `config/profiles/`, which is checked in -- **not** from what happens
    to be materialized under `target/`. This is read while the plan is being
    *built*, and plan construction cannot depend on build output: a step's
    output does not exist yet, so the same commit produced one plan on a warm
    tree and a different one on a fresh clone. `just release-profile` passed a
    57-minute gate locally on leftovers, then failed in CI with 94 tests all
    reporting `no materialized profiles found`.

    Agreement between this axis, the materialized catalog and the manifest
    under test is still required -- a materialized catalog that differs from
    the manifest means the gate would prove a pairing nobody is shipping. That
    is a run-time question, so `agree()` answers it from a step.
    """
    from . import imagebuild

    base = config.suites.pytest.base_profile
    return sorted(imagebuild.profiles(config), key=lambda identity: (identity != base, identity))


def agree(
    config: GateConfig,
    *,
    profiles_dir: Path | None = None,
    manifest: Path | None = None,
) -> None:
    """Check the materialized catalog matches the source axis and the manifest.

    The check `selected()` used to make inline, moved to where it can run: after
    the step that materializes, rather than before any step has run at all.
    """
    settings = config.suites.pytest
    source = sorted(selected(config))
    selected_profiles = profiles_dir or config.path(settings.materialized_profiles)
    selected_manifest = manifest or config.path(settings.test_manifest)
    present = sorted(materialized(selected_profiles))
    if source != present:
        raise GateError(
            "the materialized profile catalog does not match the checked-in "
            f"profiles: config/profiles={source}, materialized={present}"
        )

    wanted = declared(selected_manifest)
    if wanted is not None and sorted(wanted) != present:
        raise GateError(
            "the materialized profile catalog does not match the manifest under "
            f"test: manifest={sorted(wanted)}, materialized={present}"
        )

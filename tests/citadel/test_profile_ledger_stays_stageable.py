"""Citadel guard: a published profile ledger stays parseable by field binaries.

A profile's `profile.toml` ships in its profile release and is staged by the
binary already installed, which parses it with `deny_unknown_fields` before it
installs the new binary. A release added `default_for`; 0.6.3 refused it,
and every 0.6.3 automatic update failed in the hosted release lane
after hours of qualification. The local gate never staged a real older
release, so nothing caught it sooner.

This holds every key path to the inventory the oldest updating binary knows.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).with_name("profile_ledger_keys.toml")
PROFILES = PROJECT_ROOT / "config" / "profiles"


def _paths(table: dict, prefix: str = "") -> set[str]:
    found: set[str] = set()
    for key, value in table.items():
        path = f"{prefix}{key}"
        found |= _paths(value, f"{path}.") if isinstance(value, dict) else {path}
    return found


def test_every_profile_key_is_known_to_the_oldest_stager() -> None:
    inventory = tomllib.loads(INVENTORY.read_text())
    known = set(inventory["keys"])
    unknown = {
        f"{ledger.parent.name}: {path}"
        for ledger in sorted(PROFILES.glob("*/profile.toml"))
        for path in _paths(tomllib.loads(ledger.read_text()))
        if path not in known
    }
    assert not unknown, (
        f"these profile.toml keys are unknown to Capsem {inventory['oldest_stager']}, "
        "whose updater parses staged profiles strictly before installing a new "
        "binary, so every automatic update from it would fail. Keep the value "
        "out of profile.toml (config/profile-catalog.toml holds catalog-level "
        f"settings) or extend {INVENTORY.name} once no updating binary refuses it: "
        + ", ".join(sorted(unknown))
    )


def test_the_guard_catches_the_field_that_broke_the_update() -> None:
    code = tomllib.loads((PROFILES / "code" / "profile.toml").read_text())
    code["default_for"] = ["vm", "container"]
    known = set(tomllib.loads(INVENTORY.read_text())["keys"])
    assert "default_for" in _paths(code) - known

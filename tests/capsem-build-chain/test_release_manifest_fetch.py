"""Fail-early contracts for binary release manifest authority."""

import importlib.util
import json
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "fetch-channel-source-manifest.py"
SPEC = importlib.util.spec_from_file_location("release_manifest_fetch", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SOURCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SOURCE)


def test_binary_source_manifest_requires_staged_profile_membership() -> None:
    empty = json.dumps(
        {"channel": "nightly", "profiles": {}, "packages": []}
    ).encode()
    staged = json.dumps(
        {"channel": "nightly", "profiles": {"code": {}}, "packages": []}
    ).encode()

    with pytest.raises(ValueError, match="no staged profiles"):
        SOURCE.validate_binary_source_manifest(empty, "nightly")
    assert SOURCE.validate_binary_source_manifest(staged, "nightly")[
        "profiles"
    ] == {"code": {}}


def test_binary_release_fetches_fresh_source_without_bootstrapping_profiles() -> None:
    justfile = (ROOT / "justfile").read_text(encoding="utf-8")
    recipe = justfile.split("\nrelease-binaries channel:", 1)[1].split(
        "\nrelease-profile channel profile:", 1
    )[0]

    assert "scripts/fetch-channel-source-manifest.py" in recipe
    assert "--require-profile-membership" in recipe
    assert "--bootstrap-missing-first-party" not in recipe

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
    empty = json.dumps({"channel": "nightly", "profiles": {}, "packages": []}).encode()
    staged = json.dumps({"channel": "nightly", "profiles": {"code": {}}, "packages": []}).encode()

    with pytest.raises(ValueError, match="no staged profiles"):
        SOURCE.validate_binary_source_manifest(empty, "nightly")
    assert SOURCE.validate_binary_source_manifest(staged, "nightly")["profiles"] == {"code": {}}


def test_binary_release_fetches_fresh_source_without_bootstrapping_profiles() -> None:
    """Read out of the release plan, which is where the ordering now lives.

    The claim is the same one: the binary lane fetches the mutable manifest
    fresh, requires the channel to already have profile membership, and must
    not bootstrap it. Asserted against the plan rather than the recipe, so it
    also covers *where* in the sequence the fetch sits.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand
    from capsem.gate.sourcecommit import SourceCommit

    plan = GateCommand.registry["release-binaries"](
        RecordingRunner(ROOT),
        argparse.Namespace(
            dry_run=False,
            graph=False,
            timing=False,
            channel="nightly",
            source_commit=SourceCommit("0" * 40),
        ),
    )._describe()
    described = plan.describe()

    assert "scripts/fetch-channel-source-manifest.py" in described
    assert "--require-profile-membership" in described
    assert "--bootstrap-missing-first-party" not in described

    order = list(plan.labels)
    assert order.index("channel-source") < order.index("release"), (
        "the source manifest must be resolved before anything publishes"
    )

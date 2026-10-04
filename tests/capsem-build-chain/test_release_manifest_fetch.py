"""Fail-early contracts for binary release manifest authority."""

import json
from pathlib import Path

import pytest
from capsem_builder.release.tools import fetch_channel_source_manifest as SOURCE

ROOT = Path(__file__).resolve().parents[2]


def test_binary_source_manifest_requires_a_staged_runtime() -> None:
    empty = json.dumps({"channel": "nightly", "runtime": None, "packages": []}).encode()
    runtime = {"revision": "9.9.0-0123456789ab", "status": "current", "architectures": []}
    staged = json.dumps({"channel": "nightly", "runtime": runtime, "packages": []}).encode()

    with pytest.raises(ValueError, match="no staged runtime"):
        SOURCE.validate_binary_source_manifest(empty, "nightly")
    assert SOURCE.validate_binary_source_manifest(staged, "nightly")["runtime"] == runtime


def test_binary_release_fetches_fresh_source_without_bootstrapping_a_runtime() -> None:
    """Read out of the release plan, which is where the ordering now lives.

    The binary lane fetches the mutable manifest fresh, requires the channel
    to already carry a staged runtime, and must not bootstrap one. Asserted
    against the plan rather than the recipe, so it also covers *where* in the
    sequence the fetch sits.
    """
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.sourcecommit import SourceCommit
    from helpers.gate import RecordingRunner

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

    assert "build_system/scripts/release/fetch-channel-source-manifest.py" in described
    assert "--require-runtime" in described
    assert "--bootstrap-missing-first-party" not in described

    order = list(plan.labels)
    assert order.index("channel-source") < order.index("release"), (
        "the source manifest must be resolved before anything publishes"
    )

"""The immutable source handoff is an edge, not mutable checkout state."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import resume
from helpers.gate import built_command

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = "0" * 40


def _plan(name: str, **args):
    return built_command(PROJECT_ROOT, name, tuple(args.items()))._describe()


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("release-binaries", {"channel": "stable"}),
        ("release-profile", {"channel": "stable", "profile": "code"}),
    ],
)
def test_release_plan_freezes_source_then_dispatches_hosted_qualification(
    name: str, args: dict
) -> None:
    plan = _plan(name, **args)
    labels = set(plan.labels)

    assert {"source.remote-main", "source.publish-ref", "release"} <= labels
    assert plan.after_of("source.publish-ref")
    assert "qualification.accept" not in labels
    assert "source.remote-main" in resume.ancestors(plan, "source.publish-ref")
    assert "source.publish-ref" in resume.ancestors(plan, "release")

    rendered = plan.describe()
    assert f"publish-release-source.py {SOURCE}" in rendered
    assert "--check" in rendered
    for forbidden in ("publish-tested-main.py", "record-head", "confirm-head", "tested-head"):
        assert forbidden not in rendered


def test_binary_release_plan_passes_the_same_source_to_precheck_and_dispatch() -> None:
    rendered = _plan("release-binaries", channel="nightly").describe()

    assert f"release-binaries.py --precheck nightly {SOURCE}" in rendered
    assert f"release-binaries.py nightly {SOURCE}" in rendered


def test_profile_release_plan_passes_the_same_source_to_admin_dispatch() -> None:
    rendered = _plan("release-profile", channel="nightly", profile="code").describe()

    assert f"--source-commit {SOURCE}" in rendered

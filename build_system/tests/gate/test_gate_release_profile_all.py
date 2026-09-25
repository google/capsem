"""`release-profile <channel> all` releases every profile behind one proof.

Releasing each profile of a channel meant running the command once per
profile, and each run repeated the same clean-tree refusal, live advisory
audits and source-ref publication for the same commit before its one dispatch.
With `all`, those run once and the `release` step dispatches every profile in
turn. The hosted lanes serialize per channel and `capsem-admin release` waits
on each run, so the dispatches still land one at a time, in a fixed order, and
the first failure stops the rest.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import imagebuild
from capsem_builder.gate.errors import GateError
from helpers.gate import built_command

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
COMMIT = "f" * 40


def _plan(profile: str):
    return built_command(
        PROJECT_ROOT,
        "release-profile",
        (("channel", "stable"), ("profile", profile), ("source_commit", COMMIT)),
        None,
    )._describe()


def _dispatched(plan) -> list[str]:
    """The profiles the `release` step dispatches, in order."""
    words = [word for line in plan.steps[plan.labels.index("release")].render() for word in line.split()]
    return [words[at + 1] for at, word in enumerate(words) if word == "--profile"]


def test_all_dispatches_every_profile_once_in_order() -> None:
    assert CONFIG.release.all_profiles == "all"
    profiles = imagebuild.profiles(CONFIG)
    assert len(profiles) > 1, "the test needs more than one profile to mean anything"

    assert _dispatched(_plan(CONFIG.release.all_profiles)) == profiles


def test_all_shares_one_proof_and_one_source_publication() -> None:
    single = _plan(imagebuild.profiles(CONFIG)[0])
    every = _plan(CONFIG.release.all_profiles)

    assert every.labels == single.labels
    for label in ("audit.dependencies", "audit.cargo", "source.publish-ref", "release"):
        assert every.labels.count(label) == 1


def test_one_profile_still_dispatches_only_that_profile() -> None:
    for profile in imagebuild.profiles(CONFIG):
        assert _dispatched(_plan(profile)) == [profile]


def test_unknown_profile_is_still_refused_before_any_work() -> None:
    with pytest.raises(GateError, match="unknown profile"):
        _plan("no-such-profile")


def test_no_profile_may_be_named_like_the_all_keyword() -> None:
    """A profile directory called `all` would be silently unreachable."""
    assert CONFIG.release.all_profiles not in imagebuild.profiles(CONFIG)

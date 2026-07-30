"""First-channel test projection preserves manifest authority and lane ownership."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "project-first-channel-before.py"
SPEC = importlib.util.spec_from_file_location("first_channel_before", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
PROJECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROJECTOR)


def _source() -> dict[str, object]:
    return {
        "version": "1.0.143",
        "channel": "nightly",
        "status": "current",
        "packages": [{"name": "Capsem_1.5_amd64.deb", "status": "current"}],
        "profiles": {
            "code": {
                "revision": "2026.07",
                "architectures": [],
            }
        },
    }


def _bootstrapped_source() -> dict[str, object]:
    """What `bootstrap_first_party_channel_source` actually emits.

    A channel being bootstrapped is absent, so it has no profiles yet. This is
    the only shape the workflow can hand the projector, and the projector used
    to reject it -- the hand-written fixture below carried profiles no cold
    start could produce, so the tests agreed with the code and both were wrong.
    """
    source = _source()
    source["profiles"] = {}
    return source


def test_projection_accepts_the_profileless_source_a_bootstrap_emits() -> None:
    source = _bootstrapped_source()

    before = PROJECTOR.project_first_channel_before(
        source,
        channel="nightly",
        bootstrap=True,
    )

    assert before == source
    assert before["packages"] == source["packages"]
    assert before["packages"] is not source["packages"]


def test_projection_removes_only_profiles_from_the_serialized_candidate() -> None:
    source = _source()

    before = PROJECTOR.project_first_channel_before(
        source,
        channel="nightly",
        bootstrap=True,
    )

    assert before == {**source, "profiles": {}}
    assert source["profiles"]
    assert before["packages"] == source["packages"]
    assert before["packages"] is not source["packages"]


@pytest.mark.parametrize(
    ("channel", "bootstrap", "mutation", "message"),
    [
        ("nightly", False, None, "absent public channel"),
        ("stable", True, None, "declares channel"),
        ("nightly", True, ("profiles", []), "profiles must be an object"),
        ("nightly", True, ("packages", []), "package cohort"),
    ],
)
def test_projection_fails_closed_outside_first_channel_activation(
    channel: str,
    bootstrap: bool,
    mutation: tuple[str, object] | None,
    message: str,
) -> None:
    source = _source()
    if mutation is not None:
        source[mutation[0]] = mutation[1]

    with pytest.raises(ValueError, match=message):
        PROJECTOR.project_first_channel_before(
            source,
            channel=channel,
            bootstrap=bootstrap,
        )

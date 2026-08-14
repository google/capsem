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
        retired=False,
    )

    assert before == {**source, "packages": []}


def test_projection_empties_both_families_because_the_channel_did_not_exist() -> None:
    """An absent channel has no before-state at all -- no profiles, no packages.

    The packages are inherited from the donor channel and validated for shape,
    never for existence. Once a donor is retired its URLs are dead, and a
    before-state claiming packages nobody can fetch sends the pairing gate to a
    404. The binary release that follows publishes this channel's own packages.
    """
    source = _source()

    before = PROJECTOR.project_first_channel_before(
        source,
        channel="nightly",
        bootstrap=True,
        retired=False,
    )

    assert before == {**source, "profiles": {}, "packages": []}
    assert source["profiles"], "the caller's manifest must not be mutated"
    assert source["packages"], "the caller's manifest must not be mutated"


@pytest.mark.parametrize(
    ("channel", "bootstrap", "mutation", "message"),
    [
        ("nightly", False, None, "bootstrap authority"),
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
            retired=False,
        )


def test_retired_channel_projects_an_empty_same_channel_source() -> None:
    source = {
        "version": "1.0.143",
        "channel": "stable",
        "status": "current",
        "packages": [],
        "profiles": {"code": {"revision": "99.99.99"}},
    }

    projected = PROJECTOR.project_first_channel_before(
        source,
        channel="stable",
        bootstrap=True,
        retired=True,
    )

    assert projected["packages"] == []
    assert projected["profiles"] == {}


def test_an_empty_donor_is_rejected_without_exact_retirement() -> None:
    source = {
        "version": "1.0.143",
        "channel": "nightly",
        "status": "current",
        "packages": [],
        "profiles": {},
    }

    with pytest.raises(ValueError, match="official package cohort"):
        PROJECTOR.project_first_channel_before(
            source,
            channel="nightly",
            bootstrap=True,
            retired=False,
        )

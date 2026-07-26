"""Runtime preflight selects only manifest-authorized public graphs."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "select-runtime-preflight-manifest.py"
SPEC = importlib.util.spec_from_file_location("runtime_preflight_manifest", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SELECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SELECTOR)


def _catalog(*channels: str) -> dict[str, object]:
    return {
        "version": 1,
        "channels": {
            channel: {
                "manifests": [
                    {
                        "status": "current",
                        "url": f"/assets/{channel}/manifest.json",
                    }
                ]
            }
            for channel in channels
        },
    }


def test_existing_channel_always_selects_its_public_manifest() -> None:
    selection = SELECTOR.select_runtime_preflight_manifest(
        _catalog("stable", "nightly"),
        release_site="https://release.capsem.org",
        channel="nightly",
        bootstrap_missing_first_party=True,
    )

    assert selection == {
        "channel": "nightly",
        "manifest_channel": "nightly",
        "manifest_url": "https://release.capsem.org/assets/nightly/manifest.json",
        "bootstrap": False,
    }


def test_absent_first_party_profile_channel_uses_existing_donor_graph() -> None:
    selection = SELECTOR.select_runtime_preflight_manifest(
        _catalog("stable"),
        release_site="https://release.capsem.org",
        channel="nightly",
        bootstrap_missing_first_party=True,
    )

    assert selection == {
        "channel": "nightly",
        "manifest_channel": "stable",
        "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
        "bootstrap": True,
    }


def test_absent_first_party_binary_channel_uses_the_same_existing_donor_graph() -> None:
    selection = SELECTOR.select_runtime_preflight_manifest(
        _catalog("stable"),
        release_site="https://release.capsem.org",
        channel="nightly",
        bootstrap_missing_first_party=True,
    )

    assert selection["manifest_channel"] == "stable"
    assert selection["manifest_url"] == (
        "https://release.capsem.org/assets/stable/manifest.json"
    )
    assert selection["bootstrap"] is True


@pytest.mark.parametrize(
    ("catalog", "channel", "bootstrap", "message"),
    [
        (_catalog("stable"), "nightly", False, "is absent"),
        (_catalog(), "nightly", True, "donor stable is absent"),
        (_catalog("stable"), "corp", True, "stable or nightly"),
    ],
)
def test_missing_or_non_first_party_channels_fail_closed(
    catalog: dict[str, object],
    channel: str,
    bootstrap: bool,
    message: str,
) -> None:
    with pytest.raises(ValueError, match=message):
        SELECTOR.select_runtime_preflight_manifest(
            catalog,
            release_site="https://release.capsem.org",
            channel=channel,
            bootstrap_missing_first_party=bootstrap,
        )


def test_catalog_cannot_redirect_manifest_off_the_release_site() -> None:
    catalog = _catalog("nightly")
    catalog["channels"]["nightly"]["manifests"][0]["url"] = "https://attacker.invalid/nightly.json"

    with pytest.raises(ValueError, match="same release site"):
        SELECTOR.select_runtime_preflight_manifest(
            catalog,
            release_site="https://release.capsem.org",
            channel="nightly",
            bootstrap_missing_first_party=False,
        )

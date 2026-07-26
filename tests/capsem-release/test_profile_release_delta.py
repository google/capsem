from __future__ import annotations

import importlib.util
from copy import deepcopy
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "check_profile_release_delta",
    ROOT / "scripts" / "check-profile-release-delta.py",
)
assert SPEC is not None and SPEC.loader is not None
DELTA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DELTA)


def _manifest() -> dict[str, object]:
    return {
        "channel": "nightly",
        "packages": [{"name": "capsem.deb"}],
        "profiles": {
            "code": {
                "revision": "r1",
                "architectures": [{"architecture": "x86_64", "images": []}],
            },
            "experimental": {
                "revision": "e1",
                "architectures": [{"architecture": "x86_64", "images": []}],
            },
        },
    }


def test_profile_delta_compares_only_selected_channel_profile() -> None:
    source = _manifest()
    candidate = deepcopy(source)
    candidate["packages"] = [{"name": "forbidden-candidate-package"}]
    candidate["profiles"]["experimental"]["revision"] = "e2"
    candidate["profiles"]["code"]["architectures"][0]["images"] = [
        {"name": "vmlinuz", "url": "https://candidate.test/vmlinuz"}
    ]
    source["profiles"]["code"]["architectures"][0]["images"] = [
        {"name": "vmlinuz", "url": "https://source.test/vmlinuz"}
    ]

    unchanged = DELTA.selected_profile_delta(
        source, candidate, "nightly", "code"
    )

    assert unchanged["changed"] is False
    candidate["profiles"]["code"]["revision"] = "r2"
    changed = DELTA.selected_profile_delta(source, candidate, "nightly", "code")
    assert changed["changed"] is True
    assert changed["reason"] == "profile_changed"


def test_profile_delta_accepts_new_membership_and_rejects_wrong_channel() -> None:
    source = _manifest()
    del source["profiles"]["code"]
    candidate = _manifest()

    result = DELTA.selected_profile_delta(source, candidate, "nightly", "code")

    assert result["changed"] is True
    assert result["reason"] == "new_profile"
    with pytest.raises(ValueError, match="expected 'stable'"):
        DELTA.selected_profile_delta(source, candidate, "stable", "code")


def test_profile_delta_retries_staged_source_when_public_activation_is_behind() -> None:
    source = _manifest()
    candidate = deepcopy(source)
    public = deepcopy(source)
    del public["profiles"]["code"]

    pending = DELTA.selected_profile_delta(
        source,
        candidate,
        "nightly",
        "code",
        public_manifest=public,
    )

    assert pending["source_changed"] is False
    assert pending["activation_needed"] is True
    assert pending["release_needed"] is True
    assert pending["reason"] == "public_activation_pending"

    current = DELTA.selected_profile_delta(
        source,
        candidate,
        "nightly",
        "code",
        public_manifest=deepcopy(source),
    )
    assert current["source_changed"] is False
    assert current["activation_needed"] is False
    assert current["release_needed"] is False
    assert current["reason"] == "already_public"


def test_profile_delta_cannot_activate_a_staged_binary_package() -> None:
    source = _manifest()
    candidate = deepcopy(source)
    public = deepcopy(source)
    public["packages"] = [{"name": "older-public-capsem.deb"}]

    with pytest.raises(ValueError, match="binary release must finish"):
        DELTA.selected_profile_delta(
            source,
            candidate,
            "nightly",
            "code",
            public_manifest=public,
        )

"""Runtime preflight selects only manifest-authorized public graphs."""

from __future__ import annotations

import importlib.util
import io
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


class _FakeResponse:
    def __init__(self, payload: bytes) -> None:
        self._payload = payload

    def __enter__(self) -> _FakeResponse:
        return self

    def __exit__(self, *_: object) -> None:
        return None

    def read(self) -> bytes:
        return self._payload


def test_catalog_read_survives_a_reset_connection(monkeypatch) -> None:
    """A single CDN reset must not kill an otherwise releasable graph: this is
    the first gating step of both release lanes."""
    attempts: list[int] = []

    def _urlopen(_request: object, timeout: int = 0) -> _FakeResponse:
        attempts.append(1)
        if len(attempts) < 3:
            raise ConnectionResetError(104, "Connection reset by peer")
        return _FakeResponse(b'{"version": 1, "channels": {}}')

    monkeypatch.setattr(SELECTOR, "urlopen", _urlopen)
    monkeypatch.setattr(SELECTOR.time, "sleep", lambda _seconds: None)

    assert SELECTOR._read_catalog("https://release.capsem.org") == {
        "version": 1,
        "channels": {},
    }
    assert len(attempts) == 3


def test_catalog_read_fails_closed_on_an_authoritative_client_error(monkeypatch) -> None:
    """A 4xx is the CDN answering definitively; retrying it would only turn a
    clear failure into a slow one."""
    attempts: list[int] = []

    def _urlopen(_request: object, timeout: int = 0) -> _FakeResponse:
        attempts.append(1)
        # A real stream, not None: HTTPError(fp=None) allocates a temporary
        # file whose finalizer trips the repo's warnings-are-errors policy.
        raise SELECTOR.HTTPError(
            "https://release.capsem.org/channels.json",
            404,
            "Not Found",
            {},
            io.BytesIO(b""),
        )

    monkeypatch.setattr(SELECTOR, "urlopen", _urlopen)
    monkeypatch.setattr(SELECTOR.time, "sleep", lambda _seconds: None)

    with pytest.raises(SELECTOR.HTTPError) as captured:
        SELECTOR._read_catalog("https://release.capsem.org")
    # HTTPError holds a response body; letting it be collected unclosed raises
    # ResourceWarning, which this repo promotes to an error.
    captured.value.close()
    assert len(attempts) == 1


def test_catalog_read_gives_up_after_bounded_attempts(monkeypatch) -> None:
    attempts: list[int] = []

    def _urlopen(_request: object, timeout: int = 0) -> _FakeResponse:
        attempts.append(1)
        raise ConnectionResetError(104, "Connection reset by peer")

    monkeypatch.setattr(SELECTOR, "urlopen", _urlopen)
    monkeypatch.setattr(SELECTOR.time, "sleep", lambda _seconds: None)

    with pytest.raises(ConnectionResetError):
        SELECTOR._read_catalog("https://release.capsem.org")
    assert len(attempts) == SELECTOR.CATALOG_READ_ATTEMPTS


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

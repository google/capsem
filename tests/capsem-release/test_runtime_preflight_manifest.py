"""Runtime preflight selects only manifest-authorized public graphs."""

from __future__ import annotations

import hashlib
import io
from email.message import Message
from pathlib import Path
from typing import Any

import pytest

from capsem import runtime_preflight_manifest as SELECTOR


def _retired(
    digest: str,
) -> dict[SELECTOR.retirement.FirstPartyChannel, SELECTOR.retirement.RetiredPublicGraph]:
    channel = SELECTOR.retirement.FirstPartyChannel.STABLE
    return {
        channel: SELECTOR.retirement.RetiredPublicGraph(
            channel=channel,
            sha256=digest,
        )
    }


def _catalog(*channels: str, sha256: str | None = None) -> dict[str, Any]:
    return {
        "version": 1,
        "channels": {
            channel: {
                "manifests": [
                    {
                        "status": "current",
                        "url": f"/assets/{channel}/manifest.json",
                        **({"digest": {"sha256": sha256}} if sha256 is not None else {}),
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
        "retired": False,
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
        "retired": False,
    }


def test_absent_first_party_binary_channel_uses_the_same_existing_donor_graph() -> None:
    selection = SELECTOR.select_runtime_preflight_manifest(
        _catalog("stable"),
        release_site="https://release.capsem.org",
        channel="nightly",
        bootstrap_missing_first_party=True,
    )

    assert selection["manifest_channel"] == "stable"
    assert selection["manifest_url"] == ("https://release.capsem.org/assets/stable/manifest.json")
    assert selection["bootstrap"] is True
    assert selection["retired"] is False


def test_exact_configured_retired_graph_becomes_an_inactive_bootstrap() -> None:
    payload = b'{"channel":"stable","packages":[],"profiles":{}}\n'
    digest = hashlib.sha256(payload).hexdigest()
    catalog = _catalog("stable", sha256=digest)
    catalog["channels"]["stable"]["manifests"][0]["digest"]["blake3"] = "b" * 64

    selection = SELECTOR.select_runtime_preflight_manifest(
        catalog,
        release_site="https://release.capsem.org",
        channel="stable",
        bootstrap_missing_first_party=True,
        retired_public_graphs=_retired(digest),
        read_manifest=lambda _url: payload,
    )

    assert selection == {
        "channel": "stable",
        "manifest_channel": "stable",
        "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
        "bootstrap": True,
        "retired": True,
    }


def test_retired_catalog_digest_must_match_the_fetched_payload() -> None:
    configured = "a" * 64

    with pytest.raises(ValueError, match="retired public graph payload digest"):
        SELECTOR.select_runtime_preflight_manifest(
            _catalog("stable", sha256=configured),
            release_site="https://release.capsem.org",
            channel="stable",
            bootstrap_missing_first_party=True,
            retired_public_graphs=_retired(configured),
            read_manifest=lambda _url: b"substituted graph",
        )


def test_a_different_current_graph_is_not_retired() -> None:
    selection = SELECTOR.select_runtime_preflight_manifest(
        _catalog("stable", sha256="b" * 64),
        release_site="https://release.capsem.org",
        channel="stable",
        bootstrap_missing_first_party=True,
        retired_public_graphs=_retired("a" * 64),
        read_manifest=lambda _url: pytest.fail("normal graph must not be fetched here"),
    )

    assert selection["bootstrap"] is False
    assert selection["retired"] is False


def test_checked_in_retirement_authority_loads_as_typed_data() -> None:
    retired = SELECTOR.retirement.load_retired_public_graphs()

    assert set(retired) == {SELECTOR.retirement.FirstPartyChannel.STABLE}
    assert retired[SELECTOR.retirement.FirstPartyChannel.STABLE].sha256 == (
        "e8ddf88034a3e73beb605811d5efe5e03c04e79d1ba4b656ff6ca837ef54640e"
    )


@pytest.mark.parametrize(
    "rows",
    [
        'channel = "corp"\nsha256 = "' + "a" * 64 + '"\n',
        'channel = "stable"\nsha256 = "' + "A" * 64 + '"\n',
        ('channel = "stable"\nsha256 = "' + "a" * 64 + '"\nextra = "unguarded"\n'),
        (
            'channel = "stable"\nsha256 = "'
            + "a" * 64
            + '"\n[[release.retired_public_graphs]]\n'
            + 'channel = "stable"\nsha256 = "'
            + "b" * 64
            + '"\n'
        ),
    ],
)
def test_retirement_config_loader_rejects_open_malformed_or_duplicate_rows(
    tmp_path: Path,
    rows: str,
) -> None:
    path = tmp_path / "gate.toml"
    path.write_text(
        "[release]\n[[release.retired_public_graphs]]\n" + rows,
        encoding="utf-8",
    )

    with pytest.raises(ValueError):
        SELECTOR.retirement.load_retired_public_graphs(path)


@pytest.mark.parametrize(
    ("catalog", "channel", "bootstrap", "message"),
    [
        (_catalog("stable"), "nightly", False, "is absent"),
        (_catalog(), "nightly", True, "donor stable is absent"),
        (_catalog("stable"), "corp", True, "stable or nightly"),
    ],
)
def test_missing_or_non_first_party_channels_fail_closed(
    catalog: dict[str, Any],
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
            Message(),
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

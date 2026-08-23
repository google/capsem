"""Digest-pinned authority for retiring one known broken public graph."""

from __future__ import annotations

import hashlib
import json
import re
import tomllib
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

from capsem.releasechannel import FirstPartyChannel

ROOT = Path(__file__).resolve().parents[2]

SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")


@dataclass(frozen=True)
class RetiredPublicGraph:
    channel: FirstPartyChannel
    sha256: str

    def __post_init__(self) -> None:
        if SHA256_PATTERN.fullmatch(self.sha256) is None:
            raise ValueError("retired public graph sha256 must be lowercase 64-hex")


def load_retired_public_graphs(
    path: Path = ROOT / "config" / "gate.toml",
) -> dict[FirstPartyChannel, RetiredPublicGraph]:
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    release = document.get("release")
    if not isinstance(release, dict):
        raise ValueError("gate configuration must contain a release table")
    raw_rows = release.get("retired_public_graphs")
    if not isinstance(raw_rows, list):
        raise ValueError("release.retired_public_graphs must be an array of tables")
    retired: dict[FirstPartyChannel, RetiredPublicGraph] = {}
    for raw in raw_rows:
        if not isinstance(raw, dict) or set(raw) != {"channel", "sha256"}:
            raise ValueError("each retired public graph must contain only channel and sha256")
        raw_channel = raw["channel"]
        raw_sha256 = raw["sha256"]
        if not isinstance(raw_channel, str) or not isinstance(raw_sha256, str):
            raise ValueError("retired public graph channel and sha256 must be strings")
        channel = FirstPartyChannel.parse(raw_channel)
        row = RetiredPublicGraph(channel=channel, sha256=raw_sha256)
        if channel in retired:
            raise ValueError(f"duplicate retired public graph for {channel.value}")
        retired[channel] = row
    return retired


def _catalog_digest(catalog: dict[str, Any], channel: FirstPartyChannel) -> str | None:
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        raise ValueError("public manifest catalog must contain a channels object")
    entry = channels.get(channel.value)
    if not isinstance(entry, dict):
        return None
    manifests = entry.get("manifests")
    if not isinstance(manifests, list):
        raise ValueError(f"public channel {channel.value} manifests must be an array")
    current = [row for row in manifests if isinstance(row, dict) and row.get("status") == "current"]
    if len(current) != 1:
        raise ValueError(f"public channel {channel.value} must have exactly one current manifest")
    digest = current[0].get("digest")
    if digest is None:
        return None
    if not isinstance(digest, dict):
        raise ValueError("public current manifest digest must be an object")
    sha256 = digest.get("sha256")
    if not isinstance(sha256, str) or SHA256_PATTERN.fullmatch(sha256) is None:
        raise ValueError("public current manifest sha256 must be lowercase 64-hex")
    return sha256


def is_exact_retired_public_graph(
    *,
    catalog: dict[str, Any],
    channel: FirstPartyChannel,
    manifest_url: str,
    retired: Mapping[FirstPartyChannel, RetiredPublicGraph],
    read_manifest: Callable[[str], bytes],
) -> bool:
    """Return true only when catalog authority and fetched bytes match config."""
    configured = retired_public_graph_for_digest(
        channel=channel,
        sha256=_catalog_digest(catalog, channel),
        retired=retired,
    )
    if configured is None:
        return False
    actual = hashlib.sha256(read_manifest(manifest_url)).hexdigest()
    if actual != configured.sha256:
        raise ValueError("retired public graph payload digest does not match its catalog authority")
    return True


def retired_public_graph_for_digest(
    *,
    channel: FirstPartyChannel,
    sha256: str | None,
    retired: Mapping[FirstPartyChannel, RetiredPublicGraph],
) -> RetiredPublicGraph | None:
    """Return config authority only for its exact catalog-selected digest."""
    configured = retired.get(channel)
    if configured is None or configured.sha256 != sha256:
        return None
    return configured


def release_site_catalog_url(manifest_url: str) -> str:
    parsed = urlparse(manifest_url)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("first-party bootstrap requires an HTTP release-site fallback")
    return f"{parsed.scheme}://{parsed.netloc}/channels.json"


def public_channel_is_absent(payload: bytes, channel: FirstPartyChannel) -> bool:
    catalog = json.loads(payload)
    if not isinstance(catalog, dict) or not isinstance(catalog.get("channels"), dict):
        raise ValueError("public channel catalog must contain a channels object")
    return channel.value not in catalog["channels"]


def other_first_party_channel(channel: FirstPartyChannel) -> FirstPartyChannel:
    if channel is FirstPartyChannel.STABLE:
        return FirstPartyChannel.NIGHTLY
    return FirstPartyChannel.STABLE


def retired_public_fallback(
    *,
    channel: FirstPartyChannel,
    fallback_url: str,
    payload: bytes,
    retired_public_graphs: Mapping[FirstPartyChannel, RetiredPublicGraph],
    read_url: Callable[[str], bytes],
) -> RetiredPublicGraph | None:
    """Classify a public fallback only through config, catalog, and its bytes."""
    document = json.loads(read_url(release_site_catalog_url(fallback_url)))
    if not isinstance(document, dict):
        raise ValueError("public manifest catalog must be a JSON object")
    if not is_exact_retired_public_graph(
        catalog=document,
        channel=channel,
        manifest_url=fallback_url,
        retired=retired_public_graphs,
        read_manifest=lambda _url: payload,
    ):
        return None
    return retired_public_graphs[channel]

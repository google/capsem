"""Pure helpers for exact release transition baselines."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


def _record(channel: str, route: str, manifest: Path, blake3: str) -> dict[str, object]:
    contents = manifest.read_bytes()
    return {
        "label": channel.replace("-", " ").title(),
        "manifests": [
            {
                "version": json.loads(contents).get("version", "1.0.0"),
                "status": "current",
                "url": route,
                "digest": {
                    "sha256": hashlib.sha256(contents).hexdigest(),
                    "blake3": blake3,
                },
            }
        ],
    }


def exact_channel_catalog(
    *,
    baseline_channel: str,
    target_channel: str,
    before_route: str,
    before_manifest: Path,
    before_blake3: str,
    target_route: str,
    target_manifest: Path,
    target_blake3: str,
) -> dict[str, object]:
    """Describe the verified baseline and exact target without a mutable pointer."""
    channels = {
        baseline_channel: _record(
            baseline_channel, before_route, before_manifest, before_blake3
        )
    }
    channels[target_channel] = _record(
        target_channel, target_route, target_manifest, target_blake3
    )
    return {
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": channels,
    }

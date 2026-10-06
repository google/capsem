"""Release-owned helpers for exact release transition baselines."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from .release_glowup import TransitionKind


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
        baseline_channel: _record(baseline_channel, before_route, before_manifest, before_blake3)
    }
    channels[target_channel] = _record(target_channel, target_route, target_manifest, target_blake3)
    return {
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": channels,
    }


def validate_selected_profile_scope(
    *,
    transition: TransitionKind,
    selected_profile: str | None,
    changed_profiles: tuple[str, ...],
) -> None:
    """The selected profile must be part of the delta it claims to release.

    It need not be all of it. Profile releases may run one after another before
    the binary release activates them, so an earlier staged, still-inert
    profile is a legitimate member of the delta beside the selected one, and a
    channel switch stages the whole target graph. What a profile release can
    change is bounded upstream: `capsem-admin release` authors only the
    selected profile into the staged channel manifest.
    """
    if selected_profile is None:
        return
    if selected_profile in changed_profiles:
        return
    if transition is TransitionKind.CHANNEL_SWITCH:
        raise SystemExit("exact pairing selected profile is absent from the cross-channel target")
    raise SystemExit("exact pairing selected profile does not match the classified manifest delta")

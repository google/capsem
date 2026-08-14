"""Invoke the sole channel-source author for donor and retirement bootstrap."""

from __future__ import annotations

import json
import subprocess
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Any

from capsem.gate.sourcecommit import SourceCommit
from capsem.release_retirement import RetiredPublicGraph
from capsem.releasechannel import FirstPartyChannel

ROOT = Path(__file__).resolve().parents[2]


def validate_source_manifest(payload: bytes, channel: str) -> dict[str, Any]:
    manifest = json.loads(payload)
    if not isinstance(manifest, dict):
        raise ValueError("source manifest must be a JSON object")
    if manifest.get("channel") != channel:
        raise ValueError(
            f"source manifest declares channel {manifest.get('channel')!r}, expected {channel!r}"
        )
    if not isinstance(manifest.get("profiles"), dict):
        raise ValueError("source manifest profiles must be an object")
    if not isinstance(manifest.get("packages"), list):
        raise ValueError("source manifest packages must be an array")
    return manifest


def validate_binary_source_manifest(payload: bytes, channel: str) -> dict[str, Any]:
    manifest = validate_source_manifest(payload, channel)
    if not manifest["profiles"]:
        raise ValueError(
            f"source manifest for {channel} has no staged profiles; run release-profile first"
        )
    return manifest


def bootstrap_source_manifest(
    *,
    channel: FirstPartyChannel,
    profile: str,
    source_commit: SourceCommit,
    input_payload: bytes,
    output: Path,
    retired_graph: RetiredPublicGraph | None = None,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> bytes:
    """Ask capsem-admin to author one serialized first-party source graph."""
    output.parent.mkdir(parents=True, exist_ok=True)
    kind = "retired" if retired_graph is not None else "donor"
    with tempfile.NamedTemporaryFile(
        dir=output.parent,
        prefix=f".{channel}-bootstrap-{kind}-",
        suffix=".json",
        delete=False,
    ) as handle:
        handle.write(input_payload)
        input_path = Path(handle.name)
    command = [
        "cargo",
        "run",
        "-p",
        "capsem-admin",
        "--",
        "release",
        "--channel",
        channel.value,
        "--profile",
        profile,
        "--source-commit",
        str(source_commit),
    ]
    if retired_graph is None:
        command.extend(["--bootstrap-from-manifest", str(input_path)])
    else:
        command.extend(
            [
                "--bootstrap-retired-manifest",
                str(input_path),
                "--bootstrap-retired-sha256",
                retired_graph.sha256,
            ]
        )
    command.extend(["--bootstrap-output", str(output), "--json"])
    try:
        runner(command, cwd=ROOT, check=True, text=True)
        payload = output.read_bytes()
        validate_source_manifest(payload, channel.value)
        return payload
    finally:
        input_path.unlink(missing_ok=True)

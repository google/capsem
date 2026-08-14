"""One immutable source context shared by source-derived gate products."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Self

from . import snapshot
from .actions import Action
from .config import GateConfig
from .context import Context
from .errors import GateError
from .filesystem import remove

_DIGEST = re.compile(r"[0-9a-f]{64}\Z")


class SourceDigest(str):
    """One canonical BLAKE3 source-state digest."""

    def __new__(cls, value: str) -> Self:
        if _DIGEST.fullmatch(value) is None:
            raise ValueError("source digest must be 64-character lowercase hexadecimal")
        return str.__new__(cls, value)


@dataclass(frozen=True)
class SourceSnapshot:
    """A frozen source tree and the digest that identifies all of its bytes."""

    root: Path
    digest: SourceDigest


def _snapshot_root(config: GateConfig) -> Path:
    relative = Path(config.candidate.source_snapshot_dir)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        raise GateError("candidate source_snapshot_dir must stay beneath the checkout")
    target = config.path(str(relative))
    if target == config.root:
        raise GateError("candidate source_snapshot_dir cannot be the checkout root")
    return target


def _recorded_digest(config: GateConfig) -> SourceDigest:
    receipt = config.path(config.candidate.source_state_file)
    try:
        payload = json.loads(receipt.read_text(encoding="utf-8"))
        if not isinstance(payload, dict) or not isinstance(payload.get("digest"), str):
            raise ValueError("missing digest")
        return SourceDigest(payload["digest"])
    except (OSError, json.JSONDecodeError, ValueError) as error:
        raise GateError(f"source state receipt {receipt} has no canonical digest") from error


def _require_real_directory(root: Path, config: GateConfig) -> None:
    if root.is_symlink() or not root.is_dir():
        raise GateError(f"frozen source snapshot {root} is missing or is a symlink")
    try:
        relative = root.relative_to(config.root)
    except ValueError as error:
        raise GateError(f"frozen source snapshot escapes the checkout: {root}") from error
    current = config.root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise GateError(f"frozen source snapshot crosses symlink {current}")


def capture(config: GateConfig, *, expected: SourceDigest) -> SourceSnapshot:
    """Replace the config-owned snapshot and bind it to the recorded digest."""
    target = _snapshot_root(config)
    remove(target)
    snapshot.populate_subject(config.root, target, config)
    found = SourceDigest(snapshot.digest(target, config))
    if found != expected:
        remove(target)
        raise GateError(
            "source changed while its frozen build context was captured: "
            f"recorded {expected}, captured {found}"
        )
    return SourceSnapshot(target, found)


def require_recorded(config: GateConfig) -> SourceSnapshot:
    """Load and re-hash the exact snapshot owned by `source.record`."""
    expected = _recorded_digest(config)
    root = _snapshot_root(config)
    _require_real_directory(root, config)
    found = SourceDigest(snapshot.digest(root, config))
    if found != expected:
        raise GateError(f"frozen source snapshot moved: recorded {expected}, found {found}")
    return SourceSnapshot(root, found)


def require_snapshot(config: GateConfig, source: SourceSnapshot) -> None:
    """Revalidate a typed snapshot immediately around source consumption."""
    if not isinstance(source, SourceSnapshot):
        raise TypeError("source must be a SourceSnapshot")
    expected = require_recorded(config)
    if source != expected:
        raise GateError(
            f"selected source snapshot {source.root} at {source.digest} no longer "
            f"matches the recorded snapshot {expected.root} at {expected.digest}"
        )


class CaptureSourceSnapshot(Action, name="capture-source-snapshot"):
    """Freeze the source state immediately after it is recorded."""

    def render(self) -> str:
        return "capture the recorded source as an immutable build context"

    def perform(self, context: Context) -> None:
        if context.observing:
            return
        frozen = capture(context.config, expected=_recorded_digest(context.config))
        context.journal.note(f"frozen source context {frozen.root} at {frozen.digest}")

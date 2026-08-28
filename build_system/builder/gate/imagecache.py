"""Find Docker image products pinned by active or resumable receipts."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Literal

from . import installreceipt, prefix
from .config import GateConfig
from .errors import GateError

RECEIPT_SCHEMA = installreceipt.SCHEMA


def _receipt_roots(config: GateConfig) -> tuple[Path, ...]:
    roots = {config.root.resolve()}
    source = prefix.source_checkout(config)
    if source is not None:
        roots.add(source.resolve())
    parent = prefix.parent_dir(config)
    if parent.is_dir():
        roots.update(
            child.resolve()
            for child in parent.iterdir()
            if child.is_dir() and not child.is_symlink() and child.stat().st_uid == os.getuid()
        )
    return tuple(sorted(roots))


def protected_tags(
    config: GateConfig,
    repository: str,
    *,
    field: Literal["input_key", "helper_input_key"],
) -> tuple[str, ...]:
    """Return tags named by strict receipts in retained source lineages."""
    relative = Path(config.install.builder.source_identity_file)
    found: set[str] = set()
    for root in _receipt_roots(config):
        receipt = root / relative
        try:
            document = installreceipt.read(receipt)
        except GateError:
            continue
        value = getattr(document, field)
        if value.rsplit(":", 1)[0] == repository:
            found.add(value)
    return tuple(sorted(found))

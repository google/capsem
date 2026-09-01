"""Named generated views backed by immutable digest-checked objects."""

from __future__ import annotations

import secrets
from enum import StrEnum
from pathlib import Path
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, StringConstraints

from .objects import ObjectRef, import_file, materialize, verify
from .paths import CachePaths

SCHEMA = "capsem.object-view.v1"


class ReceiptLocation(StrEnum):
    SIDECAR = "sidecar"
    INVENTORY = "inventory"


class ViewReceipt(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.object-view.v1"]
    logical_name: Annotated[str, StringConstraints(pattern=r"^[^/\\]+$")]
    object: ObjectRef


def _bind(
    paths: CachePaths,
    reference: ObjectRef,
    view: Path,
    receipt_location: ReceiptLocation,
) -> ViewReceipt:
    materialize(paths, reference, view)
    receipt = ViewReceipt(schema_id=SCHEMA, logical_name=view.name, object=reference)
    if receipt_location is ReceiptLocation.SIDECAR:
        destination = view.with_name(f"{view.name}.object.json")
    else:
        destination = (
            paths.stage("objects")
            / "receipts"
            / "views"
            / reference.digest
            / f"{view.name}.json"
        )
        destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.{secrets.token_hex(6)}")
    try:
        temporary.write_text(receipt.model_dump_json(indent=2) + "\n", encoding="utf-8")
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    verify(paths, reference)
    return receipt


def canonicalize(paths: CachePaths, view: Path) -> ViewReceipt:
    """Replace a named output with a hardlinked object and bind a receipt."""
    return _bind(paths, import_file(paths, view), view, ReceiptLocation.SIDECAR)


def copy_view(
    paths: CachePaths,
    source: Path,
    destination: Path,
    *,
    receipt_location: ReceiptLocation = ReceiptLocation.SIDECAR,
) -> ViewReceipt:
    """Materialize a named view directly from one imported immutable source."""
    return _bind(paths, import_file(paths, source), destination, receipt_location)

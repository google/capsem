"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .snapshot_origin import SnapshotOrigin


class SnapshotInfo(Model):
    checkpoint: StrictStr
    hash: StrictStr | None = None
    name: StrictStr | None = None
    origin: SnapshotOrigin
    slot: Annotated[StrictInt, Field(ge=0)]
    timestamp: StrictStr

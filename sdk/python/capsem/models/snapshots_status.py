"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model
from .snapshot_info import SnapshotInfo


class SnapshotsStatus(Model):
    auto_count: Annotated[StrictInt, Field(ge=0)]
    manual_available: Annotated[StrictInt, Field(ge=0)]
    manual_count: Annotated[StrictInt, Field(ge=0)]
    snapshots: list[SnapshotInfo]
    total: Annotated[StrictInt, Field(ge=0)]

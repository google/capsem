"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt

from .model_base import Model
from .snapshot_info import SnapshotInfo


class SnapshotsList(Model):
    snapshots: list[SnapshotInfo]
    total: Annotated[StrictInt, Field(ge=0)]

"""Strict models for retention-focused cache inventories."""

from pathlib import Path
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictInt

from .models import StageInventory


class RetentionInventory(BaseModel):
    """Only policy-owned stages that routine pruning may reclaim."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    root: Path
    generated_ns: Annotated[StrictInt, Field(ge=0)]
    filesystem_free_bytes: Annotated[StrictInt, Field(ge=0)]
    stages: tuple[StageInventory, ...]

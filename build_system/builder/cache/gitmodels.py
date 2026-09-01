"""Strict facts read from Git for test-impact admission."""

from __future__ import annotations

from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictBool, StrictInt


class GitImpact(BaseModel):
    """Read-only Git facts used by the pure admission decision."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    baseline: str
    target: str
    ancestor: StrictBool
    commits: Annotated[StrictInt, Field(ge=0)]
    paths: tuple[str, ...]

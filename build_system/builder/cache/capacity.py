"""Finite producer intent, distinct from measured cache inventory and policy."""

from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StrictInt


class RequiredCapacity(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    bytes: Annotated[StrictInt, Field(ge=0, le=2**63 - 1)] = 0
    entries: Annotated[StrictInt, Field(ge=0, le=2**63 - 1)] = 0

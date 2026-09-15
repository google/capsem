"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .exec_source import ExecSource
from .model_base import Model


class ExecHistoryDetails(Model):
    exec_id: Annotated[StrictInt, Field(ge=0)]
    process_name: StrictStr | None = None
    source: ExecSource
    trace_id: StrictStr | None = None

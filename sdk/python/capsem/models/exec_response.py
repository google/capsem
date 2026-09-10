"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictInt, StrictStr

from .model_base import Model


class ExecResponse(Model):
    nonnullable_optional = frozenset(['truncated'])
    exit_code: StrictInt
    stderr: StrictStr
    stdout: StrictStr
    truncated: StrictBool | None = None

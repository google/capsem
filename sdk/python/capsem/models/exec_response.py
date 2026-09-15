"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictInt

from .exec_output import ExecOutput
from .model_base import Model


class ExecResponse(Model):
    nonnullable_optional = frozenset(['truncated'])
    exit_code: StrictInt
    stderr: ExecOutput
    stdout: ExecOutput
    truncated: StrictBool | None = None

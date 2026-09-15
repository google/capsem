"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .exec_output_encoding import ExecOutputEncoding
from .model_base import Model


class ExecOutput(Model):
    data: StrictStr
    encoding: ExecOutputEncoding

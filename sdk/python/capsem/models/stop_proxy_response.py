"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model


class StopProxyResponse(Model):
    session_id: StrictStr
    stopped: StrictBool

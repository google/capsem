"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class TerminalNotification(Model):
    body: StrictStr
    id: StrictStr
    title: StrictStr

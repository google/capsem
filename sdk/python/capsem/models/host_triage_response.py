"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .error_event import ErrorEvent
from .model_base import Model
from .panic_event import PanicEvent
from .slow_op_event import SlowOpEvent


class HostTriageResponse(Model):
    errors: list[ErrorEvent]
    panics: list[PanicEvent]
    slow_ops: list[SlowOpEvent]

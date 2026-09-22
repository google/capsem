"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .model_base import Model


class LogsResponse(Model):
    logs: StrictStr
    process_logs: StrictStr | None = None
    serial_logs: StrictStr | None = None

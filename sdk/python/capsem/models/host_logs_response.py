"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .host_log_source import HostLogSource
from .model_base import Model


class HostLogsResponse(Model):
    source: HostLogSource
    text: StrictStr

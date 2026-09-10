"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .capture_status import CaptureStatus
from .captured_content import CapturedContent
from .model_base import Model


class CapturedPayload(Model):
    content: CapturedContent
    status: CaptureStatus

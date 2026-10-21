"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .model_base import Model
from .stream_kind import StreamKind


class StreamControlStart(Model):
    command: StrictStr | None = None
    kind: StreamKind
    type: Literal["start"]

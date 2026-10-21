"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .stream_status_error import StreamStatusError
from .stream_status_exit import StreamStatusExit
from .stream_status_started import StreamStatusStarted
from .stream_status_terminal import StreamStatusTerminal

StreamStatus: TypeAlias = StreamStatusStarted | StreamStatusTerminal | StreamStatusExit | StreamStatusError

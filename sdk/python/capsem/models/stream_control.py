"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .stream_control_close_stdin import StreamControlCloseStdin
from .stream_control_focus import StreamControlFocus
from .stream_control_resize import StreamControlResize
from .stream_control_start import StreamControlStart

StreamControl: TypeAlias = StreamControlStart | StreamControlResize | StreamControlFocus | StreamControlCloseStdin

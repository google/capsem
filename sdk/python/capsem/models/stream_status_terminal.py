"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from .model_base import Model
from .terminal_state import TerminalState


class StreamStatusTerminal(Model):
    state: TerminalState
    type: Literal["terminal"]

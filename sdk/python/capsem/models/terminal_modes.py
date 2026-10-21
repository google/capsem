"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model


class TerminalModes(Model):
    alternate_screen: StrictBool
    application_cursor: StrictBool
    application_keypad: StrictBool
    bracketed_paste: StrictBool
    mouse_encoding: StrictStr
    mouse_protocol: StrictStr

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictStr

from .model_base import Model


class TerminalModes(Model):
    alternate_screen: StrictBool
    application_cursor: StrictBool
    application_keypad: StrictBool
    bracketed_paste: StrictBool
    mouse_encoding: Annotated[StrictStr, Field(max_length=1024)]
    mouse_protocol: Annotated[StrictStr, Field(max_length=1024)]

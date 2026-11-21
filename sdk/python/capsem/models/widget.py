"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class Widget(StrEnum):
    TOGGLE = 'toggle'
    TEXT_INPUT = 'text_input'
    NUMBER_INPUT = 'number_input'
    PASSWORD_INPUT = 'password_input'
    SELECT = 'select'
    FILE_EDITOR = 'file_editor'
    DOMAIN_CHIPS = 'domain_chips'
    STRING_CHIPS = 'string_chips'
    SLIDER = 'slider'
    KV_EDITOR = 'kv_editor'

"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool

from .model_base import Model


class ProfileAvailabilitySummary(Model):
    mobile: StrictBool
    shell: StrictBool
    web: StrictBool

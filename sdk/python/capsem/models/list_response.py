"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .sandbox_info import SandboxInfo


class ListResponse(Model):
    sandboxes: list[SandboxInfo]

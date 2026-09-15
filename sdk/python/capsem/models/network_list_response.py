"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .network_info import NetworkInfo


class NetworkListResponse(Model):
    networks: list[NetworkInfo]

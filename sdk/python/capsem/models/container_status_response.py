"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictInt, StrictStr

from .container_state import ContainerState
from .model_base import Model


class ContainerStatusResponse(Model):
    digest: StrictStr | None = None
    error: StrictStr | None = None
    exit_code: StrictInt | None = None
    image: StrictStr
    state: ContainerState

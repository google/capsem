"""Exact approved startup timing, read before any creation mutation."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport
from .execution import CREATE_READY_SECS, GATEWAY_REQUEST_BUDGET_SECS


async def creation_timeout(transport: Transport, request: models.ProvisionRequest) -> float:
    seconds = 0
    if request.environment is not None and not request.attach:
        selected = request.environment
        response = await api.get_environment_readiness(
            transport, id=selected.environment_id, revision_id=selected.revision_id)
        if response.environment != selected:
            raise ValueError("environment readiness identity changed")
        seconds = response.startup_timeout_seconds
        if seconds > 2**32 - 1:
            raise ValueError("environment readiness timeout exceeds the approved domain")
    return max(transport.timeout, float(CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS + seconds))

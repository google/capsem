"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from .model_base import Model
from .restart_authentication import RestartAuthentication
from .restart_status import RestartStatus
from .service_manager import ServiceManager


class RestartResponse(Model):
    authentication: RestartAuthentication
    manager: ServiceManager
    status: RestartStatus

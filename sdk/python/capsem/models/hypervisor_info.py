"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .profile_catalog_status import ProfileCatalogStatus
from .resource_summary import ResourceSummary
from .service_availability import ServiceAvailability
from .update_status_response import UpdateStatusResponse
from .vm_summary import VmSummary


class HypervisorInfo(Model):
    gateway_version: StrictStr
    profiles: ProfileCatalogStatus | None = None
    resource_summary: ResourceSummary | None = None
    service: ServiceAvailability
    updates: UpdateStatusResponse | None = None
    vm_count: Annotated[StrictInt, Field(ge=0)]
    vms: list[VmSummary]

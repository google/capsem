"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictBool, StrictInt, StrictStr

from .model_base import Model
from .supply_chain_evidence import SupplyChainEvidence
from .update_track_status import UpdateTrackStatus
from .validation_status import ValidationStatus


class UpdateStatusResponse(Model):
    assets: UpdateTrackStatus
    binary: UpdateTrackStatus
    channel_hash: StrictStr | None = None
    channel_url: StrictStr | None = None
    checked_at: Annotated[StrictInt, Field(ge=0)] | None = None
    images: UpdateTrackStatus
    last_error: StrictStr | None = None
    profiles: UpdateTrackStatus
    stale: StrictBool
    supply_chain: SupplyChainEvidence
    validation_error: StrictStr | None = None
    validation_status: ValidationStatus | None = None

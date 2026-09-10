"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictBool, StrictStr

from .model_base import Model
from .update_compatibility_state import UpdateCompatibilityState
from .update_track_state import UpdateTrackState


class UpdateTrackStatus(Model):
    blocked_reason: StrictStr | None = None
    compatibility: UpdateCompatibilityState
    current: StrictStr | None = None
    latest: StrictStr | None = None
    state: UpdateTrackState
    update_available: StrictBool

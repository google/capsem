"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from pydantic import StrictStr

from .credential_event_type import CredentialEventType
from .credential_outcome import CredentialOutcome
from .material_class import MaterialClass
from .model_base import Model


class CredentialEvent(Model):
    context_json: StrictStr | None = None
    event_id: StrictStr
    event_type: CredentialEventType | None = None
    material_class: MaterialClass
    origin: CredentialEventType | None = None
    provider: StrictStr | None = None
    source: StrictStr
    timestamp: StrictStr
    trace_id: StrictStr | None = None
    verb: CredentialOutcome

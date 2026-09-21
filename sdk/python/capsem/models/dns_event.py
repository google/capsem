"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, StrictInt, StrictStr

from .model_base import Model
from .network_decision import NetworkDecision
from .network_protocol import NetworkProtocol


class DnsEvent(Model):
    credential_ref: StrictStr | None = None
    decision: NetworkDecision
    event_id: StrictStr
    matched_rule: StrictStr | None = None
    policy_rule: StrictStr | None = None
    process_name: StrictStr | None = None
    qclass: Annotated[StrictInt, Field(ge=0)]
    qname: StrictStr
    qtype: Annotated[StrictInt, Field(ge=0)]
    rcode: Annotated[StrictInt, Field(ge=0)]
    source_proto: NetworkProtocol | None = None
    timestamp: StrictStr
    trace_id: StrictStr | None = None
    upstream_resolver_ms: Annotated[StrictInt, Field(ge=0)] | None = None

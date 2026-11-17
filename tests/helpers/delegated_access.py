"""Operator fixtures issue finite access to one real VM owner generation."""

from __future__ import annotations

import json
import secrets
import uuid
from contextlib import contextmanager
from dataclasses import dataclass, field

from .service import vm_session_dir
from .service_gateway import ServiceGateway


@dataclass(frozen=True)
class DelegatedAccess:
    gateway_url: str
    access_id: str
    capability: str = field(repr=False)

    @property
    def bearer(self) -> str:
        return f"{self.access_id}.{self.capability}"


@contextmanager
def delegated_vm_access(service, operator, session_id: str, *, lease_seconds: int = 300):
    """Read host identity as operator, then pass only scoped access onward."""
    gateway = ServiceGateway(service)
    gateway.start()
    issued_access = None
    try:
        path = vm_session_dir(service.tmp_dir, operator, session_id) / "spawn-identity.json"
        identity = json.loads(path.read_text())
        assert identity["schema_version"] == 1
        assert identity["binding"]["id"] == session_id
        generation = identity["binding"]["generation"]
        assert str(uuid.UUID(generation)) == generation
        access = DelegatedAccess(gateway.base_url, str(uuid.uuid4()), secrets.token_hex(32))
        scope = {"operations": ["execute"], "environment_ids": [], "sessions": [
            {"session_id": session_id, "owner_generation": generation},
        ]}
        issued = operator.post("/agent-access", {
            "request_id": access.access_id, "ownership_token": access.capability,
            "scope": scope, "lease_seconds": lease_seconds,
        })
        assert issued["access"]["id"] == access.access_id
        assert issued["access"]["scope"] == scope
        issued_access = access
        yield access
    finally:
        try:
            if issued_access is not None:
                assert operator.delete(f"/agent-access/{issued_access.access_id}") == {"success": True}
        finally:
            gateway.stop()

"""Ironbank credential store lifecycle proof.

This is black-box service proof for the credential store rail: durable
credential material can be loaded into runtime memory through the broker retry
route, hot reads stay memory-only, service status does not expose inventory,
and raw credentials never appear in route JSON.
"""

from __future__ import annotations

import json
from pathlib import Path

import blake3
import pytest

from helpers.constants import CODE_PROFILE_ID
from helpers.service import ServiceInstance


pytestmark = pytest.mark.integration


CREDENTIAL_REF_PREFIX = "credential:blake3:"
CREDENTIAL_REF_DOMAIN = b"capsem.credential.v1"


def _credential_reference(provider: str, raw_credential: str) -> str:
    hasher = blake3.blake3()
    hasher.update(CREDENTIAL_REF_DOMAIN)
    hasher.update(b"\0")
    hasher.update(provider.encode())
    hasher.update(b"\0")
    hasher.update(raw_credential.encode())
    return f"{CREDENTIAL_REF_PREFIX}{hasher.hexdigest()}"


def test_credential_store_retry_and_hot_status_reads_pay_lifecycle_debt_blackbox() -> None:
    service = ServiceInstance()
    raw_credential = "this_is_not_a_real_key_ironbank_lifecycle"
    provider = "google"
    credential_ref = _credential_reference(provider, raw_credential)
    try:
        service.start()
        client = service.client()
        store_path = Path(service.tmp_dir) / "credential-store.json"
        store_path.write_text(
            json.dumps({f"{provider}:{credential_ref}": raw_credential}, indent=2),
            encoding="utf-8",
        )

        service_status = client.get("/status")
        assert service_status["ready"] is True
        assert service_status["components"]["credential_store"] == {
            "ready": True,
            "status": "ready",
            "last_error": None,
        }
        assert raw_credential not in json.dumps(service_status)
        assert credential_ref not in json.dumps(service_status)

        detail_path = f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/info"
        reload_path = f"/profiles/{CODE_PROFILE_ID}/plugins/credential_broker/credentials/reload"

        before = client.get(detail_path)
        assert before["plugin_id"] == "credential_broker"
        assert before["store"]["backend"] == "disk_override"
        assert before["store"]["ready"] is True
        assert before["store"]["status"] == "ready"
        assert before["store"]["cached_count"] == 0
        assert before["store"]["last_hydrated_count"] == 0
        startup_hydrated_at = before["store"]["last_hydrated_unix_ms"]
        assert isinstance(startup_hydrated_at, int)
        assert before["inventory"] == []
        assert raw_credential not in json.dumps(before)

        reloaded = client.post(reload_path, {})
        assert reloaded["store"]["cached_count"] == 1
        assert reloaded["store"]["last_hydrated_count"] == 1
        hydrated_at = reloaded["store"]["last_hydrated_unix_ms"]
        assert isinstance(hydrated_at, int)
        assert hydrated_at >= startup_hydrated_at
        assert reloaded["inventory"] == []
        assert raw_credential not in json.dumps(reloaded)

        for _ in range(5):
            status = client.get("/status")
            assert status["components"]["credential_store"] == {
                "ready": True,
                "status": "ready",
                "last_error": None,
            }
            detail = client.get(detail_path)
            assert detail["store"]["cached_count"] == 1
            assert detail["store"]["last_hydrated_count"] == 1
            assert detail["store"]["last_hydrated_unix_ms"] == hydrated_at
            assert detail["inventory"] == []
            assert raw_credential not in json.dumps(status)
            assert raw_credential not in json.dumps(detail)
    finally:
        service.stop()

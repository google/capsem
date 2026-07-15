"""Credential store route contract.

The credential broker owns credential inventory and retry. Service status may
report readiness, but it must not expose inventory counters or hammer durable
storage on hot reads.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import blake3


PROFILE = "code"
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


def _write_test_store(service_env: Any, *, provider: str, raw_credential: str) -> str:
    credential_ref = _credential_reference(provider, raw_credential)
    store_path = Path(service_env.home_dir) / "credential-store.json"
    store_path.write_text(
        json.dumps({f"{provider}:{credential_ref}": raw_credential}, indent=2),
        encoding="utf-8",
    )
    return credential_ref


def test_status_reports_credential_store_readiness_without_inventory(client: Any) -> None:
    status = client.get("/status")
    credential_store = status["components"]["credential_store"]

    assert credential_store == {
        "ready": True,
        "status": "ready",
        "last_error": None,
    }
    assert "cached_count" not in credential_store
    assert "inventory" not in credential_store


def test_credential_broker_retry_loads_store_once_and_hot_reads_are_memory_only(
    client: Any,
    service_env: Any,
) -> None:
    provider = "openai"
    raw_credential = "this_is_not_a_real_key_route_contract"
    credential_ref = _write_test_store(
        service_env,
        provider=provider,
        raw_credential=raw_credential,
    )

    before = client.get(f"/profiles/{PROFILE}/plugins/credential_broker/credentials/info")
    assert before["store"]["backend"] == "disk_override"
    assert before["store"]["ready"] is True
    assert before["store"]["status"] == "ready"
    assert before["store"]["last_error"] is None
    assert before["store"]["cached_count"] == 0
    assert before["store"]["last_hydrated_count"] == 0
    startup_hydrated_at = before["store"]["last_hydrated_unix_ms"]
    assert isinstance(startup_hydrated_at, int)
    assert raw_credential not in json.dumps(before)
    assert credential_ref not in json.dumps(before)

    for _ in range(3):
        hot_status = client.get("/status")
        assert hot_status["components"]["credential_store"] == {
            "ready": True,
            "status": "ready",
            "last_error": None,
        }
        hot_detail = client.get(f"/profiles/{PROFILE}/plugins/credential_broker/credentials/info")
        assert hot_detail["store"]["last_hydrated_unix_ms"] == startup_hydrated_at
        assert hot_detail["store"]["cached_count"] == 0
        assert credential_ref not in json.dumps(hot_detail)

    reloaded = client.post(
        f"/profiles/{PROFILE}/plugins/credential_broker/credentials/reload",
        {},
    )
    assert reloaded["store"]["backend"] == "disk_override"
    assert reloaded["store"]["ready"] is True
    assert reloaded["store"]["status"] == "ready"
    assert reloaded["store"]["last_error"] is None
    assert reloaded["store"]["cached_count"] == 1
    assert reloaded["store"]["last_hydrated_count"] == 1
    hydrated_at = reloaded["store"]["last_hydrated_unix_ms"]
    assert isinstance(hydrated_at, int)
    assert hydrated_at >= startup_hydrated_at
    assert raw_credential not in json.dumps(reloaded)
    assert credential_ref not in json.dumps(reloaded)

    for _ in range(3):
        detail = client.get(f"/profiles/{PROFILE}/plugins/credential_broker/credentials/info")
        assert detail["store"]["cached_count"] == 1
        assert detail["store"]["last_hydrated_count"] == 1
        assert detail["store"]["last_hydrated_unix_ms"] == hydrated_at
        assert detail["inventory"] == []
        assert raw_credential not in json.dumps(detail)
        assert credential_ref not in json.dumps(detail)

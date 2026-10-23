"""The inspect case owns a real public network lifecycle outside timing."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest

from benchmarks.helpers.route_cases import RouteCase
from benchmarks.helpers.route_fixtures import corpus_config, network_fixture
from benchmarks.helpers.route_validation import validate_wire

ID = "f1acb5b3-2f0a-450b-ae96-2f0980472a7f"


def test_network_fixture_uses_public_create_and_delete_even_after_failure() -> None:
    calls = []

    class Client:
        def post(self, path, body):
            calls.append(("POST", path, body))
            return {"id": ID, "name": corpus_config()["network_name"], "members": []}

        def delete(self, path):
            calls.append(("DELETE", path))
            return {"success": True}

    service = SimpleNamespace(client=lambda: Client())
    with pytest.raises(RuntimeError, match="sample failure"), network_fixture(service) as network_id:
        assert network_id == ID
        raise RuntimeError("sample failure")
    assert calls == [
        ("POST", "/networks", {"name": corpus_config()["network_name"]}),
        ("DELETE", f"/networks/{ID}"),
    ]


def test_inspect_case_uses_real_network_id_and_public_schema() -> None:
    case = RouteCase("GET", "/networks/{id}", "created-network")
    with pytest.raises(ValueError, match="network"):
        case.resolve(None)
    assert case.resolve(None, ID) == f"/networks/{ID}"
    info = {"id": ID, "name": corpus_config()["network_name"], "subnet": "10.42.0.0/24", "created_unix_ms": 123, "members": []}
    validate_wire(case, json.dumps(info).encode(), None, ID)
    for key, wrong in (("id", "wrong"), ("name", "wrong"), ("subnet", "0.0.0.0/0"), ("created_unix_ms", 0)):
        changed = {**info, key: wrong}
        with pytest.raises((ValueError, AssertionError)):
            validate_wire(case, json.dumps(changed).encode(), None, ID)


def test_network_logs_remain_uncovered_without_real_traffic() -> None:
    import tomllib
    from pathlib import Path

    cfg = tomllib.loads((Path(__file__).parents[1] / "config/gate.toml").read_text())["benchmark"]["routes"]
    assert "GET /networks/{id}/logs" in cfg["unmeasured"]
    assert not any(case["path"] == "/networks/{id}/logs" for case in cfg["measured"])
    assert "transport" in cfg["unmeasured"]["GET /networks/{id}/logs"]["reason"]

"""Source contracts for native filter stimuli; these do not claim VM acceptance."""

from __future__ import annotations

import ast
import json
import tomllib

import pytest
from helpers.content_filter_session import FIXTURE_HOST, fixture_config, upstream_records
from ironbank.test_content_filter_ledger import request_script


def test_hermetic_content_config_uses_existing_public_settings_and_scoped_rules():
    ready = {"dns_udp_addr": "127.0.0.1:42000", "http_addr": "127.0.0.1:43000"}
    config = tomllib.loads(fixture_config(ready))
    assert config["network"] == {
        "dns": {"upstreams": [ready["dns_udp_addr"]]},
        "upstream_overrides": {
            f"{FIXTURE_HOST}:443": {"dial": ready["http_addr"], "protocol": "http"}
        },
    }
    settings = config["settings"]
    assert settings["security.web.http_upstream_ports"]["value"] == [80, 43000]
    assert settings["vm.resources.log_bodies"]["value"] is True
    assert settings["vm.resources.max_body_capture"]["value"] == 8192
    assert config["ai"]["google"]["allowed_remote_targets"] == [f"{FIXTURE_HOST}:443"]
    binding = config["ai"]["google"]["rules"]["content_broker_binding"]
    assert binding["match"] == f'http.host == "{FIXTURE_HOST}" && tcp.port == "443"'
    rule = config["corp"]["rules"]["allow_content_fixture"]
    assert rule["action"] == "allow"
    assert rule["priority"] == -100
    assert "/content/(echo|text|stream)" in rule["match"]
    assert 'http.path == "/oauth/token" || http.path == "/echo"' in rule["match"]


def test_dns_exchanges_are_checked_separately_from_http_records():
    dns = {
        "kind": "dns",
        "qname": FIXTURE_HOST,
        "qtype": 1,
        "qclass": 1,
        "rcode": 0,
        "source_proto": "udp",
        "request_bytes": 37,
        "response_bytes": 53,
    }
    http = {"method": "POST", "path": "/content/echo", "headers": {}, "request_bytes": 3}
    transcript = "\n".join(json.dumps(row) for row in (dns, http))
    assert upstream_records(transcript) == [http]


@pytest.mark.parametrize(
    "bad",
    [
        {"kind": "unexpected"},
        {"kind": "dns", "qname": "unrelated.test"},
        {"method": "POST"},
    ],
)
def test_upstream_records_never_silently_discard_unknown_or_malformed_rows(bad):
    with pytest.raises((AssertionError, KeyError)):
        upstream_records(json.dumps(bad))


def test_missing_or_failed_dns_is_not_accepted_as_a_successful_native_path():
    with pytest.raises(AssertionError, match="DNS"):
        upstream_records('{"method":"POST","path":"/content/echo","headers":{},"request_bytes":1}')
    failed = {
        "kind": "dns",
        "qname": FIXTURE_HOST,
        "qtype": 1,
        "qclass": 1,
        "rcode": 3,
        "source_proto": "udp",
        "request_bytes": 37,
        "response_bytes": 37,
    }
    with pytest.raises(AssertionError):
        upstream_records(json.dumps(failed))


def test_guest_stimuli_remain_valid_python_for_binary_gzip_and_chunked_cases():
    for body in (b"demo.person@example.test", b"\xff\xe2\x82", "🌍 café 漢字".encode()):
        for options in (
            {"chunked": True},
            {"compress": True},
            {"media": "application/octet-stream"},
            {"headers": {"x-leaked-token": "synthetic"}},
        ):
            ast.parse(request_script(body, **options))

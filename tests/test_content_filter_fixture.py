"""Source contracts for native filter stimuli; these do not claim VM acceptance."""

from __future__ import annotations

import ast

import tomllib
from helpers.content_filter_session import FIXTURE_HOST, fixture_config
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
    assert config["ai"]["google"]["allowed_remote_targets"] == ["127.0.0.1:43000"]
    rule = config["corp"]["rules"]["allow_content_fixture"]
    assert rule["action"] == "allow"
    assert rule["priority"] == -100
    assert "/content/(echo|text|stream)" in rule["match"]
    assert 'http.path == "/oauth/token" || http.path == "/echo"' in rule["match"]


def test_guest_stimuli_remain_valid_python_for_binary_gzip_and_chunked_cases():
    for body in (b"demo.person@example.test", b"\xff\xe2\x82", "🌍 café 漢字".encode()):
        for options in (
            {"chunked": True},
            {"compress": True},
            {"media": "application/octet-stream"},
            {"headers": {"x-leaked-token": "synthetic"}},
        ):
            ast.parse(request_script(body, **options))

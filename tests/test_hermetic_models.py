"""Shipped-agent startup connections stay inside the hermetic upstream."""

import tomllib

from helpers.hermetic_models import model_corp_config


def test_claude_startup_platform_uses_the_same_mock_as_its_model_api():
    ready = {"http_addr": "127.0.0.1:12345", "dns_udp_addr": "127.0.0.1:12346"}
    corp = tomllib.loads(model_corp_config(ready))
    overrides = corp["network"]["upstream_overrides"]
    assert (
        overrides["platform.claude.com:443"]
        == overrides["api.anthropic.com:443"]
        == {
            "dial": ready["http_addr"],
            "protocol": "http",
        }
    )
    assert "unknown.claude.com:443" not in overrides
    assert corp["network"]["dns"]["upstreams"] == [ready["dns_udp_addr"]]

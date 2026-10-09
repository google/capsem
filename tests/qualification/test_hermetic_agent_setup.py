"""Agent startup must use the local model fixture before a workload is booted."""

import os
import tomllib
from pathlib import Path

import pytest

pytestmark = pytest.mark.integration


@pytest.mark.capability("agent")
def test_agent_provider_routing_is_local_before_workload_startup(service, candidate):
    configured = os.environ.get("CAPSEM_CORP_CONFIG")
    assert configured, "agent qualification must bind hermetic routing before startup"
    routing = tomllib.loads(Path(configured).read_text())["network"]
    assert routing["dns"]["upstreams"], routing
    for target in ["api.openai.com:443", "api.anthropic.com:443", "platform.claude.com:443"]:
        override = routing["upstream_overrides"][target]
        assert override["dial"].startswith("127.0.0.1:"), override
        assert override["protocol"] == "http", override

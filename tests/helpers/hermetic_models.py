"""Model providers answered by the hermetic mock, through Capsem's own routing.

An agent keeps its shipped configuration and dials its real provider host
(`api.anthropic.com`, `api.openai.com`, Google's endpoints); Capsem's corp
`network.upstream_overrides` deliver those connections to the mock model
server, and explicit allow rules admit them. No agent config is rewritten to
reach the fixture, so what is proven is the agent as it ships.
"""

from __future__ import annotations

import contextlib
import json
from collections.abc import Iterator
from pathlib import Path

from helpers.mock_server import start_mock_server, stop_process

#: The Ollama-compatible fixture endpoint a workload can name directly.
WORKLOAD_OLLAMA_HOST = "ollama.capsem.test"
WORKLOAD_OLLAMA_PORT = 3713
WORKLOAD_OLLAMA_URL = f"http://{WORKLOAD_OLLAMA_HOST}:{WORKLOAD_OLLAMA_PORT}"


def model_corp_config(ready: dict) -> str:
    """The corp config routing every provider host to the started mock."""
    return (
        f'''
refresh_policy = "24h"

[network.dns]
upstreams = [{json.dumps(ready["dns_udp_addr"])}]

[network.upstream_overrides."daily-cloudcode-pa.googleapis.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."generativelanguage.googleapis.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."www.googleapis.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."play.googleapis.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."antigravity-unleash.goog:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."api.openai.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."api.anthropic.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."platform.claude.com:443"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[network.upstream_overrides."{WORKLOAD_OLLAMA_HOST}:{WORKLOAD_OLLAMA_PORT}"]
dial = {json.dumps(ready["http_addr"])}
protocol = "http"

[settings."security.web.http_upstream_ports"]
value = [80, 3713, 8080, 11434]
modified = "2026-06-14T00:00:00Z"

[ai.ollama]
name = "Ollama"
protocol = "ollama"
url = "{WORKLOAD_OLLAMA_URL}"
listen_ports = [3713]
allowed_remote_targets = ["{WORKLOAD_OLLAMA_HOST}:{WORKLOAD_OLLAMA_PORT}"]

[ai.ollama.rules.local_fixture_endpoint]
name = "ollama_local_fixture_endpoint"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Declare the hermetic Ollama-compatible endpoint for Ironbank launcher tests."
match = 'http.host == "{WORKLOAD_OLLAMA_HOST}" && tcp.port == "{WORKLOAD_OLLAMA_PORT}" && (http.path == "/" || http.path == "/api/show" || http.path == "/api/tags" || http.path == "/api/chat" || http.path == "/v1/responses" || http.path == "/v1/messages")'

[corp.rules.allow_ironbank_mock_model_server]
name = "allow_ironbank_mock_model_server"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow the hermetic Ironbank model fixture while preserving local-network ask defaults."
match = 'http.host == "{WORKLOAD_OLLAMA_HOST}" && tcp.port == "{WORKLOAD_OLLAMA_PORT}" && (http.path == "/" || http.path == "/api/show" || http.path == "/api/tags" || http.path == "/api/chat" || http.path == "/v1/responses" || http.path == "/v1/messages")'

[corp.rules.allow_ironbank_google_code_assist]
name = "allow_ironbank_google_code_assist"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow hermetic AGY Google Code Assist replay through the declared upstream override."
match = 'tcp.port == "443" && ((http.host == "daily-cloudcode-pa.googleapis.com" && http.path.matches("^/v1internal:")) || (http.host == "www.googleapis.com" && http.path == "/oauth2/v2/userinfo") || (http.host == "play.googleapis.com" && http.path == "/log") || (http.host == "antigravity-unleash.goog" && http.path.matches("^/api/client/")))'

[corp.rules.allow_ironbank_gemini_api]
name = "allow_ironbank_gemini_api"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow hermetic Gemini API replay through the declared upstream override."
match = 'tcp.port == "443" && http.host == "generativelanguage.googleapis.com" && http.path.matches("^/v1beta/models/")'

[corp.rules.allow_ironbank_openai_api]
name = "allow_ironbank_openai_api"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow hermetic OpenAI API replay through the declared upstream override."
match = 'tcp.port == "443" && http.host == "api.openai.com" && http.path.matches("^/v1/")'

[corp.rules.allow_ironbank_anthropic_api]
name = "allow_ironbank_anthropic_api"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow hermetic Anthropic API replay through the declared upstream override."
match = 'tcp.port == "443" && http.host == "api.anthropic.com" && http.path.matches("^/v1/")'

[corp.rules.allow_ironbank_claude_connectivity]
name = "allow_ironbank_claude_connectivity"
action = "allow"
priority = -100
detection_level = "informational"
reason = "Allow the shipped Claude connectivity probes through declared hermetic upstreams."
match = 'tcp.port == "443" && ((http.host == "api.anthropic.com" && http.path == "/api/hello") || (http.host == "platform.claude.com" && http.path == "/v1/oauth/hello"))'
'''.strip()
        + "\n"
    )


@contextlib.contextmanager
def hermetic_models(tmp_dir: Path) -> Iterator[tuple[dict, Path]]:
    """Start the mock model server and write the corp config that routes the
    providers to it; yields (the server's addresses, the corp config path)."""
    process, ready = start_mock_server(
        request_log=tmp_dir / "upstream-transcript.jsonl", dns_answers="routable"
    )
    try:
        corp = tmp_dir / "corp.toml"
        corp.write_text(model_corp_config(ready), encoding="utf-8")
        yield ready, corp
    finally:
        stop_process(process)

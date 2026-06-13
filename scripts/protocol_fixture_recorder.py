#!/usr/bin/env python3
"""Record sanitized protocol fixtures from capsem-mock-server.

Ironbank note: recorder fixtures are inputs, not proof. The release proof lives
in tests/ironbank/ and must replay through Capsem as a black box, then assert
logs, DB rows, UDS/HTTP routes, counters, and UI-facing JSON without reading
Rust internals.
"""

import argparse
import json
import re
import socket
import struct
from pathlib import Path
from typing import Any, Literal
from urllib.error import HTTPError
from urllib.parse import urljoin
from urllib.request import Request, urlopen

import blake3
from pydantic import BaseModel, ConfigDict, Field

SECRET_RE = re.compile(r"capsem_test_[A-Za-z0-9_]+")

ProtocolFamily = Literal["http", "model", "mcp", "dns", "oauth", "credential"]
AuthMode = Literal["none", "bearer", "api_key", "oauth_code"]


class ClientInfo(BaseModel):
    name: str
    version: str


class HttpExchange(BaseModel):
    method: str
    path: str
    status_code: int
    request_headers: dict[str, str] = Field(default_factory=dict)
    request_body: Any = None
    response_headers: dict[str, str] = Field(default_factory=dict)
    response_body: Any = None


class ProtocolFixture(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    schema_: Literal["capsem.protocol_fixture.v1"] = Field(
        "capsem.protocol_fixture.v1",
        alias="schema",
    )
    name: str
    client: ClientInfo
    protocol_family: ProtocolFamily
    auth_mode: AuthMode
    exchange: HttpExchange
    expected_ledger_rows: list[str]
    expected_visible_bytes: int
    substitutions: dict[str, str] = Field(default_factory=dict)


class ReplayResult(BaseModel):
    name: str
    protocol_family: ProtocolFamily
    status_matches: bool
    visible_bytes_match: bool
    expected_status_code: int
    actual_status_code: int
    expected_visible_bytes: int
    actual_visible_bytes: int


def _substitution_for(secret: str) -> str:
    digest = blake3.blake3(secret.encode("utf-8")).hexdigest()
    return f"credential:blake3:{digest}"


def sanitize(value: Any, substitutions: dict[str, str] | None = None) -> Any:
    substitutions = substitutions if substitutions is not None else {}
    if isinstance(value, str):
        def replace(match: re.Match[str]) -> str:
            secret = match.group(0)
            replacement = substitutions.get(secret)
            if replacement is None:
                replacement = _substitution_for(secret)
                substitutions[secret] = replacement
            return replacement

        return SECRET_RE.sub(replace, value)
    if isinstance(value, list):
        return [sanitize(item, substitutions) for item in value]
    if isinstance(value, dict):
        return {key: sanitize(item, substitutions) for key, item in value.items()}
    return value


def _decode_body(body: bytes, content_type: str | None) -> Any:
    text = body.decode("utf-8", errors="replace")
    if content_type and "json" in content_type:
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text
    return text


def _http_exchange(
    base_url: str,
    method: str,
    path: str,
    *,
    headers: dict[str, str] | None = None,
    body: Any = None,
) -> tuple[HttpExchange, int, dict[str, str]]:
    headers = dict(headers or {})
    data: bytes | None = None
    if body is not None:
        if isinstance(body, (dict, list)):
            data = json.dumps(body, sort_keys=True).encode("utf-8")
            headers.setdefault("content-type", "application/json")
        elif isinstance(body, str):
            data = body.encode("utf-8")
        else:
            raise TypeError(f"unsupported request body type: {type(body)!r}")

    url = urljoin(base_url.rstrip("/") + "/", path.lstrip("/"))
    request = Request(url, data=data, headers=headers, method=method)
    try:
        with urlopen(request, timeout=10) as response:
            status_code = response.status
            response_headers = {key.lower(): value for key, value in response.headers.items()}
            response_body_bytes = response.read()
    except HTTPError as exc:
        with exc:
            status_code = exc.code
            response_headers = {key.lower(): value for key, value in exc.headers.items()}
            response_body_bytes = exc.read()

    substitutions: dict[str, str] = {}
    decoded_request = body
    if isinstance(body, str) and headers.get("content-type") == "application/x-www-form-urlencoded":
        decoded_request = body
    decoded_response = _decode_body(response_body_bytes, response_headers.get("content-type"))
    exchange = HttpExchange(
        method=method,
        path=path,
        status_code=status_code,
        request_headers=sanitize(headers, substitutions),
        request_body=sanitize(decoded_request, substitutions),
        response_headers=sanitize(response_headers, substitutions),
        response_body=sanitize(decoded_response, substitutions),
    )
    visible_bytes = len(json.dumps(exchange.response_body, sort_keys=True).encode("utf-8"))
    return exchange, visible_bytes, {
        _substitution_for(secret): replacement
        for secret, replacement in substitutions.items()
    }


def _scenario_definitions() -> list[dict[str, Any]]:
    model_body = {
        "model": "mock-local",
        "messages": [{"role": "user", "content": "hello from capsem recorder"}],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "fixture_lookup",
                    "parameters": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                    },
                },
            }
        ],
    }
    return [
        {
            "name": "anthropic_claude_messages",
            "client": {"name": "claude", "version": "fixture"},
            "protocol_family": "model",
            "auth_mode": "bearer",
            "method": "POST",
            "path": "/v1/chat/completions",
            "headers": {"authorization": "Bearer capsem_test_claude_bearer"},
            "body": {**model_body, "model": "claude-mock"},
            "expected_ledger_rows": [
                "net_events:/v1/chat/completions",
                "model_calls:request",
                "model_calls:response",
            ],
        },
        {
            "name": "openai_codex_chat_completions",
            "client": {"name": "codex", "version": "fixture"},
            "protocol_family": "model",
            "auth_mode": "api_key",
            "method": "POST",
            "path": "/v1/chat/completions",
            "headers": {"authorization": "Bearer capsem_test_openai_api_key"},
            "body": {**model_body, "model": "gpt-mock"},
            "expected_ledger_rows": [
                "net_events:/v1/chat/completions",
                "model_calls:request",
                "tool_calls:fixture_lookup",
            ],
        },
        {
            "name": "gemini_agy_generate_content",
            "client": {"name": "antigravity", "version": "fixture"},
            "protocol_family": "model",
            "auth_mode": "oauth_code",
            "method": "POST",
            "path": "/v1/chat/completions",
            "headers": {"authorization": "Bearer capsem_test_agy_oauth_access"},
            "body": {**model_body, "model": "gemini-mock"},
            "expected_ledger_rows": [
                "net_events:/v1/chat/completions",
                "model_calls:request",
                "model_calls:response",
            ],
        },
        {
            "name": "ollama_openai_chat_completions",
            "client": {"name": "ollama", "version": "fixture"},
            "protocol_family": "model",
            "auth_mode": "none",
            "method": "POST",
            "path": "/v1/chat/completions",
            "body": {**model_body, "model": "gemma4:latest"},
            "expected_ledger_rows": [
                "net_events:/v1/chat/completions",
                "model_calls:request",
                "model_calls:response",
            ],
        },
        {
            "name": "oauth_token_exchange",
            "client": {"name": "oauth-provider", "version": "fixture"},
            "protocol_family": "oauth",
            "auth_mode": "oauth_code",
            "method": "POST",
            "path": "/oauth/token",
            "headers": {"content-type": "application/x-www-form-urlencoded"},
            "body": (
                "grant_type=authorization_code"
                "&code=capsem_test_oauth_code_0123456789abcdef"
                "&client_secret=capsem_test_oauth_client_secret"
            ),
            "expected_ledger_rows": [
                "net_events:/oauth/token",
                "credential_broker_events:captured",
            ],
        },
        {
            "name": "mcp_tools_list",
            "client": {"name": "mcp-json-rpc", "version": "2024-11-05"},
            "protocol_family": "mcp",
            "auth_mode": "none",
            "method": "POST",
            "path": "/mcp",
            "body": {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
            "expected_ledger_rows": ["net_events:/mcp", "mcp_events:tools/list"],
        },
        {
            "name": "mcp_tool_call",
            "client": {"name": "mcp-json-rpc", "version": "2024-11-05"},
            "protocol_family": "mcp",
            "auth_mode": "none",
            "method": "POST",
            "path": "/mcp",
            "body": {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "fixture_lookup", "arguments": {"query": "capsem"}},
            },
            "expected_ledger_rows": ["net_events:/mcp", "mcp_events:tools/call"],
        },
        {
            "name": "credential_response_capture",
            "client": {"name": "credential-broker", "version": "fixture"},
            "protocol_family": "credential",
            "auth_mode": "none",
            "method": "GET",
            "path": "/credential/response",
            "expected_ledger_rows": [
                "net_events:/credential/response",
                "credential_broker_events:captured",
            ],
        },
    ]


def _dns_scenario_definitions() -> list[dict[str, Any]]:
    return [
        {
            "name": "dns_a_fixture",
            "client": {"name": "dns-client", "version": "fixture"},
            "protocol_family": "dns",
            "auth_mode": "none",
            "qname": "fixture.capsem.test",
            "qtype": 1,
            "expected_ledger_rows": ["dns_events:fixture.capsem.test"],
        }
    ]


def _dns_query(name: str, qtype: int = 1, query_id: int = 0xCACE) -> bytes:
    labels = b"".join(bytes([len(part)]) + part.encode("ascii") for part in name.split("."))
    question = labels + b"\0" + struct.pack("!HH", qtype, 1)
    return struct.pack("!HHHHHH", query_id, 0x0100, 1, 0, 0, 0) + question


def _parse_dns_a_response(response: bytes) -> dict[str, Any]:
    if len(response) < 12:
        raise ValueError("truncated DNS response")
    query_id, flags, qdcount, ancount, _, _ = struct.unpack("!HHHHHH", response[:12])
    rcode = flags & 0x000F
    answers: list[str] = []
    offset = 12
    for _ in range(qdcount):
        while response[offset] != 0:
            offset += 1 + response[offset]
        offset += 1 + 4
    for _ in range(ancount):
        if offset + 12 > len(response):
            raise ValueError("truncated DNS answer")
        _name, rr_type, rr_class, ttl, rdlength = struct.unpack("!HHHIH", response[offset:offset + 12])
        offset += 12
        rdata = response[offset:offset + rdlength]
        offset += rdlength
        if rr_type == 1 and rr_class == 1 and ttl == 60 and rdlength == 4:
            answers.append(".".join(str(part) for part in rdata))
    return {
        "query_id": query_id,
        "rcode": rcode,
        "answers": answers,
    }


def _dns_exchange(dns_udp_addr: str, qname: str, qtype: int) -> tuple[HttpExchange, int]:
    host, port_text = dns_udp_addr.rsplit(":", 1)
    query = _dns_query(qname, qtype=qtype)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.settimeout(5)
        sock.sendto(query, (host, int(port_text)))
        response, _ = sock.recvfrom(512)
    parsed = _parse_dns_a_response(response)
    exchange = HttpExchange(
        method="DNS",
        path=qname,
        status_code=0 if parsed["rcode"] == 0 else parsed["rcode"],
        request_body={"qname": qname, "qtype": qtype, "qclass": 1},
        response_body=parsed,
    )
    visible_bytes = len(json.dumps(exchange.response_body, sort_keys=True).encode("utf-8"))
    return exchange, visible_bytes


def record_mock_server(
    base_url: str,
    output_dir: str | Path,
    *,
    dns_udp_addr: str | None = None,
    scenarios: set[str] | None = None,
) -> list[Path]:
    output_path = Path(output_dir)
    output_path.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    for scenario in _scenario_definitions():
        if scenarios and scenario["name"] not in scenarios:
            continue
        exchange, visible_bytes, substitutions = _http_exchange(
            base_url,
            scenario["method"],
            scenario["path"],
            headers=scenario.get("headers"),
            body=scenario.get("body"),
        )
        fixture = ProtocolFixture(
            name=scenario["name"],
            client=ClientInfo.model_validate(scenario["client"]),
            protocol_family=scenario["protocol_family"],
            auth_mode=scenario["auth_mode"],
            exchange=exchange,
            expected_ledger_rows=scenario["expected_ledger_rows"],
            expected_visible_bytes=visible_bytes,
            substitutions=substitutions,
        )
        destination = output_path / f"{fixture.name}.json"
        destination.write_text(fixture.model_dump_json(indent=2, by_alias=True) + "\n")
        written.append(destination)
    for scenario in _dns_scenario_definitions():
        if scenarios and scenario["name"] not in scenarios:
            continue
        if not dns_udp_addr:
            if scenarios and scenario["name"] in scenarios:
                raise ValueError(f"DNS fixture scenario {scenario['name']} requires dns_udp_addr")
            continue
        exchange, visible_bytes = _dns_exchange(
            dns_udp_addr,
            scenario["qname"],
            scenario["qtype"],
        )
        fixture = ProtocolFixture(
            name=scenario["name"],
            client=ClientInfo.model_validate(scenario["client"]),
            protocol_family=scenario["protocol_family"],
            auth_mode=scenario["auth_mode"],
            exchange=exchange,
            expected_ledger_rows=scenario["expected_ledger_rows"],
            expected_visible_bytes=visible_bytes,
        )
        destination = output_path / f"{fixture.name}.json"
        destination.write_text(fixture.model_dump_json(indent=2, by_alias=True) + "\n")
        written.append(destination)
    if scenarios:
        missing = scenarios - {path.stem for path in written}
        if missing:
            raise ValueError(f"unknown protocol fixture scenario(s): {', '.join(sorted(missing))}")
    return written


def replay_fixtures(
    base_url: str,
    fixture_paths: list[str | Path],
    *,
    dns_udp_addr: str | None = None,
) -> list[ReplayResult]:
    results: list[ReplayResult] = []
    for path in fixture_paths:
        fixture = ProtocolFixture.model_validate_json(Path(path).read_text())
        if fixture.protocol_family == "dns":
            if not dns_udp_addr:
                raise ValueError(f"replaying DNS fixture {fixture.name} requires dns_udp_addr")
            qtype = int((fixture.exchange.request_body or {}).get("qtype", 1))
            exchange, visible_bytes = _dns_exchange(dns_udp_addr, fixture.exchange.path, qtype)
        else:
            exchange, visible_bytes, _substitutions = _http_exchange(
                base_url,
                fixture.exchange.method,
                fixture.exchange.path,
                headers=dict(fixture.exchange.request_headers),
                body=fixture.exchange.request_body,
            )
        results.append(
            ReplayResult(
                name=fixture.name,
                protocol_family=fixture.protocol_family,
                status_matches=exchange.status_code == fixture.exchange.status_code,
                visible_bytes_match=visible_bytes == fixture.expected_visible_bytes,
                expected_status_code=fixture.exchange.status_code,
                actual_status_code=exchange.status_code,
                expected_visible_bytes=fixture.expected_visible_bytes,
                actual_visible_bytes=visible_bytes,
            )
        )
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True, help="capsem-mock-server base URL")
    parser.add_argument("--dns-udp-addr", help="capsem-mock-server DNS UDP address")
    parser.add_argument("--out-dir", required=True, type=Path, help="fixture output directory")
    parser.add_argument(
        "--replay",
        action="store_true",
        help="replay written fixtures after recording and include replay results",
    )
    parser.add_argument(
        "--scenario",
        action="append",
        dest="scenarios",
        help="scenario name to record; may be repeated",
    )
    args = parser.parse_args()
    written = record_mock_server(
        args.base_url,
        args.out_dir,
        dns_udp_addr=args.dns_udp_addr,
        scenarios=set(args.scenarios) if args.scenarios else None,
    )
    output: dict[str, Any] = {"written": [str(path) for path in written]}
    if args.replay:
        output["replay"] = [
            result.model_dump()
            for result in replay_fixtures(
                args.base_url,
                written,
                dns_udp_addr=args.dns_udp_addr,
            )
        ]
    print(json.dumps(output, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

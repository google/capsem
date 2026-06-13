#!/usr/bin/env python3
"""Capsem's deterministic local mock server runtime."""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import json
import socketserver
import struct
import sys
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse


TINY_BODY = b"capsem-mock-server:tiny\n"
EXPECTED_POEM = "Capsem ironbank poem\nledgers count the sparks\nno secret crosses raw"
HTML_ABOUT = """<!doctype html>
<html>
  <head><title>Capsem Mock Server About</title></head>
  <body>
    <div id="about">
      <p>Capsem mock server about page for local MCP fetch tests.</p>
      <p>Google, Anthropic, and OpenAI appear here as fixture text only.</p>
      <a href="https://example.invalid/local">Local fixture link</a>
    </div>
  </body>
</html>
"""
ENDPOINTS = [
    "/tiny",
    "/html/about",
    "/html/large",
    "/bytes/{size}",
    "/gzip/{size}",
    "/sse/model",
    "/model/response",
    "/v1/chat/completions",
    "/oauth/authorize",
    "/oauth/token",
    "/mcp",
    "/slow-chunks",
    "/credential/response",
    "/echo",
    "/deny-target",
    "/ws/echo",
    "/ws/ping",
    "/ws/close",
]
DNS_FIXTURES = {
    "fixture.capsem.test": "127.0.0.1",
    "model.capsem.test": "127.0.0.1",
    "mcp.capsem.test": "127.0.0.1",
}


def _deterministic_bytes(size: str) -> bytes:
    lengths = {"10kb": 10 * 1024, "1mb": 1024 * 1024, "10mb": 10 * 1024 * 1024}
    try:
        length = lengths[size.lower()]
    except KeyError as exc:
        raise ValueError(f"unsupported size '{size}'") from exc
    return bytes(ord("a") + (idx % 26) for idx in range(length))


def _model_payload(model: str = "mock-local") -> dict:
    return {
        "id": "chatcmpl-mock-local",
        "object": "chat.completion",
        "provider": "mock",
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": EXPECTED_POEM,
                    "tool_calls": [
                        {
                            "id": "tool_0001",
                            "type": "function",
                            "function": {
                                "name": "fixture_lookup",
                                "arguments": '{"query":"capsem"}',
                            },
                        }
                    ],
                },
                "finish_reason": "tool_calls",
            }
        ],
        "usage": {
            "prompt_tokens": 7,
            "completion_tokens": 5,
            "total_tokens": 12,
        },
    }


class MockHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "capsem-mock-server/1.0"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def _body(self) -> bytes:
        length = int(self.headers.get("content-length") or "0")
        return self.rfile.read(length) if length else b""

    def _json_body(self) -> dict:
        body = self._body()
        if not body:
            return {}
        try:
            value = json.loads(body)
        except json.JSONDecodeError:
            return {}
        return value if isinstance(value, dict) else {}

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_json(self, value: object, status: int = HTTPStatus.OK) -> None:
        body = json.dumps(value, separators=(",", ":")).encode()
        self._send(status, body, "application/json")

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if self.headers.get("upgrade", "").lower() == "websocket":
            self._websocket(path)
            return
        if path == "/tiny":
            self._send(HTTPStatus.OK, TINY_BODY, "text/plain; charset=utf-8")
        elif path == "/html/about":
            self._send(HTTPStatus.OK, HTML_ABOUT.encode(), "text/html; charset=utf-8")
        elif path == "/html/large":
            body = "<!doctype html><html><body><main>\n"
            for idx in range(80):
                body += (
                    f"<p>Capsem local pagination fixture paragraph {idx}: "
                    "mock server content for MCP fetch tests.</p>\n"
                )
            body += "</main></body></html>\n"
            self._send(HTTPStatus.OK, body.encode(), "text/html; charset=utf-8")
        elif path.startswith("/bytes/"):
            self._bytes(path.removeprefix("/bytes/"), gzip_body=False)
        elif path.startswith("/gzip/"):
            self._bytes(path.removeprefix("/gzip/"), gzip_body=True)
        elif path == "/sse/model":
            body = (
                'event: model.delta\ndata: {"provider":"mock","model":"mock-local",'
                '"content":"hello"}\n\n'
                'event: model.tool_call\ndata: {"id":"tool_0001","name":"fixture_lookup",'
                '"arguments":{"query":"capsem"}}\n\n'
                'event: model.done\ndata: {"finish_reason":"stop"}\n\n'
            ).encode()
            self._send(HTTPStatus.OK, body, "text/event-stream")
        elif path == "/model/response":
            self._send_json(_model_payload())
        elif path == "/oauth/authorize":
            self._send_json(
                {
                    "kind": "synthetic_oauth_authorization_fixture",
                    "authorization_code": "capsem_test_oauth_code_0123456789abcdef",
                    "redirect_uri": "https://capsem.invalid/oauth/callback",
                    "state": "capsem-fixture-state",
                    "scope": "openid profile email offline_access",
                }
            )
        elif path == "/slow-chunks":
            self.send_response(HTTPStatus.OK)
            self.send_header("content-type", "text/plain; charset=utf-8")
            self.send_header("connection", "close")
            self.end_headers()
            for idx in range(4):
                time.sleep(0.01)
                self.wfile.write(f"chunk-{idx}\n".encode())
                self.wfile.flush()
            self.close_connection = True
        elif path == "/credential/response":
            self._send_json(
                {
                    "kind": "synthetic_credential_fixture",
                    "api_key": "sk-capsem_test_api_key_0123456789abcdef",
                    "oauth": {
                        "access_token": "capsem_test_oauth_access_0123456789abcdef",
                        "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
                        "expires_in": 3600,
                    },
                }
            )
        elif path == "/deny-target":
            self._send(HTTPStatus.OK, b"capsem-mock-server:deny-target\n", "text/plain")
        else:
            self._send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)

    def do_POST(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/v1/chat/completions":
            payload = self._json_body()
            model = payload.get("model") if isinstance(payload.get("model"), str) else "mock-local"
            self._send_json(_model_payload(model))
        elif path == "/oauth/token":
            self._body()
            self._send_json(
                {
                    "kind": "synthetic_oauth_token_fixture",
                    "token_type": "Bearer",
                    "access_token": "capsem_test_oauth_access_0123456789abcdef",
                    "refresh_token": "capsem_test_oauth_refresh_0123456789abcdef",
                    "id_token": "capsem_test_oauth_id_0123456789abcdef",
                    "expires_in": 3600,
                    "scope": "openid profile email offline_access",
                }
            )
        elif path == "/mcp":
            self._mcp(self._json_body())
        elif path == "/echo":
            body = self._body()
            lower_headers = {key.lower(): value for key, value in self.headers.items()}
            authorization = lower_headers.get("authorization", "")
            self._send_json(
                {
                    "method": "POST",
                    "path": "/echo",
                    "body_size": len(body),
                    "content_type": lower_headers.get("content-type"),
                    "user_agent": lower_headers.get("user-agent"),
                    "header_count": len(self.headers),
                    "has_authorization": "authorization" in lower_headers,
                    "authorization_is_broker_ref": "credential:blake3:" in authorization,
                    "query_has_broker_ref": "credential:blake3:" in parsed.query,
                    "query_has_access_token": "access_token=" in parsed.query,
                    "has_cookie": "cookie" in lower_headers,
                    "has_x_api_key": "x-api-key" in lower_headers,
                }
            )
        else:
            self._send_json({"error": "not found"}, HTTPStatus.NOT_FOUND)

    def _bytes(self, size: str, *, gzip_body: bool) -> None:
        try:
            data = _deterministic_bytes(size)
        except ValueError as exc:
            self._send_json({"error": str(exc), "allowed": ["10kb", "1mb", "10mb"]}, 400)
            return
        if gzip_body:
            data = gzip.compress(data)
            self.send_response(HTTPStatus.OK)
            self.send_header("content-type", "application/octet-stream")
            self.send_header("content-encoding", "gzip")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self._send(HTTPStatus.OK, data, "application/octet-stream")

    def _mcp(self, payload: dict) -> None:
        request_id = payload.get("id")
        method = payload.get("method")
        if method == "initialize":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": False}, "resources": {}},
                    "serverInfo": {"name": "capsem-mock-server", "version": "1.0.0"},
                },
            }
        elif method == "tools/list":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "tools": [
                        {
                            "name": "fixture_lookup",
                            "description": "Return deterministic debug content.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"query": {"type": "string"}},
                            },
                        },
                        {
                            "name": "fetch_http",
                            "description": "Fetch a local mock server URL.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"url": {"type": "string"}},
                            },
                        },
                        {
                            "name": "slow_sleep",
                            "description": "Sleep before returning deterministic text.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    ]
                },
            }
        elif method == "tools/call":
            name = payload.get("params", {}).get("name", "unknown")
            if name == "slow_sleep":
                time.sleep(3)
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "content": [
                        {"type": "text", "text": f"capsem-mock-server:mcp:{name}"}
                    ],
                    "isError": False,
                },
            }
        elif method == "resources/list":
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "resources": [
                        {
                            "uri": "doc://slow",
                            "name": "slow-doc",
                            "description": "Slow deterministic resource.",
                            "mimeType": "text/plain",
                        }
                    ]
                },
            }
        elif method == "resources/read":
            if payload.get("params", {}).get("uri") == "doc://slow":
                time.sleep(3)
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "contents": [
                        {
                            "uri": payload.get("params", {}).get("uri", "doc://unknown"),
                            "mimeType": "text/plain",
                            "text": "capsem-mock-server:mcp:resource",
                        }
                    ]
                },
            }
        else:
            response = {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": "method not found"},
            }
        self._send_json(response)

    def _websocket(self, path: str) -> None:
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        accept = base64.b64encode(
            hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        ).decode()
        self.send_response(HTTPStatus.SWITCHING_PROTOCOLS)
        self.send_header("upgrade", "websocket")
        self.send_header("connection", "Upgrade")
        self.send_header("sec-websocket-accept", accept)
        self.end_headers()
        if path == "/ws/close":
            self._ws_send_close()
            return
        if path == "/ws/ping":
            self._ws_send_frame(0x9, b"capsem-ping")
        if path != "/ws/echo":
            return
        while True:
            frame = self._ws_read_frame()
            if frame is None:
                return
            opcode, payload = frame
            if opcode == 0x8:
                self._ws_send_close()
                return
            if opcode in {0x1, 0x2}:
                self._ws_send_frame(opcode, payload)
            elif opcode == 0x9:
                self._ws_send_frame(0xA, payload)

    def _ws_read_frame(self) -> tuple[int, bytes] | None:
        head = self.connection.recv(2)
        if len(head) < 2:
            return None
        first, second = head
        opcode = first & 0x0F
        masked = second & 0x80
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", self.connection.recv(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self.connection.recv(8))[0]
        mask = self.connection.recv(4) if masked else b"\0\0\0\0"
        payload = bytearray()
        while len(payload) < length:
            chunk = self.connection.recv(length - len(payload))
            if not chunk:
                return None
            payload.extend(chunk)
        if masked:
            payload = bytearray(byte ^ mask[idx % 4] for idx, byte in enumerate(payload))
        return opcode, bytes(payload)

    def _ws_send_frame(self, opcode: int, payload: bytes) -> None:
        header = bytearray([0x80 | opcode])
        length = len(payload)
        if length < 126:
            header.append(length)
        elif length <= 0xFFFF:
            header.extend([126])
            header.extend(struct.pack("!H", length))
        else:
            header.extend([127])
            header.extend(struct.pack("!Q", length))
        self.connection.sendall(bytes(header) + payload)

    def _ws_send_close(self) -> None:
        self._ws_send_frame(0x8, struct.pack("!H", 1000) + b"capsem-fixture-close")


def _decode_dns_name(packet: bytes, offset: int = 12) -> tuple[str, int]:
    labels: list[str] = []
    while True:
        if offset >= len(packet):
            raise ValueError("truncated dns name")
        length = packet[offset]
        offset += 1
        if length == 0:
            break
        if length & 0xC0:
            raise ValueError("compressed dns query names are unsupported in fixtures")
        if offset + length > len(packet):
            raise ValueError("truncated dns label")
        labels.append(packet[offset:offset + length].decode("ascii").lower())
        offset += length
    return ".".join(labels), offset


def _dns_response(packet: bytes) -> bytes:
    if len(packet) < 12:
        return b""
    query_id, _flags, qdcount, _ancount, _nscount, _arcount = struct.unpack("!HHHHHH", packet[:12])
    if qdcount != 1:
        return struct.pack("!HHHHHH", query_id, 0x8183, qdcount, 0, 0, 0) + packet[12:]
    try:
        qname, offset = _decode_dns_name(packet)
    except ValueError:
        return struct.pack("!HHHHHH", query_id, 0x8183, 0, 0, 0, 0)
    if offset + 4 > len(packet):
        return struct.pack("!HHHHHH", query_id, 0x8183, 0, 0, 0, 0)
    qtype, qclass = struct.unpack("!HH", packet[offset:offset + 4])
    question = packet[12:offset + 4]
    address = DNS_FIXTURES.get(qname)
    if qtype != 1 or qclass != 1 or address is None:
        return struct.pack("!HHHHHH", query_id, 0x8183, 1, 0, 0, 0) + question
    rdata = bytes(int(part) for part in address.split("."))
    answer = (
        struct.pack("!HHHIH", 0xC00C, 1, 1, 60, len(rdata))
        + rdata
    )
    return struct.pack("!HHHHHH", query_id, 0x8180, 1, 1, 0, 0) + question + answer


class DnsUdpHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        data, socket = self.request
        response = _dns_response(data)
        if response:
            socket.sendto(response, self.client_address)


class DnsTcpHandler(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        length_bytes = self.request.recv(2)
        if len(length_bytes) != 2:
            return
        length = struct.unpack("!H", length_bytes)[0]
        packet = b""
        while len(packet) < length:
            chunk = self.request.recv(length - len(packet))
            if not chunk:
                return
            packet += chunk
        response = _dns_response(packet)
        if response:
            self.request.sendall(struct.pack("!H", len(response)) + response)


class ThreadingUdpServer(socketserver.ThreadingMixIn, socketserver.UDPServer):
    daemon_threads = True
    allow_reuse_address = True


class ThreadingTcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    daemon_threads = True
    allow_reuse_address = True


def _ready_payload(
    http_addr: tuple[str, int],
    dns_udp_addr: tuple[str, int],
    dns_tcp_addr: tuple[str, int],
) -> dict:
    host, port = http_addr
    dns_udp_host, dns_udp_port = dns_udp_addr
    dns_tcp_host, dns_tcp_port = dns_tcp_addr
    return {
        "service": "capsem-mock-server",
        "http_addr": f"{host}:{port}",
        "base_url": f"http://{host}:{port}",
        "dns_udp_addr": f"{dns_udp_host}:{dns_udp_port}",
        "dns_tcp_addr": f"{dns_tcp_host}:{dns_tcp_port}",
        "dns_fixtures": sorted(DNS_FIXTURES),
        "endpoints": ENDPOINTS,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--addr", default="127.0.0.1:0")
    args = parser.parse_args()
    host, port_text = args.addr.rsplit(":", 1)
    server = ThreadingHTTPServer((host, int(port_text)), MockHandler)
    dns_udp = ThreadingUdpServer((host, 0), DnsUdpHandler)
    dns_tcp = ThreadingTcpServer((host, 0), DnsTcpHandler)
    print(
        json.dumps(
            _ready_payload(
                server.server_address,
                dns_udp.server_address,
                dns_tcp.server_address,
            )
        ),
        flush=True,
    )
    threads = [
        threading.Thread(target=server.serve_forever, daemon=True),
        threading.Thread(target=dns_udp.serve_forever, daemon=True),
        threading.Thread(target=dns_tcp.serve_forever, daemon=True),
    ]
    for thread in threads:
        thread.start()
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        for fixture_server in (server, dns_udp, dns_tcp):
            fixture_server.shutdown()
            fixture_server.server_close()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as exc:
        print(f"capsem-mock-server failed: {exc}", file=sys.stderr)
        raise

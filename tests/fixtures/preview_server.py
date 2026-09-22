"""Small HTTP/WebSocket workload for the real preview acceptance test."""

import base64
import hashlib
import json
import struct
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HITS = Path("/root/preview-hits")
WEBSOCKET_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        del format, args

    def hit(self):
        with HITS.open("a") as stream:
            stream.write(f"{self.command} {self.path}\n")

    def body(self, status, body, content_type="text/plain"):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        self.hit()
        length = int(self.headers.get("Content-Length", "0"))
        payload = self.rfile.read(length)
        self.body(200, b"form:" + payload)

    def do_GET(self):
        self.hit()
        if self.path == "/":
            body = b'<html><script src="/asset.js"></script><form method="post" action="/form"></form></html>'
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Set-Cookie", "app_session=guest; HttpOnly")
            self.send_header("Set-Cookie", "capsem_preview=forged; Path=/")
            self.end_headers()
            self.wfile.write(body)
        elif self.path == "/asset.js":
            self.body(200, b"window.previewLoaded=true", "application/javascript")
        elif self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/final")
            self.send_header("Content-Length", "0")
            self.end_headers()
        elif self.path == "/final":
            self.body(200, b"redirected")
        elif self.path == "/headers":
            payload = json.dumps(
                {
                    "authorization": "Authorization" in self.headers,
                    "proxy_authorization": "Proxy-Authorization" in self.headers,
                    "cookie": self.headers.get("Cookie", ""),
                },
                sort_keys=True,
            ).encode()
            self.body(200, payload, "application/json")
        elif self.path == "/stream":
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            for part in (b"stream-", b"complete"):
                self.wfile.write(f"{len(part):x}\r\n".encode() + part + b"\r\n")
                self.wfile.flush()
                time.sleep(0.02)
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
        elif self.path == "/live":
            self.websocket()
        else:
            self.body(404, b"missing")

    def websocket(self):
        key = self.headers["Sec-WebSocket-Key"]
        digest = hashlib.sha1(
            (key + WEBSOCKET_GUID).encode(), usedforsecurity=False
        ).digest()
        accept = base64.b64encode(digest).decode()
        assert "Authorization" not in self.headers
        assert "Proxy-Authorization" not in self.headers
        assert "capsem_preview=" not in self.headers.get("Cookie", "")
        self.send_response(101)
        self.send_header("Connection", "Upgrade")
        self.send_header("Upgrade", "websocket")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        first, second = self.rfile.read(2)
        assert first == 0x81 and second & 0x80
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", self.rfile.read(2))[0]
        mask = self.rfile.read(4)
        payload = bytes(
            byte ^ mask[index % 4] for index, byte in enumerate(self.rfile.read(length))
        )
        reply = b"echo:" + payload
        assert len(reply) < 126
        self.connection.sendall(bytes([0x81, len(reply)]) + reply)


class Server(ThreadingHTTPServer):
    daemon_threads = True


Server(("127.0.0.1", 8080), Handler).serve_forever()

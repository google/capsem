"""Local HTTPS OCI v2 loopback registry serving content-addressed blobs by digest."""

from __future__ import annotations

import errno
import fcntl
import http.server
import re
import shutil
import socketserver
import ssl
import subprocess
import tempfile
import threading
import urllib.request
from pathlib import Path
from typing import Any

from .oci_ingest import _OCI_MANIFEST_MEDIA_TYPE, default_cache_dir

DEFAULT_REGISTRY_PORT = 5055
_MANIFEST_PATH_RE = re.compile(r"^/v2/.+/manifests/sha256:([0-9a-f]{64})$")
_BLOB_PATH_RE = re.compile(r"^/v2/.+/blobs/sha256:([0-9a-f]{64})$")
_LOCK = threading.Lock()
_TLS_LOCK = threading.Lock()
_SERVERS: dict[tuple[str, int], tuple[http.server.HTTPServer, int, str]] = {}


def configured_registry_port() -> int:
    """Return the configured loopback OCI registry port (`5055` by default)."""
    import os

    raw = os.environ.get("CAPSEM_INSPECT_BUILD_REGISTRY_PORT", "").strip()
    return int(raw) if raw else DEFAULT_REGISTRY_PORT


def ensure_localhost_tls(cache_dir: Path | None = None) -> tuple[Path, Path, str]:
    """Ensure a local CA and `127.0.0.1` server certificate exist; return `(cert, key, ca_pem)`."""
    c_dir = (cache_dir or default_cache_dir()).resolve()
    tls_dir = c_dir / "tls"
    tls_dir.mkdir(parents=True, exist_ok=True)
    ca_cert = tls_dir / "ca.pem"
    srv_cert = tls_dir / "server.pem"
    srv_key = tls_dir / "server.key"
    lock_path = tls_dir / ".tls.lock"
    with _TLS_LOCK, lock_path.open("a+", encoding="utf-8") as lock_fh:
        fcntl.flock(lock_fh.fileno(), fcntl.LOCK_EX)
        if ca_cert.is_file() and srv_cert.is_file() and srv_key.is_file():
            return srv_cert, srv_key, ca_cert.read_text(encoding="utf-8")

        openssl = shutil.which("openssl")
        if openssl is None:
            raise RuntimeError(
                "openssl is required on PATH to mint loopback OCI registry TLS certs"
            )

        with tempfile.TemporaryDirectory(dir=tls_dir, prefix=".tls-") as tmp:
            t_dir = Path(tmp)
            cnf, csr = t_dir / "openssl.cnf", t_dir / "server.csr"
            t_ca_key, t_ca_cert = t_dir / "ca.key", t_dir / "ca.pem"
            t_srv_key, t_srv_cert = t_dir / "server.key", t_dir / "server.pem"
            cnf.write_text(
                "[req]\ndistinguished_name=dn\nprompt=no\n"
                "[dn]\nCN=Capsem Local Build CA\n"
                "[ca]\nbasicConstraints=critical,CA:TRUE,pathlen:0\n"
                "keyUsage=critical,keyCertSign,cRLSign\n"
                "subjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always,issuer\n"
                "[srv]\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n"
                "basicConstraints=critical,CA:FALSE\n"
                "keyUsage=critical,digitalSignature,keyEncipherment\n"
                "extendedKeyUsage=serverAuth\nsubjectKeyIdentifier=hash\n"
                "authorityKeyIdentifier=keyid:always\n",
                encoding="utf-8",
            )
            ec_args = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"]
            for cmd in (
                [
                    openssl,
                    "req",
                    "-x509",
                    *ec_args,
                    "-days",
                    "3650",
                    "-keyout",
                    str(t_ca_key),
                    "-out",
                    str(t_ca_cert),
                    "-config",
                    str(cnf),
                    "-extensions",
                    "ca",
                ],
                [
                    openssl,
                    "req",
                    *ec_args,
                    "-keyout",
                    str(t_srv_key),
                    "-out",
                    str(csr),
                    "-subj",
                    "/CN=127.0.0.1",
                ],
                [
                    openssl,
                    "x509",
                    "-req",
                    "-in",
                    str(csr),
                    "-CA",
                    str(t_ca_cert),
                    "-CAkey",
                    str(t_ca_key),
                    "-CAcreateserial",
                    "-out",
                    str(t_srv_cert),
                    "-days",
                    "3650",
                    "-extfile",
                    str(cnf),
                    "-extensions",
                    "srv",
                ],
            ):
                subprocess.run(cmd, check=True, capture_output=True, text=True, timeout=15)
            for name in ("ca.key", "server.key", "server.pem", "ca.pem"):
                (t_dir / name).replace(tls_dir / name)
        return srv_cert, srv_key, ca_cert.read_text(encoding="utf-8")


class _ReusableHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def _make_handler(blobs_dir: Path) -> type[http.server.BaseHTTPRequestHandler]:
    class _OciHandler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, format: str, *args: Any) -> None:
            return

        def _send_headers_only(self, code: int, headers: dict[str, str]) -> None:
            self.send_response(code)
            for k, v in headers.items():
                self.send_header(k, v)
            self.end_headers()

        def _handle(self, *, head_only: bool) -> None:
            path = self.path.split("?", 1)[0]
            if path in ("/v2", "/v2/"):
                self._send_headers_only(
                    200,
                    {"Docker-Distribution-Api-Version": "registry/2.0", "Content-Length": "2"},
                )
                if not head_only:
                    self.wfile.write(b"{}")
                return

            m_match = _MANIFEST_PATH_RE.match(path)
            b_match = _BLOB_PATH_RE.match(path) if m_match is None else None
            match = m_match or b_match
            if match is None:
                self._send_headers_only(404, {"Content-Length": "0"})
                return
            hex_digest = match.group(1)
            blob_file = blobs_dir / hex_digest
            if not blob_file.is_file():
                self._send_headers_only(404, {"Content-Length": "0"})
                return
            data = blob_file.read_bytes()
            ctype = _OCI_MANIFEST_MEDIA_TYPE if m_match else "application/octet-stream"
            self._send_headers_only(
                200,
                {
                    "Content-Type": ctype,
                    "Content-Length": str(len(data)),
                    "Docker-Content-Digest": f"sha256:{hex_digest}",
                },
            )
            if not head_only:
                self.wfile.write(data)

        def do_HEAD(self) -> None:
            self._handle(head_only=True)

        def do_GET(self) -> None:
            self._handle(head_only=False)

    return _OciHandler


def _probe_existing_registry(port: int, ca_pem: str) -> bool:
    if port <= 0:
        return False
    ctx = ssl.create_default_context(cadata=ca_pem)
    req = urllib.request.Request(f"https://127.0.0.1:{port}/v2/", method="GET")
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=2.0) as resp:
            return int(resp.status) == 200
    except Exception:
        return False


def ensure_registry_server(
    cache_dir: Path | None = None, port: int | None = None
) -> tuple[int, str]:
    """Ensure the background loopback HTTPS OCI registry server is running."""
    c_dir = (cache_dir or default_cache_dir()).resolve()
    req_port = configured_registry_port() if port is None else int(port)
    key = (str(c_dir), req_port)
    with _LOCK:
        if key in _SERVERS:
            _, bound_port, ca_pem = _SERVERS[key]
            return bound_port, ca_pem

        srv_cert, srv_key, ca_pem = ensure_localhost_tls(c_dir)
        blobs_dir = c_dir / "blobs" / "sha256"
        blobs_dir.mkdir(parents=True, exist_ok=True)
        try:
            httpd = _ReusableHTTPServer(("127.0.0.1", req_port), _make_handler(blobs_dir))
        except OSError as exc:
            if exc.errno == errno.EADDRINUSE and _probe_existing_registry(req_port, ca_pem):
                return req_port, ca_pem
            raise RuntimeError(
                f"Failed to bind loopback OCI registry on 127.0.0.1:{req_port} ({exc}); "
                "set CAPSEM_INSPECT_BUILD_REGISTRY_PORT to an available port"
            ) from exc

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=str(srv_cert), keyfile=str(srv_key))
        httpd.socket = ssl_ctx.wrap_socket(httpd.socket, server_side=True)
        bound_port = int(httpd.server_address[1])
        thread = threading.Thread(
            target=httpd.serve_forever, name=f"capsem-oci-reg-{bound_port}", daemon=True
        )
        thread.start()
        _SERVERS[key] = (httpd, bound_port, ca_pem)
        return bound_port, ca_pem

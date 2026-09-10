"""Hermetic TLS distribution registry backed by the pinned Redis rootfs fixture."""

import contextlib
import gzip
import hashlib
import http.server
import json
import ssl
import subprocess
import threading
from pathlib import Path

from tests.fixtures.oci.prepare_redis import native_pin

FIXTURES = Path(__file__).parent
IMAGE = FIXTURES.parents[2] / "cache/target/tests/redis-image"


@contextlib.contextmanager
def registry(directory, *, image_config=None):
    pin = native_pin()
    metadata = json.loads((IMAGE / "redis-image.json").read_text())
    assert all(metadata[key] == value for key, value in pin.items())
    archive = (IMAGE / "redis-rootfs.tar.gz").read_bytes()
    assert hashlib.sha256(archive).hexdigest() == metadata["archive_sha256"]
    diff_id = hashlib.sha256(gzip.decompress(archive)).hexdigest()
    blobs = {}

    def blob(data, kind):
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        blobs[digest] = data
        return {"mediaType": kind, "digest": digest, "size": len(data)}

    config = json.dumps(
        {
            "architecture": pin["platform"].split("/")[1],
            "os": "linux",
            "config": image_config
            or json.loads((FIXTURES / "redis-config.json").read_text()),
            "rootfs": {"type": "layers", "diff_ids": ["sha256:" + diff_id]},
        },
        sort_keys=True,
    ).encode()
    media = "application/vnd.oci.image.manifest.v1+json"
    manifest = json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": media,
            "config": blob(config, "application/vnd.oci.image.config.v1+json"),
            "layers": [blob(archive, "application/vnd.oci.image.layer.v1.tar+gzip")],
        },
        sort_keys=True,
    ).encode()
    digest = "sha256:" + hashlib.sha256(manifest).hexdigest()
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            if self.path == "/v2/":
                body, kind = b"{}", "application/json"
            elif self.path.startswith("/v2/library/redis/manifests/"):
                body, kind = manifest, media
            elif self.path.startswith("/v2/library/redis/blobs/"):
                body, kind = (
                    blobs.get(self.path.rsplit("/", 1)[1]),
                    "application/octet-stream",
                )
            else:
                body, kind = None, "application/json"
            self.send_response(200 if body is not None else 404)
            body = body or b""
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.send_header(
                "Docker-Content-Digest",
                digest
                if kind == media
                else "sha256:" + hashlib.sha256(body).hexdigest(),
            )
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format, *args):
            pass

    certificate, key = directory / "registry.pem", directory / "registry.key"
    leaf, leaf_key, csr = (
        directory / "server.pem",
        directory / "server.key",
        directory / "server.csr",
    )
    config_path = directory / "openssl.cnf"
    config_path.write_text(
        "[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=localhost\n[ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign\n[server]\nsubjectAltName=IP:127.0.0.1,DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n"
    )
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-config",
            str(config_path),
            "-keyout",
            str(key),
            "-out",
            str(certificate),
        ],
        check=True,
        capture_output=True,
        timeout=15,
    )
    subprocess.run(
        [
            "openssl",
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            str(leaf_key),
            "-out",
            str(csr),
        ],
        check=True,
        capture_output=True,
        timeout=15,
    )
    subprocess.run(
        [
            "openssl",
            "x509",
            "-req",
            "-in",
            str(csr),
            "-CA",
            str(certificate),
            "-CAkey",
            str(key),
            "-set_serial",
            "1",
            "-days",
            "1",
            "-extfile",
            str(config_path),
            "-extensions",
            "server",
            "-out",
            str(leaf),
        ],
        check=True,
        capture_output=True,
        timeout=15,
    )
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(leaf, leaf_key)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    reference = f"127.0.0.1:{server.server_port}/library/redis@{digest}"
    try:
        yield reference, certificate, requests
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)
        assert not worker.is_alive()

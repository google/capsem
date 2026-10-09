"""Hermetic loopback TLS OCI v2 workload image fixture built from guest initrd busybox."""

from __future__ import annotations

import contextlib
import gzip
import hashlib
import http.server
import io
import json
import os
import platform
import re
import ssl
import subprocess
import tarfile
import tempfile
import threading
from collections.abc import Iterator
from pathlib import Path
from typing import Any

_APPLETS_RAW = (
    "sh ash sleep cat id head stat mkdir rm rmdir chmod chown tar dd env pwd echo ls cp mv ln "
    "base64 cut date test [ printf true false grep sed awk find wc tr sort uniq sha256sum "
    "touch uname whoami hostname ps kill setsid"
)
_APPLETS = _APPLETS_RAW.split()
_SPLIT_SH = (
    '#!/bin/sh\nshift 5\nsrc="$1"\npfx="${2:-part.}"\n'
    'if [ "$src" = "-" ]; then cat > "${pfx}000000"; else cat "$src" > "${pfx}000000"; fi\n'
    '[ -s "${pfx}000000" ] || rm -f "${pfx}000000"\n'
)
_TIMEOUT_SH = (
    '#!/bin/sh\nshift 2\nsecs="${1%s}"\nshift\nflag="/tmp/.to.$$"\n'
    '/bin/busybox rm -f "$flag"\nexec 3<&0\n'
    '/bin/busybox setsid "$@" <&3 3<&- &\npid=$!\nexec 3<&-\n'
    '/bin/busybox setsid /bin/sh -c "/bin/busybox sleep $secs; : > $flag; '
    '/bin/busybox kill -TERM -$pid 2>/dev/null; /bin/busybox kill -KILL -$pid 2>/dev/null" '
    '</dev/null >/dev/null 2>&1 &\nwpid=$!\nwait "$pid" 2>/dev/null\nrc=$?\n'
    '/bin/busybox kill -KILL -"$wpid" 2>/dev/null || true\n'
    'if [ -e "$flag" ]; then /bin/busybox rm -f "$flag"; exit 124; fi\nexit "$rc"\n'
)
_OPENSSL_CNF = (
    "[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n"
    "[dn]\nCN=Capsem hermetic fixture CA\n"
    "[ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign\n"
    "subjectKeyIdentifier=hash\n"
    "[server]\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n"
    "basicConstraints=critical,CA:FALSE\n"
    "keyUsage=critical,digitalSignature,keyEncipherment\n"
    "extendedKeyUsage=serverAuth\nsubjectKeyIdentifier=hash\n"
    "authorityKeyIdentifier=keyid:always\n"
)


def _find_initrd_path() -> Path:
    arch = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "x86_64"
    env_roots = [
        Path(v)
        for k in ("CAPSEM_ASSETS_DIR", "CAPSEM_WINTERFELL_ASSETS_DIR")
        if (v := os.environ.get(k, "").strip())
    ]
    parents = Path(__file__).resolve().parents
    roots = [
        *env_roots,
        *([parents[3] / "cache/target/assets"] if len(parents) > 3 else []),
    ]
    for root in roots:
        for candidate in (root / arch / "initrd.img", root / "initrd.img"):
            if candidate.is_file():
                return candidate
        if root.is_dir() and (matches := [m for m in root.rglob("initrd.img") if m.is_file()]):
            return matches[0]
    raise RuntimeError(f"Could not locate initrd.img in any assets root: {roots}")


def _extract_busybox_from_initrd(initrd_path: Path) -> bytes:
    raw, offset = gzip.decompress(initrd_path.read_bytes()), 0
    while offset + 110 <= len(raw):
        if raw[offset : offset + 6] not in (b"070701", b"070702"):
            raise RuntimeError(f"Unexpected cpio magic at {offset} in {initrd_path}")
        filesize = int(raw[offset + 54 : offset + 62], 16)
        namesize = int(raw[offset + 94 : offset + 102], 16)
        name_start = offset + 110
        name = raw[name_start : name_start + namesize - 1].decode("utf-8", errors="replace")
        data_start = (name_start + namesize + 3) & ~3
        if name == "TRAILER!!!":
            break
        if name.lstrip("./") == "bin/busybox" and filesize > 0:
            return raw[data_start : data_start + filesize]
        offset = (data_start + filesize + 3) & ~3
    raise RuntimeError(f"bin/busybox not found in {initrd_path}")


def _build_rootfs_tar_bytes(busybox: bytes) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tf:
        for mode, dirs in (
            (0o755, ("bin", "usr", "usr/bin", "etc", "var", "workspace")),
            (0o1777, ("tmp", "var/tmp")),
            (0o700, ("root", "var/tmp/sandbox-services")),
        ):
            for d in dirs:
                ti = tarfile.TarInfo(name=d)
                ti.type, ti.mode = tarfile.DIRTYPE, mode
                tf.addfile(ti)

        def _add_file(name: str, data: bytes, mode: int) -> None:
            info = tarfile.TarInfo(name=name)
            info.size, info.mode = len(data), mode
            tf.addfile(info, io.BytesIO(data))

        def _add_symlink(name: str, target: str) -> None:
            info = tarfile.TarInfo(name=name)
            info.type, info.linkname, info.mode = tarfile.SYMTYPE, target, 0o777
            tf.addfile(info)

        _add_file("bin/busybox", busybox.replace(b"\x00timeout\x00", b"\x00timeou_\x00"), 0o755)
        for applet in _APPLETS:
            _add_symlink(f"bin/{applet}", "busybox")
            _add_symlink(f"usr/bin/{applet}", "/bin/busybox")
        for script_name, content in (
            ("bash", '#!/bin/sh\nexec /bin/sh "$@"\n'),
            ("split", _SPLIT_SH),
            ("timeout", _TIMEOUT_SH),
        ):
            _add_file(f"bin/{script_name}", content.encode("utf-8"), 0o755)
            _add_symlink(f"usr/bin/{script_name}", f"/bin/{script_name}")
        _add_file("var/tmp/sandbox-services/inspect-sandbox-tools", b"#!/bin/sh\nexit 0\n", 0o700)
        _add_file("etc/capsem-workload-fixture", b"capsem-hermetic-oci-fixture\n", 0o644)
        _add_file("etc/passwd", b"root:x:0:0:root:/root:/bin/sh\n", 0o644)
        _add_file("etc/group", b"root:x:0:\n", 0o644)
    return buf.getvalue()


def _grant_in_settings(home: Path, reference: str) -> None:
    repo, _, _ = reference.partition("@")
    path = home / "settings.toml"
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    section = text.split("[images]", 1)[-1] if "[images]" in text else ""

    def _existing(key: str) -> set[str]:
        found = re.search(rf"^{key} = \[(.*)\]$", section, re.MULTILINE)
        return set(re.findall(r'"([^"]+)"', found.group(1))) if found else set()

    s_list = ", ".join(f'"{x}"' for x in sorted(_existing("sources") | {repo}))
    a_list = ", ".join(f'"{x}"' for x in sorted(_existing("admit") | {reference}))
    cleaned = re.sub(r"\[images\]\n(?:[^\[\n][^\n]*\n)*", "", text)
    sep = "\n" if cleaned and not cleaned.endswith("\n") else ""
    path.write_text(
        f"{cleaned}{sep}[images]\nsources = [{s_list}]\nadmit = [{a_list}]\n", encoding="utf-8"
    )


@contextlib.contextmanager
def hermetic_oci_workload_image(home_dir: Path) -> Iterator[tuple[str, str]]:
    """Serve a hermetic busybox OCI image over loopback TLS and admit its digest in `home_dir`."""
    raw_tar = _build_rootfs_tar_bytes(_extract_busybox_from_initrd(_find_initrd_path()))
    layer_gz = gzip.compress(raw_tar, mtime=0)
    diff_id = "sha256:" + hashlib.sha256(raw_tar).hexdigest()
    blobs: dict[str, bytes] = {}

    def _blob(data: bytes, media_type: str) -> dict[str, Any]:
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        blobs[digest] = data
        return {"mediaType": media_type, "digest": digest, "size": len(data)}

    oci_arch = "arm64" if platform.machine().lower() in ("aarch64", "arm64") else "amd64"
    cfg_obj = {
        "architecture": oci_arch,
        "os": "linux",
        "config": {
            "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
            "Cmd": ["/bin/sh", "-c", "sleep 3600"],
            "WorkingDir": "/workspace",
        },
        "rootfs": {"type": "layers", "diff_ids": [diff_id]},
    }
    cfg_bytes = json.dumps(cfg_obj, sort_keys=True).encode("utf-8")
    manifest_media = "application/vnd.oci.image.manifest.v1+json"
    manifest_obj = {
        "schemaVersion": 2,
        "mediaType": manifest_media,
        "config": _blob(cfg_bytes, "application/vnd.oci.image.config.v1+json"),
        "layers": [_blob(layer_gz, "application/vnd.oci.image.layer.v1.tar+gzip")],
    }
    manifest_bytes = json.dumps(manifest_obj, sort_keys=True).encode("utf-8")
    manifest_digest = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()

    class _Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:
            return

        def do_HEAD(self) -> None:
            self._serve(send_body=False)

        def do_GET(self) -> None:
            self._serve(send_body=True)

        def _serve(self, *, send_body: bool) -> None:
            path = self.path.split("?", 1)[0]
            if path in ("/v2", "/v2/"):
                body, kind = b"{}", "application/json"
            elif path.startswith("/v2/library/workload-fixture/manifests/"):
                body, kind = manifest_bytes, manifest_media
            elif path.startswith("/v2/library/workload-fixture/blobs/"):
                body, kind = blobs.get(path.rsplit("/", 1)[1]), "application/octet-stream"
            else:
                body, kind = None, "application/json"
            self.send_response(200 if body is not None else 404)
            payload = body or b""
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header(
                "Docker-Content-Digest", "sha256:" + hashlib.sha256(payload).hexdigest()
            )
            self.end_headers()
            if send_body:
                self.wfile.write(payload)

    settings_path = home_dir / "settings.toml"
    prev_settings = settings_path.read_text(encoding="utf-8") if settings_path.exists() else None
    with tempfile.TemporaryDirectory(prefix="capsem-oci-fixture-") as tmp:
        t = Path(tmp)
        cnf, ca_pem, ca_key = t / "openssl.cnf", t / "ca.pem", t / "ca.key"
        leaf_pem, leaf_key, csr = t / "leaf.pem", t / "leaf.key", t / "leaf.csr"
        cnf.write_text(_OPENSSL_CNF, encoding="utf-8")
        for cmd in (
            f"openssl req -x509 -newkey rsa:2048 -nodes -days 1 -config {cnf} -keyout {ca_key} -out {ca_pem}",
            f"openssl req -new -newkey rsa:2048 -nodes -subj /CN=localhost -keyout {leaf_key} -out {csr}",
            f"openssl x509 -req -in {csr} -CA {ca_pem} -CAkey {ca_key} -set_serial 1 -days 1 -extfile {cnf} -extensions server -out {leaf_pem}",
        ):
            subprocess.run(cmd.split(), check=True, capture_output=True, timeout=15)

        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(leaf_pem, leaf_key)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        server.socket = ctx.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        ref = f"127.0.0.1:{server.server_port}/library/workload-fixture@{manifest_digest}"
        try:
            home_dir.mkdir(parents=True, exist_ok=True)
            _grant_in_settings(home_dir, ref)
            yield f"docker://{ref}", ca_pem.read_text(encoding="utf-8")
        finally:
            if prev_settings is None:
                settings_path.unlink(missing_ok=True)
            else:
                settings_path.write_text(prev_settings, encoding="utf-8")
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

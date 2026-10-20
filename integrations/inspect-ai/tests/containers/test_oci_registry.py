"""Unit tests for `oci_ingest` and `oci_registry`."""

from __future__ import annotations

import concurrent.futures
import hashlib
import io
import json
import ssl
import subprocess
import tarfile
import threading
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

import pytest
from inspect_capsem.containers import oci_ingest as ing_mod
from inspect_capsem.containers import oci_registry as reg_mod

_MANIFEST_MT = "application/vnd.oci.image.manifest.v1+json"
_DOCKER_MT = "application/vnd.docker.distribution.manifest.v2+json"
_INDEX_MT = "application/vnd.oci.image.index.v1+json"


def _make_oci_tar(
    tar_path: Path,
    *,
    nested_index: bool = False,
    legacy_only: bool = False,
    docker_v2: bool = False,
) -> str:
    def _desc(mt: str, data: bytes) -> tuple[str, dict[str, Any]]:
        h = hashlib.sha256(data).hexdigest()
        return h, {"mediaType": mt, "digest": f"sha256:{h}", "size": len(data)}

    cfg_b, layer_b = b'{"architecture":"amd64","os":"linux"}', b"layer-tar-content"
    cfg_hex, cfg_d = _desc("application/vnd.oci.image.config.v1+json", cfg_b)
    layer_mt = (
        "application/vnd.docker.image.rootfs.diff.tar.gzip"
        if docker_v2
        else "application/vnd.oci.image.layer.v1.tar"
    )
    layer_hex, layer_d = _desc(layer_mt, layer_b)
    m_mt = _DOCKER_MT if docker_v2 else _MANIFEST_MT
    m_doc = {"schemaVersion": 2, "mediaType": m_mt, "config": cfg_d, "layers": [layer_d]}
    m_bytes = json.dumps(m_doc, separators=(",", ":")).encode()
    m_hex, m_d = _desc(m_mt, m_bytes)

    def _add(tf: tarfile.TarFile, name: str, data: bytes) -> None:
        info = tarfile.TarInfo(name=name)
        info.size = len(data)
        tf.addfile(info, io.BytesIO(data))

    with tarfile.open(tar_path, "w") as tf:
        if legacy_only:
            _add(tf, f"{cfg_hex}.json", cfg_b)
            _add(tf, "layer0/layer.tar.gz", layer_b)
            leg = [{"Config": f"{cfg_hex}.json", "Layers": ["layer0/layer.tar.gz"]}]
            _add(tf, "manifest.json", json.dumps(leg).encode())
        else:
            for h, b in ((cfg_hex, cfg_b), (layer_hex, layer_b), (m_hex, m_bytes)):
                _add(tf, f"blobs/sha256/{h}", b)
            top_d = m_d
            if nested_index:
                att = {**m_d, "annotations": {"vnd.docker.reference.type": "attestation-manifest"}}
                sub_b = json.dumps(
                    {"schemaVersion": 2, "mediaType": _INDEX_MT, "manifests": [att, m_d]}
                ).encode()
                sub_hex, top_d = _desc(_INDEX_MT, sub_b)
                _add(tf, f"blobs/sha256/{sub_hex}", sub_b)
            _add(tf, "index.json", json.dumps({"schemaVersion": 2, "manifests": [top_d]}).encode())
    return m_hex


def test_oci_ingest_and_registry_https_serving(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("CAPSEM_INSPECT_BUILD_CACHE_DIR", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_BUILD_REGISTRY_PORT", raising=False)
    assert ing_mod.default_cache_dir().name == "inspect-oci-builds"
    assert reg_mod.configured_registry_port() == reg_mod.DEFAULT_REGISTRY_PORT

    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_CACHE_DIR", str(tmp_path / "cache with space"))
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_REGISTRY_PORT", "0")
    assert ing_mod.default_cache_dir() == (tmp_path / "cache with space").resolve()
    assert reg_mod.configured_registry_port() == 0

    real_which = reg_mod.shutil.which
    monkeypatch.setattr(reg_mod.shutil, "which", lambda _: None)
    with pytest.raises(RuntimeError, match="openssl is required"):
        reg_mod.ensure_localhost_tls(tmp_path / "no_openssl")
    monkeypatch.setattr(reg_mod.shutil, "which", real_which)

    flock_calls: list[int] = []
    real_flock = reg_mod.fcntl.flock
    monkeypatch.setattr(
        reg_mod.fcntl,
        "flock",
        lambda fd, op: (flock_calls.append(op), real_flock(fd, op))[1],
    )
    cert_p, key_p, pem = reg_mod.ensure_localhost_tls(ing_mod.default_cache_dir())
    assert "BEGIN CERTIFICATE" in pem and cert_p.is_file() and key_p.is_file()
    assert (cert_p.parent / ".tls.lock").is_file()
    assert flock_calls == [reg_mod.fcntl.LOCK_EX]
    assert reg_mod.ensure_localhost_tls(ing_mod.default_cache_dir())[2] == pem
    assert flock_calls == [reg_mod.fcntl.LOCK_EX, reg_mod.fcntl.LOCK_EX]
    monkeypatch.setattr(reg_mod.fcntl, "flock", real_flock)
    for crt in (cert_p.parent / "ca.pem", cert_p):
        txt = subprocess.check_output(
            ["openssl", "x509", "-in", str(crt), "-noout", "-text"], text=True
        )
        assert "X509v3 Subject Key Identifier" in txt and "X509v3 Authority Key Identifier" in txt

    tar1, tar_v2, tar_legacy = tmp_path / "oci1.tar", tmp_path / "v2.tar", tmp_path / "leg.tar"
    m_hex = _make_oci_tar(tar1, nested_index=True)
    _make_oci_tar(tar_v2, docker_v2=True)
    _make_oci_tar(tar_legacy, legacy_only=True)
    barrier = threading.Barrier(4)

    def _race_worker() -> tuple[str, str]:
        barrier.wait(timeout=5.0)
        ing_hex = ing_mod.ingest_docker_save_tar(tar1)
        _, _, tls_pem = reg_mod.ensure_localhost_tls(tmp_path / "concurrent_tls")
        return ing_hex, tls_pem

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futs = [pool.submit(_race_worker) for _ in range(4)]
        race_results = [f.result(timeout=15.0) for f in futs]
    assert {r[0] for r in race_results} == {m_hex}
    assert len({r[1] for r in race_results}) == 1 and "BEGIN CERTIFICATE" in race_results[0][1]
    assert ing_mod.ingest_docker_save_tar(tar1) == m_hex
    assert len(ing_mod.ingest_docker_save_tar(tar_v2)) == 64
    assert len(ing_mod.ingest_docker_save_tar(tar_legacy)) == 64

    blobs_dir = ing_mod.default_cache_dir() / "blobs" / "sha256"
    assert ing_mod._resolve_tar_member(f"blobs/sha256/{m_hex}", {}, blobs_dir)[0] == m_hex
    assert ing_mod.verify_cached_manifest_blobs(blobs_dir, m_hex)
    assert not ing_mod._blob_valid(blobs_dir, "short")

    empty_tar = tmp_path / "empty.tar"
    with tarfile.open(empty_tar, "w"):
        pass
    with pytest.raises(RuntimeError, match="Unsupported docker save archive"):
        ing_mod.ingest_docker_save_tar(empty_tar)
    with pytest.raises(RuntimeError, match="Missing referenced archive member"):
        ing_mod._resolve_tar_member("missing.tar", {}, tmp_path)

    port, ca_pem = reg_mod.ensure_registry_server(port=0)
    assert port > 0 and ca_pem == pem and reg_mod.ensure_registry_server(port=0) == (port, ca_pem)
    assert reg_mod._probe_existing_registry(port, ca_pem) is True
    assert not reg_mod._probe_existing_registry(0, ca_pem)
    assert not reg_mod._probe_existing_registry(1, ca_pem)
    assert reg_mod.ensure_registry_server(port=port) == (port, ca_pem)
    reg_mod._SERVERS.pop((str(ing_mod.default_cache_dir()), port), None)
    assert reg_mod.ensure_registry_server(port=port) == (port, ca_pem)

    def _raise_eaddrinuse(*_a: Any, **_kw: Any) -> Any:
        raise OSError(98, "Address already in use")

    monkeypatch.setattr(reg_mod, "_ReusableHTTPServer", _raise_eaddrinuse)
    with pytest.raises(RuntimeError, match="CAPSEM_INSPECT_BUILD_REGISTRY_PORT"):
        reg_mod.ensure_registry_server(port=1)

    ctx = ssl.create_default_context(cadata=ca_pem)
    ctx.verify_flags |= getattr(ssl, "VERIFY_X509_STRICT", 0)
    for method in ("GET", "HEAD"):
        for path in ("/v2/", f"/v2/inspect-capsem/build/manifests/sha256:{m_hex}"):
            req = urllib.request.Request(f"https://127.0.0.1:{port}{path}", method=method)
            with urllib.request.urlopen(req, context=ctx) as resp:
                assert resp.status == 200
                if method == "GET" and "manifests" in path:
                    cfg_digest = json.loads(resp.read().decode())["config"]["digest"]
        req_b = urllib.request.Request(
            f"https://127.0.0.1:{port}/v2/inspect-capsem/build/blobs/{cfg_digest}", method=method
        )
        with urllib.request.urlopen(req_b, context=ctx) as resp:
            assert resp.status == 200

    for bad_path in (
        "/v2/unknown",
        "/v2/inspect-capsem/build/manifests/latest",
        f"/v2/inspect-capsem/build/blobs/sha256:{'0' * 64}",
    ):
        with pytest.raises(urllib.error.HTTPError) as exc_info:
            urllib.request.urlopen(f"https://127.0.0.1:{port}{bad_path}", context=ctx)
        exc_info.value.close()
        assert exc_info.value.code == 404

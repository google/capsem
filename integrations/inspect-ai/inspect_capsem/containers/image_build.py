"""Host-side Docker image builder and cache orchestrator for `inspect-capsem`."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import shutil
import subprocess
import tempfile
import threading
import uuid
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from .build_context import (
    compute_build_cache_key,
    default_linux_platform,
    iter_context_files,
)
from .build_grant import is_host_build_enabled
from .oci_ingest import (
    default_cache_dir,
    ingest_docker_save_tar,
    verify_cached_manifest_blobs,
)
from .oci_registry import ensure_registry_server

DEFAULT_BUILD_TIMEOUT_SECS = 600.0
_CACHE_KEY_GUARD = threading.Lock()
_CACHE_KEY_LOCKS: dict[str, threading.Lock] = {}


def _lock_for_cache_key(key: str) -> threading.Lock:
    with _CACHE_KEY_GUARD:
        lock = _CACHE_KEY_LOCKS.get(key)
        if lock is None:
            lock = threading.Lock()
            _CACHE_KEY_LOCKS[key] = lock
        return lock


def _read_valid_manifest_hex(key_file: Path, blobs_dir: Path) -> str | None:
    if not key_file.is_file():
        return None
    with contextlib.suppress(Exception):
        cached = json.loads(key_file.read_text(encoding="utf-8"))
        cand = str(cached.get("manifest_hex", ""))
        if verify_cached_manifest_blobs(blobs_dir, cand):
            return cand
    return None


def configured_build_timeout() -> float:
    """Return the timeout in seconds for host `docker build` commands."""
    raw = os.environ.get("CAPSEM_INSPECT_BUILD_TIMEOUT", "").strip()
    return float(raw) if raw else DEFAULT_BUILD_TIMEOUT_SECS


def run_host_build(
    spec: Mapping[str, Any],
    *,
    cache_dir: Path | None = None,
    platform: str | None = None,
) -> str:
    """Run `docker build` + `docker image save`, ingest into `blobs/sha256`, and return manifest hex."""
    if not is_host_build_enabled():
        raise RuntimeError(
            "Host-side image builds are disabled; set CAPSEM_INSPECT_HOST_BUILD=1 in the evaluator environment."
        )
    docker_bin = shutil.which("docker")
    if docker_bin is None:
        raise RuntimeError(
            "Docker CLI ('docker') is required on PATH to build Compose 'build:' / Dockerfile sandboxes"
        )
    c_dir = (cache_dir or default_cache_dir()).resolve()
    eff_platform = (platform or default_linux_platform()).strip()
    ctx_dir = Path(str(spec["context"]))
    df_path = Path(str(spec["dockerfile"]))
    if not df_path.is_file():
        raise FileNotFoundError(f"Dockerfile not found: {df_path}")
    iter_context_files(ctx_dir)

    timeout = configured_build_timeout()
    temp_tag = f"capsem-inspect-build:{uuid.uuid4().hex[:16]}"
    network_mode = str(spec.get("network") or "none").strip()

    with tempfile.TemporaryDirectory(prefix="capsem-oci-build-") as tmp:
        tmp_root = Path(tmp)
        tar_path = tmp_root / "image.tar"
        explicit_cfg = spec.get("docker_config_dir")
        if explicit_cfg:
            cfg_dir = Path(str(explicit_cfg))
        else:
            cfg_dir = tmp_root / "docker-config"
            cfg_dir.mkdir(parents=True, exist_ok=True)
            (cfg_dir / "config.json").write_text('{"auths":{}}', encoding="utf-8")

        cmd = [
            docker_bin,
            "--config",
            str(cfg_dir),
            "build",
            "--platform",
            eff_platform,
            "--network",
            network_mode,
            "-t",
            temp_tag,
            "-f",
            str(df_path),
        ]
        if spec.get("target"):
            cmd.extend(["--target", str(spec["target"])])
        for k, v in sorted((spec.get("args") or {}).items()):
            cmd.extend(["--build-arg", f"{k}={v}"])
        cmd.append(str(ctx_dir))

        try:
            proc = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )
            if proc.returncode != 0:
                detail = (proc.stderr or proc.stdout or "").strip()[-2000:]
                raise RuntimeError(
                    f"Host 'docker build' failed (exit {proc.returncode}):\n{detail}"
                )
            save_proc = subprocess.run(
                [
                    docker_bin,
                    "--config",
                    str(cfg_dir),
                    "image",
                    "save",
                    "-o",
                    str(tar_path),
                    temp_tag,
                ],
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )
            if save_proc.returncode != 0:
                detail = (save_proc.stderr or save_proc.stdout or "").strip()[-2000:]
                raise RuntimeError(
                    f"Host 'docker image save' failed (exit {save_proc.returncode}):\n{detail}"
                )
            return ingest_docker_save_tar(tar_path, cache_dir=c_dir)
        except subprocess.TimeoutExpired as exc:
            raise RuntimeError(
                f"Host docker build timed out after {timeout}s "
                "(adjust CAPSEM_INSPECT_BUILD_TIMEOUT)"
            ) from exc
        finally:
            with contextlib.suppress(Exception):
                subprocess.run(
                    [docker_bin, "--config", str(cfg_dir), "image", "rm", "-f", temp_tag],
                    capture_output=True,
                    timeout=30,
                    check=False,
                )


def build_and_stage_oci_image_sync(
    spec: Mapping[str, Any],
    *,
    cache_dir: Path | None = None,
    port: int | None = None,
) -> tuple[str, str]:
    """Build or reuse a cached OCI image and return `(loopback_image_ref, ca_pem)`."""
    if not is_host_build_enabled():
        raise RuntimeError(
            "Host-side image builds are disabled; set CAPSEM_INSPECT_HOST_BUILD=1 in the evaluator environment."
        )
    c_dir = (cache_dir or default_cache_dir()).resolve()
    keys_dir = c_dir / "keys"
    blobs_dir = c_dir / "blobs" / "sha256"
    keys_dir.mkdir(parents=True, exist_ok=True)
    blobs_dir.mkdir(parents=True, exist_ok=True)

    eff_platform = default_linux_platform()
    cache_key = compute_build_cache_key(spec, platform=eff_platform)
    key_file = keys_dir / f"{cache_key}.json"
    manifest_hex = _read_valid_manifest_hex(key_file, blobs_dir)
    if manifest_hex is None:
        with _lock_for_cache_key(f"{c_dir}:{cache_key}"):
            manifest_hex = _read_valid_manifest_hex(key_file, blobs_dir)
            if manifest_hex is None:
                manifest_hex = run_host_build(spec, cache_dir=c_dir, platform=eff_platform)
                tmp_key = keys_dir / f".{cache_key}.{os.getpid()}.{uuid.uuid4().hex}.tmp"
                tmp_key.write_text(json.dumps({"manifest_hex": manifest_hex}), encoding="utf-8")
                tmp_key.replace(key_file)

    bound_port, ca_pem = ensure_registry_server(cache_dir=c_dir, port=port)
    return f"127.0.0.1:{bound_port}/inspect-capsem/build@sha256:{manifest_hex}", ca_pem


async def build_and_stage_oci_image(spec: Mapping[str, Any]) -> tuple[str, str]:
    """Async wrapper around `build_and_stage_oci_image_sync`."""
    return await asyncio.to_thread(build_and_stage_oci_image_sync, spec)

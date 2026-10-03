"""Build context tar packing, VM archive staging, and host/VM Docker image caching."""

from __future__ import annotations

import asyncio
import contextlib
import gzip
import hashlib
import io
import logging
import os
import posixpath
import re
import shlex
import tarfile
import uuid
import weakref
from pathlib import Path
from typing import Any

from inspect_capsem.containers.controller import CapsemController, CommandResult

logger = logging.getLogger(__name__)

_DEFAULT_MAX_BUILD_CONTEXT_BYTES = 256 * 1024 * 1024  # 256 MiB build context cap
_MAX_BUILD_CONTEXT_BYTES = _DEFAULT_MAX_BUILD_CONTEXT_BYTES


def _max_build_context_bytes() -> int:
    raw = os.environ.get("INSPECT_CAPSEM_MAX_BUILD_CONTEXT_BYTES")
    if raw is not None:
        try:
            return int(raw.strip())
        except ValueError:
            pass
    return _MAX_BUILD_CONTEXT_BYTES


def _normalize_tarinfo(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    return info


def _clean_dockerignore_pattern(raw_pat: str) -> str:
    cleaned = posixpath.normpath(raw_pat.strip().strip("/")).lstrip("/")
    return "" if cleaned in ("", ".") else cleaned


def _parse_dockerignore(ctx_dir: Path) -> list[tuple[bool, str]]:
    ignore_file = ctx_dir / ".dockerignore"
    if not ignore_file.is_file():
        return []
    rules: list[tuple[bool, str]] = []
    for raw_line in ignore_file.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        negate = line.startswith("!")
        pat = _clean_dockerignore_pattern(line[1:] if negate else line)
        if pat:
            rules.append((negate, pat))
    return rules


def _compile_dockerignore_regex(pat: str) -> re.Pattern[str]:
    out = ["^"]
    i = 0
    n = len(pat)
    while i < n:
        if pat.startswith("**/", i):
            out.append("(?:.+/)?")
            i += 3
        elif pat.startswith("/**", i) and i + 3 == n:
            out.append("(?:/.*)?")
            i += 3
        elif pat.startswith("**", i):
            out.append(".*")
            i += 2
        elif pat[i] == "*":
            out.append("[^/]*")
            i += 1
        elif pat[i] == "?":
            out.append("[^/]")
            i += 1
        elif pat[i] == "[":
            end = pat.find("]", i + 1)
            if end < 0:
                out.append(re.escape("["))
                i += 1
            else:
                inner = pat[i + 1 : end]
                if inner.startswith("!"):
                    inner = "^" + inner[1:]
                out.append(f"[{inner}]")
                i = end + 1
        else:
            out.append(re.escape(pat[i]))
            i += 1
    out.append("$")
    return re.compile("".join(out))


def _matches_dockerignore_pattern(rel_posix: str, pat: str) -> bool:
    cleaned = _clean_dockerignore_pattern(pat)
    if not cleaned:
        return False
    rx = _compile_dockerignore_regex(cleaned)
    parts = posixpath.normpath(rel_posix).lstrip("/").split("/")
    return any(rx.match("/".join(parts[: idx + 1])) is not None for idx in range(len(parts)))


def _is_dockerignored(rel_posix: str, rules: list[tuple[bool, str]]) -> bool:
    ignored = False
    for negate, pat in rules:
        if _matches_dockerignore_pattern(rel_posix, pat):
            ignored = not negate
    return ignored


def _pack_path_tar_gz(
    src: Path,
    *,
    extra_files: dict[str, Path] | None = None,
    ignore_rules: list[tuple[bool, str]] | None = None,
    keep_rel_paths: frozenset[str] = frozenset(),
    normalize: bool = False,
    max_bytes: int | None = None,
    label: str = "Build context",
) -> bytes:
    """Pack `src` (directory contents or single file) into a `.tar.gz` archive."""
    rules = ignore_rules or []
    has_negate = any(neg for neg, _ in rules)
    tar_filter = _normalize_tarinfo if normalize else None
    total_bytes = 0

    def _add_size(num_bytes: int) -> None:
        nonlocal total_bytes
        total_bytes += num_bytes
        if max_bytes is not None and max_bytes > 0 and total_bytes > max_bytes:
            msg = f"{label} at {src} exceeds maximum size of {max_bytes} bytes"
            raise ValueError(msg)

    def _walk_dir(tf: tarfile.TarFile, cur_dir: Path, rel_prefix: str) -> None:
        for item in sorted(cur_dir.iterdir(), key=lambda p: p.name):
            rel_posix = f"{rel_prefix}/{item.name}" if rel_prefix else item.name
            is_kept = rel_posix in keep_rel_paths or any(
                k.startswith(f"{rel_posix}/") for k in keep_rel_paths
            )
            ignored = not is_kept and bool(rules) and _is_dockerignored(rel_posix, rules)
            if item.is_dir() and not item.is_symlink():
                if ignored and not has_negate:
                    continue
                if not ignored:
                    tf.add(item, arcname=rel_posix, recursive=False, filter=tar_filter)
                _walk_dir(tf, item, rel_posix)
            else:
                if ignored:
                    continue
                if item.is_file() and not item.is_symlink():
                    _add_size(item.stat().st_size)
                tf.add(item, arcname=rel_posix, recursive=False, filter=tar_filter)

    buf = io.BytesIO()
    with (
        gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as gz,
        tarfile.open(fileobj=gz, mode="w") as tf,
    ):
        if src.is_dir():
            _walk_dir(tf, src, "")
        elif src.exists():
            if src.is_file() and not src.is_symlink():
                _add_size(src.stat().st_size)
            tf.add(src, arcname=src.name, recursive=False, filter=tar_filter)
        if extra_files:
            for arcname in sorted(extra_files):
                extra_path = extra_files[arcname]
                if extra_path.is_file() and not extra_path.is_symlink():
                    _add_size(extra_path.stat().st_size)
                tf.add(extra_path, arcname=arcname, recursive=False, filter=tar_filter)
    packed = buf.getvalue()
    if max_bytes is not None and max_bytes > 0 and len(packed) > max_bytes:
        msg = f"{label} archive at {src} exceeds maximum size of {max_bytes} bytes"
        raise ValueError(msg)
    return packed


def pack_build_context(ctx_dir: Path, df_path: Path) -> tuple[bytes, str]:
    """Pack `ctx_dir` (and `df_path` if outside `ctx_dir`) into a `.tar.gz` byte archive."""
    ctx_resolved = ctx_dir.resolve()
    df_resolved = df_path.resolve()
    ignore_rules = _parse_dockerignore(ctx_resolved) if ctx_resolved.is_dir() else []
    if df_resolved.is_relative_to(ctx_resolved):
        rel_df = df_resolved.relative_to(ctx_resolved).as_posix()
        extra_files: dict[str, Path] | None = None
        keep_rel = frozenset({rel_df, ".dockerignore"})
    else:
        rel_df = ".capsem.Dockerfile"
        extra_files = {rel_df: df_path}
        keep_rel = frozenset({".dockerignore"})
    return (
        _pack_path_tar_gz(
            ctx_dir,
            extra_files=extra_files,
            ignore_rules=ignore_rules,
            keep_rel_paths=keep_rel,
            normalize=True,
            max_bytes=_max_build_context_bytes(),
        ),
        rel_df,
    )


def _tail_stream_text(text: str, *, max_lines: int = 40, max_chars: int = 4096) -> str:
    """Return the last `max_lines` / `max_chars` of `text` with a truncation prefix when trimmed."""
    cleaned = text.strip()
    if not cleaned:
        return ""
    lines = cleaned.splitlines()
    truncated = False
    if len(lines) > max_lines:
        lines = lines[-max_lines:]
        truncated = True
    joined = "\n".join(lines)
    if len(joined) > max_chars:
        joined = joined[-max_chars:]
        truncated = True
    return f"... [truncated]\n{joined}" if truncated else joined


def _format_exec_failure(res: CommandResult, *, max_lines: int = 40, max_chars: int = 4096) -> str:
    """Format both `stderr` and `stdout` tails for a failed VM/container command.

    Classic `docker build` (`buildkit: false`) emits step output and compiler/apt/cargo errors
    on `stdout` while writing only a short `The command '/bin/sh -c ...' returned a non-zero
    code: N` summary on `stderr`. Using `res.stderr or res.stdout` hides the actual failure on
    `stdout`. Including both streams when both are non-empty preserves the root cause.
    """
    err = _tail_stream_text(res.stderr or "", max_lines=max_lines, max_chars=max_chars)
    out = _tail_stream_text(res.stdout or "", max_lines=max_lines, max_chars=max_chars)
    if err and out and err != out:
        return f"exit_code={res.exit_code}\nstderr:\n{err}\nstdout:\n{out}"
    if err:
        return err
    if out:
        return out
    return f"exit_code={res.exit_code} (no output)"


async def _stage_archive_to_vm_dir(
    controller: CapsemController, vm_id: str, tar_bytes: bytes, guest_dir: str
) -> None:
    guest_dir_q = shlex.quote(guest_dir)
    tar_path = f"{guest_dir}.tar.gz"
    tar_path_q = shlex.quote(tar_path)
    await controller.upload_to_vm(vm_id, tar_path, tar_bytes)
    extract_cmd = (
        f"rm -rf {guest_dir_q} && mkdir -p {guest_dir_q} && "
        f"tar -xzf {tar_path_q} -C {guest_dir_q} && rm -f {tar_path_q}"
    )
    res = await controller.exec_in_vm(vm_id, extract_cmd, timeout=120)
    if res.exit_code != 0:
        msg = f"Failed extracting staged archive in VM {vm_id}: {_format_exec_failure(res)}"
        raise RuntimeError(msg)


def _tar_single_file(name: str, data: bytes) -> bytes:
    buf = io.BytesIO()
    with (
        gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as gz,
        tarfile.open(fileobj=gz, mode="w") as tf,
    ):
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = 0o644
        tf.addfile(info, io.BytesIO(data))
    return buf.getvalue()


_IMAGE_SAVED = "CAPSEM_IMAGE_SAVED"
_IMAGE_TOO_LARGE = "CAPSEM_IMAGE_TOO_LARGE"
_DEFAULT_MAX_CACHE_IMAGE_BYTES = 250 * 1024 * 1024  # 250 MiB uncompressed image cap
_DEFAULT_MAX_CACHE_TOTAL_BYTES = 2 * 1024 * 1024 * 1024  # 2 GiB total cache dir cap
_MAX_VM_EXEC_TIMEOUT = 3600
_IMAGE_BUILD_LOCKS: weakref.WeakKeyDictionary[
    asyncio.AbstractEventLoop, dict[str, asyncio.Lock]
] = weakref.WeakKeyDictionary()
_CORRUPT_ARCHIVE_PATTERNS = (
    "corrupt",
    "not in gzip format",
    "unexpected end of file",
    "unexpected eof",
    "invalid compressed data",
    "invalid tar",
    "archive/tar",
    "short read",
    "damaged",
    "truncated",
    "crc error",
    "length error",
)


def _image_archive_guest_path(cache_file: Path) -> str:
    stem = cache_file.name.removesuffix(".tar.gz").removesuffix(".tgz")
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", stem).strip("-") or "default"
    return f"/root/capsem-image-cache-{slug}.tar.gz"


def _is_corrupt_archive_error(output: str) -> bool:
    lower = output.lower()
    return any(pat in lower for pat in _CORRUPT_ARCHIVE_PATTERNS)


def _is_image_cache_enabled() -> bool:
    val = os.environ.get("INSPECT_CAPSEM_IMAGE_CACHE")
    if val is not None:
        return val.strip().lower() not in ("0", "false", "no", "off")
    return True


def _image_cache_dir() -> Path:
    env_dir = os.environ.get("INSPECT_CAPSEM_IMAGE_CACHE_DIR")
    if env_dir:
        return Path(env_dir)
    return Path.home() / ".cache" / "inspect-capsem" / "images"


def _max_cache_image_bytes() -> int:
    raw = os.environ.get("INSPECT_CAPSEM_MAX_CACHE_IMAGE_BYTES")
    if raw is not None:
        try:
            return int(raw.strip())
        except ValueError:
            pass
    return _DEFAULT_MAX_CACHE_IMAGE_BYTES


def _max_cache_total_bytes() -> int:
    raw = os.environ.get("INSPECT_CAPSEM_MAX_CACHE_TOTAL_BYTES")
    if raw is not None:
        try:
            return int(raw.strip())
        except ValueError:
            pass
    return _DEFAULT_MAX_CACHE_TOTAL_BYTES


def _resolve_build_timeout(cfg_timeout: int | None = None) -> int:
    """Resolve `docker build` timeout in seconds."""
    timeout_val = cfg_timeout if cfg_timeout is not None else _MAX_VM_EXEC_TIMEOUT
    return max(1, min(int(timeout_val), _MAX_VM_EXEC_TIMEOUT))


def _prune_image_cache_dir(
    cache_dir: Path, max_total_bytes: int, *, preserve: Path | None = None
) -> None:
    """Evict oldest `.tar.gz` archives in `cache_dir` until total size is `<= max_total_bytes`."""
    if max_total_bytes <= 0 or not cache_dir.is_dir():
        return
    entries: list[tuple[float, int, Path]] = []
    total_bytes = 0
    for p in cache_dir.glob("*.tar.gz"):
        try:
            st = p.stat()
        except OSError:
            continue
        total_bytes += st.st_size
        entries.append((st.st_mtime, st.st_size, p))
    if total_bytes <= max_total_bytes:
        return
    entries.sort(key=lambda item: item[0])
    for _, sz, p in entries:
        if total_bytes <= max_total_bytes:
            break
        if preserve is not None and p == preserve:
            continue
        with contextlib.suppress(OSError):
            p.unlink(missing_ok=True)
            total_bytes -= sz


def _compute_image_digest(
    tar_bytes: bytes, rel_df: str, df_text: str, *, has_ca: bool, ca_fingerprint: str = ""
) -> str:
    if has_ca:
        ca_marker = f"ca:1:{ca_fingerprint}".encode() if ca_fingerprint else b"ca:1"
    else:
        ca_marker = b"ca:0"
    return hashlib.sha256(
        tar_bytes
        + b"\0"
        + rel_df.encode("utf-8")
        + b"\0"
        + ca_marker
        + b"\0"
        + df_text.encode("utf-8")
    ).hexdigest()[:12]


def _get_image_build_lock(digest: str) -> asyncio.Lock:
    loop = asyncio.get_running_loop()
    loop_locks = _IMAGE_BUILD_LOCKS.get(loop)
    if loop_locks is None:
        loop_locks = {}
        _IMAGE_BUILD_LOCKS[loop] = loop_locks
    lock = loop_locks.get(digest)
    if lock is None:
        lock = asyncio.Lock()
        loop_locks[digest] = lock
    return lock


async def _try_load_cached_image(controller: Any, vm_id: str, tag: str, cache_file: Path) -> bool:
    guest_archive = _image_archive_guest_path(cache_file)
    guest_archive_q = shlex.quote(guest_archive)
    try:
        data = cache_file.read_bytes()
        await controller.upload_to_vm(vm_id, guest_archive, data)
        load_cmd = (
            f"(docker image inspect {shlex.quote(tag)} >/dev/null 2>&1 || "
            f"gunzip -c {guest_archive_q} | docker load >/dev/null); "
            f"rc=$?; rm -f {guest_archive_q}; exit $rc"
        )
        res = await controller.exec_in_vm(vm_id, load_cmd, timeout=300)
    except Exception:
        logger.debug("Failed loading cached image %s into VM %s", cache_file, vm_id, exc_info=True)
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -f {guest_archive_q}", timeout=30)
        return False
    if res.exit_code == 0:
        with contextlib.suppress(OSError):
            cache_file.touch()
        return True
    combined_err = f"{res.stderr or ''}\n{res.stdout or ''}"
    if _is_corrupt_archive_error(combined_err):
        cache_file.unlink(missing_ok=True)
    return False


async def _save_built_image_to_cache(
    controller: Any, vm_id: str, tag: str, cache_file: Path
) -> None:
    max_img_bytes = _max_cache_image_bytes()
    max_total_bytes = _max_cache_total_bytes()
    if max_img_bytes <= 0 or max_total_bytes <= 0:
        return
    guest_archive = _image_archive_guest_path(cache_file)
    guest_archive_q = shlex.quote(guest_archive)
    tmp_file: Path | None = None
    try:
        save_cmd = (
            f"img_size=$(docker image inspect -f '{{{{.Size}}}}' {shlex.quote(tag)} 2>/dev/null || echo 0); "
            f'if [ "${{img_size:-0}}" -gt {max_img_bytes} ] 2>/dev/null; then '
            f'echo "{_IMAGE_TOO_LARGE}:$img_size"; exit 0; fi; '
            f"docker save {shlex.quote(tag)} | gzip -1 > {guest_archive_q} && "
            f"echo {_IMAGE_SAVED}"
        )
        save_res = await controller.exec_in_vm(vm_id, save_cmd, timeout=300)
        if _IMAGE_TOO_LARGE in (save_res.stdout or ""):
            logger.debug(
                "Skipping host image cache for %s in VM %s (exceeds %d-byte cap: %s)",
                tag,
                vm_id,
                max_img_bytes,
                save_res.stdout.strip(),
            )
            return
        if save_res.exit_code != 0 or _IMAGE_SAVED not in save_res.stdout:
            return
        archive_bytes = await controller.download_from_vm(vm_id, guest_archive)
        if archive_bytes and len(archive_bytes) <= min(max_img_bytes, max_total_bytes):
            cache_file.parent.mkdir(parents=True, exist_ok=True)
            tmp_file = cache_file.with_name(f".{cache_file.name}.{uuid.uuid4().hex[:8]}.tmp")
            tmp_file.write_bytes(archive_bytes)
            tmp_file.replace(cache_file)
            _prune_image_cache_dir(cache_file.parent, max_total_bytes, preserve=cache_file)
    except Exception:
        logger.debug("Failed saving built image %s from VM %s", tag, vm_id, exc_info=True)
        if tmp_file is not None:
            tmp_file.unlink(missing_ok=True)
    finally:
        with contextlib.suppress(Exception):
            await controller.exec_in_vm(vm_id, f"rm -f {guest_archive_q}", timeout=30)

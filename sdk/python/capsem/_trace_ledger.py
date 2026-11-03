"""Capsem `rows.ndjson.gz` (`capsem.rows.v1`) stream parser and table helpers."""

from __future__ import annotations

import contextlib
import datetime as dt
import gzip
import io
import json
import logging
import os
import tempfile
from collections.abc import Iterator, Mapping, Sequence
from pathlib import Path
from typing import Any

from ._transport import HttpError

logger = logging.getLogger(__name__)
EXPORT_PERSIST_PREFIX = "export-"
ROWS_STREAM_FORMAT = "capsem.rows.v1"

HARBOR_EVENT_TABLES: tuple[str, ...] = (
    "exec_events", "fs_events", "net_events", "dns_events", "audit_events", "substitution_events",
)
DROPPED_SECURITY_TABLES: tuple[str, ...] = (
    "security_rule_events", "security_decision_events", "security_ask_events",
)
DEFAULT_EXPORT_TABLES: tuple[str, ...] = (
    "model_calls", "model_items", "tool_calls", "tool_responses", "event_body_blobs",
    "exec_events", "audit_events", "net_events", "dns_events", "fs_events",
    "substitution_events", "security_ask_events", "policy_mutation_events",
    "transport_events", "security_rule_runs", "security_decision_runs",
)
ALL_EXPORT_TABLES: tuple[str, ...] = (
    *DEFAULT_EXPORT_TABLES, "security_rule_events", "security_decision_events",
)


def parse_iso_datetime(ts: Any) -> dt.datetime | None:
    """Parse an ISO-8601 timestamp string or `datetime` into an aware UTC `datetime`."""
    if ts is None:
        return None
    if isinstance(ts, dt.datetime):
        return ts.replace(tzinfo=dt.UTC) if ts.tzinfo is None else ts.astimezone(dt.UTC)
    s = str(ts).strip()
    if not s:
        return None
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    try:
        parsed = dt.datetime.fromisoformat(s)
    except ValueError:
        return None
    return parsed.replace(tzinfo=dt.UTC) if parsed.tzinfo is None else parsed.astimezone(dt.UTC)


def resolve_include_security_events(
    include_security_events: bool | None = None,
) -> bool:
    """Resolve `include_security_events` from flag or `CAPSEM_TRACE_INCLUDE_SECURITY`."""
    if include_security_events is not None:
        return bool(include_security_events)
    raw = os.environ.get("CAPSEM_TRACE_INCLUDE_SECURITY", "").strip().lower()
    return raw in ("1", "true", "yes", "on")


def atomic_write_bytes(dest: Path, data: bytes) -> None:
    """Atomically write `data` to `dest` via a sibling temporary file and `os.replace`."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(dir=dest.parent, prefix=f".{dest.name}.", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        os.replace(tmp_name, dest)
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp_name)
        raise


@contextlib.contextmanager
def _iter_rows_lines(source: str | Path | bytes) -> Iterator[Iterator[str]]:
    if isinstance(source, (str, Path)):
        with Path(source).open("rb") as f:
            magic = f.read(2)
            f.seek(0)
            stream: io.BufferedIOBase = gzip.GzipFile(fileobj=f) if magic == b"\x1f\x8b" else f
            with stream, io.TextIOWrapper(stream, encoding="utf-8") as txt:
                yield (ln.rstrip("\r\n") for ln in txt if ln.strip())
    else:
        raw = bytes(source)
        bstream: io.BufferedIOBase = (
            gzip.GzipFile(fileobj=io.BytesIO(raw)) if raw.startswith(b"\x1f\x8b") else io.BytesIO(raw)
        )
        with bstream, io.TextIOWrapper(bstream, encoding="utf-8") as txt:
            yield (ln.rstrip("\r\n") for ln in txt if ln.strip())


def parse_rows_ndjson_gz(
    source: str | Path | bytes,
    *,
    tables: Sequence[str] | None = None,
) -> tuple[dict[str, Any], dict[str, list[dict[str, Any]]]]:
    """Parse a `capsem.rows.v1` `.rows.ndjson.gz` file or byte stream into `(header, tables)`."""
    with _iter_rows_lines(source) as line_iter:
        first_line = next(line_iter, None)
        if first_line is None:
            raise ValueError(f"Empty {ROWS_STREAM_FORMAT} stream: missing header line")
        header = json.loads(first_line)
        if (
            not isinstance(header, dict)
            or header.get("type") != "header"
            or header.get("format") != ROWS_STREAM_FORMAT
        ):
            raise ValueError(f"Expected {ROWS_STREAM_FORMAT} header on line 1, got {first_line!r}")

        filter_set = set(tables) if tables is not None else None
        loaded: dict[str, list[dict[str, Any]]] = {}
        if tables is not None:
            loaded = {t: [] for t in tables}
        elif isinstance(hdr_tables := header.get("tables"), list):
            loaded = {str(t): [] for t in hdr_tables if isinstance(t, str)}

        for idx, line in enumerate(line_iter, start=2):
            rec = json.loads(line)
            tbl = rec.get("table") if isinstance(rec, dict) else None
            row = rec.get("row") if isinstance(rec, dict) else None
            if not (isinstance(tbl, str) and tbl and isinstance(row, dict)):
                raise ValueError(f"Invalid {ROWS_STREAM_FORMAT} row on line {idx}: {line!r}")
            if filter_set is None or tbl in filter_set:
                loaded.setdefault(tbl, []).append(dict(row))
    return header, loaded


def load_rows_tables(
    source: str | Path | bytes | Mapping[str, Sequence[Mapping[str, Any]]],
    tables: Sequence[str] | None = None,
) -> dict[str, list[dict[str, Any]]]:
    """Load table row dicts from a `.rows.ndjson.gz` file, bytes, or pre-parsed mapping."""
    if isinstance(source, Mapping):
        eff = tuple(tables) if tables is not None else tuple(source.keys())
        return {t: [dict(r) for r in source.get(t, ())] for t in eff}
    if isinstance(source, (str, Path)) and not Path(source).is_file():
        return {t: [] for t in (tables or ())}
    _, loaded = parse_rows_ndjson_gz(source, tables=tables)
    return loaded


def has_session_events(
    source: str | Path | bytes | Mapping[str, Sequence[Mapping[str, Any]]],
) -> bool:
    """Return True if `source` contains at least one `model_calls` or `exec_events` row."""
    tbls = load_rows_tables(source, ("model_calls", "exec_events"))
    return bool(tbls.get("model_calls") or tbls.get("exec_events"))


def resolve_warc_path(rows_path: str | Path, warc_path: str | Path | None = None) -> Path | None:
    """Resolve the sibling `<vm_id>.warc.gz` path for `rows_path` when `warc_path` is omitted."""
    if warc_path is not None:
        wp = Path(warc_path)
        return wp if wp.is_file() else None
    rp = Path(rows_path)
    for suffix in (".rows.ndjson.gz", ".events.ndjson.gz", ".ndjson.gz", ".ndjson"):
        if rp.name.endswith(suffix):
            cand = rp.with_name(f"{rp.name.removesuffix(suffix)}.warc.gz")
            if cand.is_file():
                return cand
    fallback = rp.parent / "export.warc.gz"
    return fallback if fallback.is_file() else None


def write_export_meta(
    out_dir: str | Path, vm_id: str, *, rows_path: Path, warc_path: Path,
    include_security_events: bool = False, labels: Mapping[str, str] | None = None,
) -> Path:
    """Write `<out_dir>/<vm_id>.meta.json` describing an exported `(rows, warc)` pair."""
    dest = Path(out_dir) / f"{vm_id}.meta.json"
    meta = {
        "vm_id": vm_id, "rows_ndjson_gz": rows_path.name, "warc_gz": warc_path.name,
        "dropped_security_tables": not include_security_events,
        "labels": dict(labels) if labels is not None else {},
    }
    atomic_write_bytes(dest, (json.dumps(meta, indent=2) + "\n").encode("utf-8"))
    return dest


def is_export_persisted_vm(vm_info: Any) -> bool:
    """Return whether `vm_info` is a temporary persistent VM created by `quiesce()`."""
    if not bool(getattr(vm_info, "persistent", False)):
        return False
    name = str(getattr(vm_info, "name", "") or "")
    return name.startswith(EXPORT_PERSIST_PREFIX)


async def quiesce_vm_for_export(session: Any, vm_id: str | None = None) -> bool:
    """Persist (if ephemeral and unmanaged) and stop `session` so `#346` drains the ledger."""
    if session is None:
        return False
    is_managed = bool(getattr(session, "_managed", False))
    vid = str(vm_id or getattr(session, "id", "") or getattr(session, "_id", "") or "")
    already_persistent = False
    info_fn = getattr(session, "info", None)
    if callable(info_fn):
        try:
            info = await info_fn()
            raw_status = getattr(info, "status", None)
            status_str = str(getattr(raw_status, "value", raw_status) or "").lower()
            if status_str == "stopped":
                return True
            already_persistent = bool(getattr(info, "persistent", False))
        except HttpError as exc:
            if exc.status == 404:
                logger.warning("VM %s not found before export drain barrier: %s", vid, exc)
                return False
            logger.warning("Failed to query VM %s info before export drain: %s", vid, exc)
        except (OSError, RuntimeError, ValueError, TypeError) as exc:
            logger.warning("Failed to query VM %s info before export drain: %s", vid, exc)

    persist_fn = getattr(session, "persist", None)
    stop_fn = getattr(session, "stop", None)
    if not callable(stop_fn):
        return False
    if not already_persistent and not is_managed and callable(persist_fn):
        slug = "".join(c for c in vid.lower() if c.isalnum() or c == "-").strip("-")[:48] or "vm"
        persisted_ok = False
        try:
            await persist_fn(f"{EXPORT_PERSIST_PREFIX}{slug}")
            persisted_ok = True
        except HttpError as exc:
            msg = str(exc).lower()
            if exc.status == 400 and "already persistent" in msg:
                already_persistent = True
            elif exc.status in (400, 409) and any(
                k in msg for k in ("already exists", "already taken", "duplicate", "in use")
            ):
                suffix = os.urandom(3).hex()
                try:
                    await persist_fn(f"{EXPORT_PERSIST_PREFIX}{slug[:40]}-{suffix}")
                    persisted_ok = True
                except (HttpError, OSError, RuntimeError, ValueError, TypeError) as retry_exc:
                    logger.warning("Failed to persist VM %s after name collision: %s", vid, retry_exc)
                    return False
            else:
                logger.warning("Failed to persist VM %s for export drain barrier: %s", vid, exc)
                return False
        except (OSError, RuntimeError, ValueError, TypeError) as exc:
            logger.warning("Failed to persist VM %s for export drain barrier: %s", vid, exc)
            return False
        if persisted_ok:
            with contextlib.suppress(AttributeError, TypeError):
                session._export_persisted = True
    try:
        await stop_fn()
    except (HttpError, OSError, RuntimeError, ValueError, TypeError) as exc:
        logger.warning("Failed to stop VM %s for export drain barrier: %s", vid, exc)
        return False
    return True


async def cleanup_export_persisted_vm(session: Any) -> bool:
    """Delete `session` if and only if it is a temporary `export-*` persisted VM."""
    if session is None or bool(getattr(session, "_managed", False)):
        return False
    delete_fn = getattr(session, "delete", None)
    if not callable(delete_fn):
        return False
    should_delete = bool(getattr(session, "_export_persisted", False))
    if not should_delete and callable(info_fn := getattr(session, "info", None)):
        with contextlib.suppress(HttpError, OSError, RuntimeError, ValueError, TypeError):
            should_delete = is_export_persisted_vm(await info_fn())
    if not should_delete:
        return False
    try:
        await delete_fn()
    except (HttpError, OSError, RuntimeError, ValueError, TypeError) as exc:
        logger.warning("Failed to delete export-persisted VM %s: %s", getattr(session, "id", ""), exc)
        return False
    with contextlib.suppress(AttributeError, TypeError):
        session._export_persisted = False
    return True


__all__ = [
    "ALL_EXPORT_TABLES", "DEFAULT_EXPORT_TABLES", "DROPPED_SECURITY_TABLES",
    "EXPORT_PERSIST_PREFIX", "HARBOR_EVENT_TABLES", "ROWS_STREAM_FORMAT",
    "atomic_write_bytes", "cleanup_export_persisted_vm", "has_session_events",
    "is_export_persisted_vm", "load_rows_tables", "parse_iso_datetime",
    "parse_rows_ndjson_gz", "quiesce_vm_for_export", "resolve_include_security_events",
    "resolve_warc_path", "write_export_meta",
]

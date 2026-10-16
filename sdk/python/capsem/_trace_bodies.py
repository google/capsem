"""WARC 1.1 body reader (`export.warc.gz`) and ATIF table hydration."""

from __future__ import annotations

import base64
import contextlib
import gzip
import importlib
import json
import re
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, Literal, Protocol, runtime_checkable

BodiesMode = Literal["none", "previews", "warc", "http", "full"]
_EVENT_ID_RE = re.compile(r"^[0-9a-f]{12}$")
_SECURITY_TABLES = ("security_rule_events", "security_decision_events", "security_ask_events")
_KNOWN_SOURCE_TABLES = frozenset({
    "net_events", "model_calls", "tool_calls", "tool_responses", "exec_events", *_SECURITY_TABLES,
})
_URI_PREFIX_TABLES = (
    ("capsem://tool-response/", "tool_responses"), ("capsem://tool/", "tool_calls"),
    ("capsem://exec/", "exec_events"), ("capsem://security-decision/", "security_decision_events"),
    ("capsem://security-ask/", "security_ask_events"),
    ("capsem://security/", "security_rule_events"),
)


def validate_event_id(event_id: str) -> str:
    """Validate that `event_id` is a 12-character lowercase hexadecimal string."""
    s = str(event_id).strip()
    if not _EVENT_ID_RE.fullmatch(s):
        raise ValueError(f"Invalid Capsem event_id {event_id!r}: expected 12 lowercase hex chars")
    return s


@runtime_checkable
class BodyReader(Protocol):
    """Protocol for synchronous lookup of archived bodies for a given `event_id`."""

    def get_event_bodies(self, event_id: str) -> Sequence[Mapping[str, Any] | Any]: ...


def _parse_warc_record_id(raw_id: str, target_uri: str) -> tuple[str, str, str] | None:
    cleaned = raw_id.strip().strip("<>").strip()
    if not cleaned.startswith("urn:capsem:"):
        return None
    parts = cleaned.removeprefix("urn:capsem:").rsplit(":", 3)
    if len(parts) < 2:
        return None
    event_id, direction = parts[-2], parts[-1]
    if not _EVENT_ID_RE.fullmatch(event_id) or not direction:
        return None
    cand_table = parts[-3] if len(parts) >= 3 else ""
    source_table = cand_table if cand_table in _KNOWN_SOURCE_TABLES else ""
    if not source_table and target_uri:
        for prefix, tbl in _URI_PREFIX_TABLES:
            if target_uri.startswith(prefix):
                source_table = tbl
                break
    return source_table, event_id, direction


def parse_warc_records(source: str | Path | bytes) -> list[dict[str, Any]]:
    """Parse a single- or multi-member `.warc.gz` / `.warc` stream into WARC 1.1 records."""
    raw = Path(source).read_bytes() if isinstance(source, (str, Path)) else bytes(source)
    if not raw:
        return []
    data = gzip.decompress(raw) if raw.startswith(b"\x1f\x8b") else raw
    records: list[dict[str, Any]] = []
    pos, n = 0, len(data)
    while pos < n:
        while pos < n and data[pos : pos + 1] in (b"\r", b"\n"):
            pos += 1
        if pos >= n:
            break
        line_end = data.find(b"\n", pos)
        if line_end < 0:
            raise ValueError("Truncated WARC version line")
        ver_line = data[pos:line_end].rstrip(b"\r").decode("ascii", errors="replace")
        if not ver_line.startswith("WARC/"):
            raise ValueError(f"Expected WARC version line, got {ver_line!r}")
        pos = line_end + 1
        headers: dict[str, str] = {}
        while True:
            if pos >= n:
                raise ValueError("Unexpected EOF in WARC header block")
            hline_end = data.find(b"\n", pos)
            if hline_end < 0:
                raise ValueError("Truncated WARC header line")
            hline = data[pos:hline_end].rstrip(b"\r")
            pos = hline_end + 1
            if not hline:
                break
            if b":" not in hline:
                raise ValueError(f"Malformed WARC header line: {hline!r}")
            k, v = hline.decode("utf-8", errors="replace").split(":", 1)
            headers[k.strip().lower()] = v.strip()
        try:
            content_len = int(headers.get("content-length") or 0)
        except ValueError as exc:
            cl = headers.get("content-length")
            raise ValueError(f"Invalid WARC Content-Length: {cl!r}") from exc
        if content_len < 0 or pos + content_len > n:
            raise ValueError("Truncated WARC record block")
        body = data[pos : pos + content_len]
        pos += content_len
        records.append({"headers": headers, "body": body})
    return records


class WarcBodyReader:
    """In-memory `BodyReader` backed by a Capsem `export.warc.gz` stream or file."""

    def __init__(self, source: str | Path | bytes | None = None) -> None:
        self._by_eid: dict[str, list[dict[str, Any]]] = {}
        if source is None or (isinstance(source, (str, Path)) and not Path(source).is_file()):
            return
        for rec in parse_warc_records(source):
            hdrs, payload = rec["headers"], rec["body"]
            if hdrs.get("warc-type", "").lower() == "warcinfo":
                continue
            rec_id, target_uri = hdrs.get("warc-record-id", ""), hdrs.get("warc-target-uri", "")
            if (parsed := _parse_warc_record_id(rec_id, target_uri)) is None:
                continue
            source_table, event_id, direction = parsed
            try:
                content, encoding = payload.decode("utf-8"), "utf8"
            except UnicodeDecodeError:
                content, encoding = base64.b64encode(payload).decode("ascii"), "base64"
            self._by_eid.setdefault(event_id, []).append({
                "event_id": event_id, "source_table": source_table, "direction": direction,
                "content_type": hdrs.get("content-type") or "application/octet-stream",
                "encoding": encoding, "content": content, "stored_bytes": len(payload),
                "truncated": bool(hdrs.get("warc-truncated")),
                "body_hash": hdrs.get("warc-block-digest") or "",
            })

    def get_event_bodies(self, event_id: str) -> Sequence[dict[str, Any]]:
        return self._by_eid.get(str(event_id).strip(), ())

    def event_ids(self) -> tuple[str, ...]:
        return tuple(self._by_eid.keys())


def _attr(obj: Any, key: str) -> Any:
    return obj.get(key) if isinstance(obj, Mapping) else getattr(obj, key, None)


def _body_utf8_text(body: Mapping[str, Any] | Any) -> str | None:
    enc = str(_attr(body, "encoding") or "utf8").lower()
    content = _attr(body, "content")
    return content if enc in ("utf8", "utf-8") and isinstance(content, str) else None


def _body_raw_bytes(body: Mapping[str, Any] | Any) -> bytes | None:
    enc = str(_attr(body, "encoding") or "").lower()
    content = _attr(body, "content")
    if not isinstance(content, str):
        return None
    if enc == "base64":
        with contextlib.suppress(ValueError):
            return base64.b64decode(content)
    return content.encode("utf-8")


def decode_security_payload(raw_or_envelope: Any) -> Any:
    """Decode a security payload from UTF-8 JSON, msgpack bytes (if installed), or base64."""
    if isinstance(raw_or_envelope, Mapping) and "data" in raw_or_envelope:
        enc = str(raw_or_envelope.get("encoding") or "").lower()
        data = raw_or_envelope.get("data")
        if enc in ("utf8", "utf-8") and isinstance(data, str):
            with contextlib.suppress(ValueError):
                return json.loads(data)
            return data
        return dict(raw_or_envelope)
    raw: bytes | None = raw_or_envelope if isinstance(raw_or_envelope, bytes) else None
    if raw is None and (isinstance(raw_or_envelope, Mapping) or hasattr(raw_or_envelope, "content")):
        if (txt := _body_utf8_text(raw_or_envelope)) is not None:
            with contextlib.suppress(ValueError):
                return json.loads(txt)
            return txt
        raw = _body_raw_bytes(raw_or_envelope)
    if raw is None:
        return None
    with contextlib.suppress(UnicodeDecodeError, ValueError):
        return json.loads(raw.decode("utf-8"))
    with contextlib.suppress(Exception):
        mp = importlib.import_module("msgpack")
        return mp.unpackb(raw, raw=False)
    return {"encoding": "base64", "data": base64.b64encode(raw).decode("ascii")}


def candidate_body_event_ids(
    tables: Mapping[str, Sequence[Mapping[str, Any]]], *, full: bool = False
) -> list[str]:
    """Return deduplicated `event_id`s whose archived bodies should be hydrated into `tables`."""
    blobs_by_eid: dict[str, list[Mapping[str, Any]]] = {}
    for b in tables.get("event_body_blobs", ()):
        if (eid := str(b.get("event_id") or "")) and _EVENT_ID_RE.fullmatch(eid):
            blobs_by_eid.setdefault(eid, []).append(b)
    if not blobs_by_eid:
        return []
    wanted: list[str] = []
    seen: set[str] = set()

    def _add(eid: str) -> None:
        if eid in blobs_by_eid and eid not in seen:
            seen.add(eid)
            wanted.append(eid)

    def _preview_truncated(eid: str, direction: str, preview: Any) -> bool:
        prev_len = len(str(preview or "").encode("utf-8"))
        for b in blobs_by_eid.get(eid, ()):
            if b.get("direction") == direction and (
                b.get("truncated") or int(b.get("stored_bytes") or 0) > prev_len
            ):
                return True
        return False

    for idx, mc in enumerate(tables.get("model_calls", ())):
        eid, req_prev = str(mc.get("event_id") or ""), mc.get("request_body_preview")
        if full or (idx == 0 and _preview_truncated(eid, "request", req_prev)):
            _add(eid)
    for tc in tables.get("tool_calls", ()):
        eid = str(tc.get("event_id") or "")
        if full or _preview_truncated(eid, "request", tc.get("arguments")):
            _add(eid)
    for tr in tables.get("tool_responses", ()):
        eid = str(tr.get("event_id") or "")
        if full or _preview_truncated(eid, "response", tr.get("content_preview")):
            _add(eid)
    if full:
        for tbl in ("exec_events", *_SECURITY_TABLES):
            for row in tables.get(tbl, ()):
                _add(str(row.get("event_id") or ""))
    return wanted


def hydrate_tables_from_bodies(
    tables: Mapping[str, Sequence[dict[str, Any]]], reader: BodyReader, *, full: bool = False
) -> None:
    """Hydrate `tables` in place using `reader` (`WarcBodyReader` or custom `BodyReader`)."""
    cands = candidate_body_event_ids(tables, full=full)
    if not cands and callable(eids_fn := getattr(reader, "event_ids", None)):
        raw_ids = eids_fn()
        cands = [str(x) for x in raw_ids] if isinstance(raw_ids, Sequence) else []
    by_eid: dict[str, dict[tuple[str, str], Any]] = {}
    for eid in cands:
        by_key: dict[tuple[str, str], Any] = {}
        for b in reader.get_event_bodies(eid):
            st, d = str(_attr(b, "source_table") or ""), str(_attr(b, "direction") or "")
            by_key[(st, d)] = b
            if st:
                by_key.setdefault(("", d), b)
        if by_key:
            by_eid[eid] = by_key

    def _get(bmap: Mapping[tuple[str, str], Any], tbl: str, d: str) -> Any:
        return bmap.get((tbl, d)) or bmap.get(("", d))

    for idx, mc in enumerate(tables.get("model_calls", ())):
        if not full and idx > 0:
            break
        if (bmap := by_eid.get(str(mc.get("event_id") or ""))) is None:
            continue
        if (req_b := _get(bmap, "model_calls", "request")) is not None:
            if (txt := _body_utf8_text(req_b)) is not None:
                mc["request_body"] = txt
            if _attr(req_b, "truncated"):
                mc["truncated"] = True
        if full and (resp_b := _get(bmap, "model_calls", "response")) is not None:
            if (txt := _body_utf8_text(resp_b)) is not None:
                mc["response_body"] = txt
            if _attr(resp_b, "truncated"):
                mc["truncated"] = True
    for tc in tables.get("tool_calls", ()):
        if (bmap := by_eid.get(str(tc.get("event_id") or ""))) and (
            b := _get(bmap, "tool_calls", "request")
        ) and (txt := _body_utf8_text(b)):
            tc["arguments"] = txt
    for tr in tables.get("tool_responses", ()):
        if (bmap := by_eid.get(str(tr.get("event_id") or ""))) and (
            b := _get(bmap, "tool_responses", "response")
        ) and (txt := _body_utf8_text(b)):
            tr["content_full"] = txt
    if full:
        for ex in tables.get("exec_events", ()):
            if (bmap := by_eid.get(str(ex.get("event_id") or ""))) is None:
                continue
            for d in ("stdout", "stderr"):
                if (b := _get(bmap, "exec_events", d)) and (txt := _body_utf8_text(b)) is not None:
                    ex[d] = txt
        for st in _SECURITY_TABLES:
            for sr in tables.get(st, ()):
                if (bmap := by_eid.get(str(sr.get("event_id") or ""))) and (
                    b := _get(bmap, st, "payload")
                ) is not None:
                    sr["payload"] = decode_security_payload(b)

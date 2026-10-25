"""Bounded byte-for-byte route validation against logger-owned fixture facts."""
from __future__ import annotations

import base64
import gzip
from functools import lru_cache
from io import BytesIO
from urllib.parse import parse_qs, urlsplit

import msgpack
from helpers.stopped_workspace import BODY_EVENT_ID, VM_ID
from warcio.archiveiterator import ArchiveIterator

from .route_fixtures import ROOT, corpus_config


@lru_cache(maxsize=1)
def archive_facts() -> list[dict]:
    cfg = corpus_config()
    path = ROOT / "tests/fixtures/session/test.facts.msgpack"
    with path.open("rb") as source:
        raw = source.read(cfg["archive_facts_max_bytes"] + 1)
    assert len(raw) <= cfg["archive_facts_max_bytes"], "facts byte budget exceeded"
    records = msgpack.unpackb(raw, raw=False)
    assert isinstance(records, list) and 0 < len(records) <= cfg["archive_max_records"]
    keys = [(row["event_id"], row["source_table"], row["direction"]) for row in records]
    assert len(set(keys)) == len(keys)
    assert all(isinstance(row["body"], bytes) and row["body"] for row in records)
    assert sum(len(row["body"]) for row in records) <= cfg["archive_inflated_max_bytes"]
    return records


@lru_cache(maxsize=2)
def file_bytes(size: int) -> bytes:
    assert size in corpus_config()["file_sizes"] and size % 256 == 0
    return bytes(range(256)) * (size // 256)


def validate_file(path: str, body: bytes, headers) -> None:
    name = parse_qs(urlsplit(path).query)["path"][0]
    size = int(name.removeprefix("route-").removesuffix(".bin"))
    assert name == f"route-{size}.bin" and body == file_bytes(size)
    assert headers["content-length"] == str(size)
    assert headers["content-type"] == "application/octet-stream"
    assert headers["content-disposition"] == f'attachment; filename="{name}"'
    assert headers["x-capsem-vm-path"] == "/workspace/" + name
    assert "content-encoding" not in headers


def _fields(record, budget: int) -> dict[str, str]:
    assert record.rec_type == "warcinfo"
    raw = record.content_stream().read(budget + 1)
    assert len(raw) <= budget
    lines = raw.decode("utf-8").splitlines()
    fields = [line.split(": ", 1) for line in lines if line]
    assert all(len(field) == 2 for field in fields)
    assert len({field[0] for field in fields}) == len(fields)
    return dict(fields)


def validate_archive(body: bytes, headers) -> None:
    cfg = corpus_config()
    assert headers["content-type"] == "application/warc"
    assert headers["content-disposition"] == f'attachment; filename="capsem-session-{VM_ID}.warc.gz"'
    assert "content-encoding" not in headers
    # Consume every gzip member/footer first: an HTTP200 or warcio's last
    # parsed record alone does not prove transport/file completion.
    with gzip.GzipFile(fileobj=BytesIO(body)) as source:
        inflated = source.read(cfg["archive_inflated_max_bytes"] + 1)
    assert len(inflated) <= cfg["archive_inflated_max_bytes"], "WARC inflation budget exceeded"
    expected = {
        f"<urn:capsem:{VM_ID}:{row['source_table']}:{row['event_id']}:{row['direction']}>": row
        for row in archive_facts()
    }
    stream = iter(ArchiveIterator(BytesIO(inflated)))
    opening = next(stream)
    assert opening.rec_headers.protocol == "WARC/1.1"
    assert _fields(opening, cfg["warc_info_max_bytes"])["capsem-session"] == VM_ID
    seen = set()
    closing = None
    for record in stream:
        assert closing is None, "records after completion summary"
        assert record.rec_headers.protocol == "WARC/1.1"
        if record.rec_type == "warcinfo":
            closing = _fields(record, cfg["warc_info_max_bytes"])
            continue
        assert record.rec_type == "resource"
        key = record.rec_headers.get_header("WARC-Record-ID")
        assert key in expected and key not in seen
        fact = expected[key]
        assert record.rec_headers.get_header("WARC-Target-URI")
        assert record.rec_headers.get_header("Content-Type") == (fact["content_type"] or "application/octet-stream")
        assert int(record.rec_headers.get_header("Content-Length")) == len(fact["body"])
        assert record.rec_headers.get_header("WARC-Truncated") == ("length" if fact["truncated"] else None)
        assert record.content_stream().read(len(fact["body"]) + 1) == fact["body"]
        seen.add(key)
    assert seen == set(expected), "missing archive records"
    assert closing is not None and closing["capsem-records"] == str(len(expected))
    assert closing["capsem-skipped"] == "0"
    assert all(value == "0" for key, value in closing.items() if key.startswith("capsem-skipped-"))


def validate_full_event(payload: dict) -> None:
    assert payload["event_id"] == BODY_EVENT_ID
    facts = {(row["source_table"], row["direction"]): row for row in archive_facts() if row["event_id"] == BODY_EVENT_ID}
    assert len(payload["bodies"]) == len(facts) > 0
    seen = set()
    for row in payload["bodies"]:
        key = row["source_table"], row["direction"]
        assert key in facts and key not in seen
        seen.add(key)
        fact = facts[key]
        assert row["event_id"] == BODY_EVENT_ID and row["content_type"] == fact["content_type"]
        try:
            fact["body"].decode("utf-8")
            encoding = "utf8"
        except UnicodeDecodeError:
            encoding = "base64"
        assert row["encoding"] == encoding
        content = row["content"].encode() if row["encoding"] == "utf8" else base64.b64decode(row["content"], validate=True)
        assert content == fact["body"] and row["stored_bytes"] == len(content)
        assert row["original_bytes"] == fact["original_bytes"] and row["truncated"] == fact["truncated"]
        assert row["truncated_for_transport"] is False

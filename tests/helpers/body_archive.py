"""Read archived bodies out of a session's ``session.bodies`` from a test.

Bodies left SQLite for a block archive beside it: ``event_body_blobs`` says
which block a body is in and where inside that block it starts, and the block
itself is a deflated span of raw bytes. A black-box test that wants to know
what the ledger actually stored has to read it the way the product does, so
this is the reader, in the few lines the format needs.

The layout, from ``crates/capsem-archive/src/format.rs``::

    file header (16 bytes):  magic "CAPSEMBL"  u16 version  u16 pad  u32 pad
    block (repeated):        magic "BLK1"  u32 raw_len  u32 comp_len
                             blake3(raw)[32]  deflate(raw)

Every read verifies the block's hash and the body's own hash, exactly as the
product reader does: bytes that do not match the row that named them are a
broken ledger, not a body, and a test must not quietly assert on them.
"""

from __future__ import annotations

import json
import sqlite3
import zlib
from pathlib import Path
from typing import Any

FILE_MAGIC = b"CAPSEMBL"
FILE_VERSION = 1
FILE_HEADER_BYTES = 16
BLOCK_MAGIC = b"BLK1"
BLOCK_HEADER_BYTES = 44


def archive_path_for_db(db_path: Path | str) -> Path:
    """``session.bodies`` beside ``session.db``."""
    return Path(db_path).with_suffix(".bodies")


def _main_db_path(conn: sqlite3.Connection) -> Path:
    for _seq, name, file in conn.execute("PRAGMA database_list"):
        if name == "main":
            if not file:
                raise AssertionError("an in-memory session DB has no body archive")
            return Path(file)
    raise AssertionError("connection has no main database")


def read_archived_body(
    db_path: Path | str,
    event_id: str,
    source_table: str,
    direction: str,
) -> bytes | None:
    """One archived body, or ``None`` when the index has no such row."""
    with sqlite3.connect(f"file:{Path(db_path)}?mode=ro", uri=True) as conn:
        row = conn.execute(
            """
            SELECT block_offset, body_offset, body_len, body_hash
            FROM event_body_blobs
            WHERE event_id = ? AND source_table = ? AND direction = ?
            """,
            (event_id, source_table, direction),
        ).fetchone()
    if row is None:
        return None
    block_offset, body_offset, body_len, body_hash = row
    block = _inflate_block(archive_path_for_db(db_path), int(block_offset))
    body = block[int(body_offset) : int(body_offset) + int(body_len)]
    if len(body) != int(body_len):
        raise AssertionError(f"body of {event_id} runs past the end of its block")
    got = _blake3(body)
    if got != body_hash:
        raise AssertionError(
            f"archived body of {event_id} does not match the row that named it: "
            f"index says {body_hash}, bytes hash to {got}"
        )
    return body


def _blake3(data: bytes) -> str:
    try:
        from blake3 import blake3 as _hash  # type: ignore[import-not-found]
    except ImportError as error:  # pragma: no cover - environment-dependent
        raise AssertionError(
            "reading an archived body verifies its blake3 hash; install the blake3 package"
        ) from error
    return "blake3:" + _hash(data).hexdigest()


def _inflate_block(archive: Path, block_offset: int) -> bytes:
    data = archive.read_bytes()
    header = data[:FILE_HEADER_BYTES]
    if header[:8] != FILE_MAGIC or int.from_bytes(header[8:10], "little") != FILE_VERSION:
        raise AssertionError(f"{archive} is not a capsem body archive")
    block = data[block_offset : block_offset + BLOCK_HEADER_BYTES]
    if block[:4] != BLOCK_MAGIC:
        raise AssertionError(f"no block at offset {block_offset} of {archive}")
    raw_len = int.from_bytes(block[4:8], "little")
    comp_len = int.from_bytes(block[8:12], "little")
    start = block_offset + BLOCK_HEADER_BYTES
    raw = zlib.decompress(data[start : start + comp_len], -15)
    if len(raw) != raw_len:
        raise AssertionError(f"block at {block_offset} of {archive} inflated to the wrong length")
    return raw


def security_payload_at(db_path: Path | str, event_id: str) -> dict[str, Any]:
    """The forensic payload of one ``security_rule_events`` match, parsed.

    The payload is archive-backed rather than a column, so a test that used to
    read ``event_json`` off the row reads it here instead. A match with no
    payload is a ledger that lost one, and fails.
    """
    body = read_archived_body(db_path, event_id, "security_rule_events", "payload")
    if body is None:
        raise AssertionError(f"security rule match {event_id} has no archived payload")
    return json.loads(body.decode())


def security_payload(conn: sqlite3.Connection, event_id: str) -> dict[str, Any]:
    """`security_payload_at`, for a test that already holds the connection."""
    return security_payload_at(_main_db_path(conn), event_id)

"""Read archived bodies out of a session's ``session.bodies`` from a test.

Bodies left SQLite for a block archive beside it: ``event_body_blobs`` says
which block a body is in and where inside that block it starts, and the block
itself is a deflated span of raw bytes. A black-box test that wants to know
what the ledger actually stored has to read it the way the product does, so
this is the reader, in the few lines the format needs.

This file is a second implementation of a format whose first implementation is
``crates/capsem-archive/src/format.rs``, and it has to track it: the magics, the
version, the header widths, the deflate variant and the two size ceilings below
are copied from there, and a change on that side that is not made here reads
garbage or refuses a valid archive. It is the only copy that is allowed to
exist -- ``tests/citadel/test_body_archive_format_is_one_place.py`` allowlists exactly
this path, for exactly that reason, so that a third parser cannot appear
quietly.

The layout, from that file::

    file header (16 bytes):  magic "CAPSEMBL"  u16 version  u16 pad  u32 pad
    block (repeated):        magic "BLK1"  u32 raw_len  u32 comp_len
                             blake3(raw)[32]  deflate(raw)

Every read verifies the block's blake3 against its header and the body's own
blake3 against the index row that named it, exactly as the product reader does:
bytes that do not match what named them are a broken ledger, not a body, and a
test must not quietly assert on them. Both lengths are bounded before anything
is allocated or inflated, because they come off disk.
"""

from __future__ import annotations

import contextlib
import json
import sqlite3
import zlib
from pathlib import Path
from typing import Any, BinaryIO

FILE_MAGIC = b"CAPSEMBL"
FILE_VERSION = 1
FILE_HEADER_BYTES = 16
BLOCK_MAGIC = b"BLK1"
BLOCK_HEADER_BYTES = 44
MAX_BLOCK_RAW_BYTES = 16 * 1024 * 1024
MAX_BLOCK_COMP_BYTES = MAX_BLOCK_RAW_BYTES + 64 * 1024


def archive_path_for_db(db_path: Path | str) -> Path:
    """``session.bodies`` beside ``session.db``."""
    return Path(db_path).with_suffix(".bodies")


def _blake3(data: bytes) -> str:
    try:
        from blake3 import blake3 as _hash  # type: ignore[import-not-found]
    except ImportError as error:  # pragma: no cover - environment-dependent
        raise AssertionError(
            "reading an archived body verifies its blake3 hash; install the blake3 package"
        ) from error
    return _hash(data).hexdigest()


class SessionArchive:
    """One session archive, with the product reader's one-block cache.

    Held across a page of rows rather than rebuilt per body: the bodies of one
    page mostly share a block, and reading the file per body turned a page into
    N opens and N inflates of the same bytes.

    It holds no file descriptor between reads. The archive is opened only on a
    block-cache miss -- exactly when a block has to be inflated anyway -- and
    closed before the read returns. Callers keep one of these for a whole test
    function, well past the loop that needs it, and an instance that held the
    file open leaked it into pytest's unraisable-exception check, which fails
    the test that happened to be running when the collector found it.
    """

    def __init__(self, db_path: Path | str) -> None:
        self.db_path = Path(db_path)
        self._archive = archive_path_for_db(self.db_path)
        self._verify_file_header()
        self._cached_offset: int | None = None
        self._cached_block: bytes = b""

    def close(self) -> None:
        """Nothing is held open; drop the cached block."""
        self._cached_offset, self._cached_block = None, b""

    def __enter__(self) -> SessionArchive:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def read(self, event_id: str, source_table: str, direction: str) -> bytes | None:
        """One archived body, or ``None`` when the index has no such row."""
        with contextlib.closing(
            sqlite3.connect(f"file:{self.db_path}?mode=ro", uri=True)
        ) as conn:
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
        block_offset, body_offset, body_len, body_hash = (int(row[0]), int(row[1]), int(row[2]), row[3])
        block = self._block(block_offset)
        body = block[body_offset : body_offset + body_len]
        if len(body) != body_len:
            raise AssertionError(f"body of {event_id} runs past the end of its block")
        got = f"blake3:{_blake3(body)}"
        if got != body_hash:
            raise AssertionError(
                f"archived body of {event_id} does not match the row that named it: "
                f"index says {body_hash}, bytes hash to {got}"
            )
        return body

    def security_payload(self, event_id: str) -> dict[str, Any]:
        """The forensic payload of one ``security_rule_events`` match, parsed."""
        body = self.read(event_id, "security_rule_events", "payload")
        if body is None:
            raise AssertionError(f"security rule match {event_id} has no archived payload")
        return json.loads(body.decode())

    def _verify_file_header(self) -> None:
        with self._archive.open("rb") as file:
            header = file.read(FILE_HEADER_BYTES)
        if header[:8] != FILE_MAGIC or int.from_bytes(header[8:10], "little") != FILE_VERSION:
            raise AssertionError(f"{self._archive} is not a capsem body archive")

    def _block(self, block_offset: int) -> bytes:
        if self._cached_offset == block_offset:
            return self._cached_block
        with self._archive.open("rb") as file:
            raw = self._read_block(file, block_offset)
        self._cached_offset, self._cached_block = block_offset, raw
        return raw

    def _read_block(self, file: BinaryIO, block_offset: int) -> bytes:
        file.seek(block_offset)
        header = file.read(BLOCK_HEADER_BYTES)
        if len(header) != BLOCK_HEADER_BYTES or header[:4] != BLOCK_MAGIC:
            raise AssertionError(f"no block at offset {block_offset} of {self._archive}")
        raw_len = int.from_bytes(header[4:8], "little")
        comp_len = int.from_bytes(header[8:12], "little")
        # Bounded before anything is sized by them: these lengths come off
        # disk, and the product refuses the same two ceilings rather than
        # allocating whatever a forged or torn header asks for.
        #
        # Zero is refused with them, and not as a nicety: `max_length=0` means
        # *unlimited* to Python's inflater, so a forged `raw_len` of 0 would
        # turn the bound below into no bound at all. A block of nothing cannot
        # exist anyway -- `body_blocks.raw_len` is CHECK(raw_len > 0), and the
        # Rust reader's `decompress_to_vec_with_limit(_, 0)` errors.
        if not 0 < raw_len <= MAX_BLOCK_RAW_BYTES or not 0 < comp_len <= MAX_BLOCK_COMP_BYTES:
            raise AssertionError(
                f"block at {block_offset} of {self._archive} declares "
                f"raw_len={raw_len} comp_len={comp_len}, outside the archive's bounds"
            )
        expected_hash = header[12:BLOCK_HEADER_BYTES].hex()
        compressed = file.read(comp_len)
        if len(compressed) != comp_len:
            raise AssertionError(f"block at {block_offset} of {self._archive} is truncated")
        # Raw deflate, matching miniz_oxide's `compress_to_vec`, and bounded by
        # the length the header declared so a decompression bomb cannot be
        # inflated by a test either.
        inflater = zlib.decompressobj(-15)
        raw = inflater.decompress(compressed, raw_len)
        # `eof` is the third thing that can be wrong: a stream that stopped at
        # the right length without reaching its own end is a truncated block
        # that happens to measure correctly, and unconsumed_tail is empty when
        # the limit was never the thing that stopped it.
        if len(raw) != raw_len or not inflater.eof or inflater.unconsumed_tail:
            raise AssertionError(
                f"block at {block_offset} of {self._archive} did not inflate to the "
                f"{raw_len} bytes it declares"
            )
        if _blake3(raw) != expected_hash:
            raise AssertionError(
                f"block at {block_offset} of {self._archive} does not match its own hash: "
                f"header says {expected_hash}, bytes hash to {_blake3(raw)}"
            )
        return raw


def read_archived_body(
    db_path: Path | str,
    event_id: str,
    source_table: str,
    direction: str,
) -> bytes | None:
    """One archived body, for a caller reading a single row."""
    with SessionArchive(db_path) as archive:
        return archive.read(event_id, source_table, direction)


def security_payload_at(db_path: Path | str, event_id: str) -> dict[str, Any]:
    """The forensic payload of one ``security_rule_events`` match, parsed.

    The payload is archive-backed rather than a column, so a test that used to
    read ``event_json`` off the row reads it here instead. A match with no
    payload is a ledger that lost one, and fails.

    A caller walking a page of rows should hold a `SessionArchive` instead, so
    the block those rows share is inflated once.
    """
    with SessionArchive(db_path) as archive:
        return archive.security_payload(event_id)


def _main_db_path(conn: sqlite3.Connection) -> Path:
    for _seq, name, file in conn.execute("PRAGMA database_list"):
        if name == "main":
            if not file:
                raise AssertionError("an in-memory session DB has no body archive")
            return Path(file)
    raise AssertionError("connection has no main database")


def session_archive(conn: sqlite3.Connection) -> SessionArchive:
    """The archive beside the session a test already has open."""
    return SessionArchive(_main_db_path(conn))


def security_payload(conn: sqlite3.Connection, event_id: str) -> dict[str, Any]:
    """`security_payload_at`, for a test that already holds the connection."""
    return security_payload_at(_main_db_path(conn), event_id)

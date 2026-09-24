"""Read archived bodies out of a session's v3 generation from a test.

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

The layout, from that file (version 3)::

    file header (80 bytes):  magic "CAPSEMBL"  u16 version=3  u16 flags=0
                             u32 header_len=80  archive_id[16]
                             generation_id[16]  blake3(bytes[0:48])[32]
    block header (8 bytes):  magic "BLK2"  u8 codec (1 = raw deflate)  u8 flags  u16 pad
    segment (repeated):      magic "SGMT"  u8 flags (bit 0 FINAL)  u8[3] pad
                             u32 raw_start  u32 raw_len  u32 comp_len
                             blake3(segment raw)[32]  deflate bytes

A block is one deflate stream cut at sync-flush points into segments; one
inflater runs across them, and the last segment of a closed block ends the
stream. A block that is still being written simply has no FINAL segment yet.

Every read verifies each segment's blake3 against its header and the body's
own blake3 against the index row that named it, exactly as the product reader
does: bytes that do not match what named them are a broken ledger, not a
body, and a test must not quietly assert on them. Every length is bounded
before anything is allocated or inflated, because it comes off disk.
"""

from __future__ import annotations

import contextlib
import importlib
import json
import sqlite3
import zlib
from pathlib import Path
from typing import Any, BinaryIO, Self

FILE_MAGIC = b"CAPSEMBL"
FILE_VERSION = 3
FILE_HEADER_BYTES = 80
BLOCK_MAGIC = b"BLK2"
BLOCK_HEADER_BYTES = 8
CODEC_DEFLATE = 1
SEGMENT_MAGIC = b"SGMT"
SEGMENT_HEADER_BYTES = 52
SEGMENT_FINAL = 0x01
SYNC_FLUSH_TAIL = b"\x00\x00\xff\xff"
MAX_BLOCK_RAW_BYTES = 16 * 1024 * 1024
MAX_SEGMENT_EXPANSION = 64 * 1024


# The ledgers that archive the event they are about as a ``payload``. They share
# the event id -- a rule match, the decision it drove, the ask it raised -- so a
# payload read names the table as well as the event.
SECURITY_PAYLOAD_TABLES = frozenset(
    {"security_rule_events", "security_decision_events", "security_ask_events"}
)


def archive_path_for_db(db_path: Path | str) -> Path:
    """The private ``session.bodies`` generation directory beside the DB."""
    return Path(db_path).with_suffix(".bodies")


def _blake3(data: bytes) -> str:
    try:
        module = importlib.import_module("blake3")
    except ModuleNotFoundError as error:  # pragma: no cover - environment-dependent
        raise AssertionError(
            "reading an archived body verifies its blake3 hash; install the blake3 package"
        ) from error
    return str(module.blake3(data).hexdigest())


def _archive_state(db_path: Path) -> tuple[bytes, bytes, int]:
    with contextlib.closing(
        sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    ) as conn:
        rows = conn.execute(
            "SELECT archive_id, generation_id, format_version, committed_end, revision "
            "FROM archive_state WHERE singleton = 1"
        ).fetchall()
    if len(rows) != 1:
        raise AssertionError(f"{db_path} must select exactly one archive generation")
    archive_id, generation_id, version, committed_end, revision = rows[0]
    if (
        not isinstance(archive_id, bytes)
        or len(archive_id) != 16
        or not isinstance(generation_id, bytes)
        or len(generation_id) != 16
        or version != FILE_VERSION
        or not isinstance(committed_end, int)
        or committed_end < FILE_HEADER_BYTES
        or not isinstance(revision, int)
        or revision < 1
        or archive_id[6] >> 4 != 4
        or archive_id[8] >> 6 != 2
        or generation_id[6] >> 4 != 4
        or generation_id[8] >> 6 != 2
    ):
        raise AssertionError(f"{db_path} has an invalid v3 archive_state singleton")
    return archive_id, generation_id, committed_end


def _generation_name(generation_id: bytes) -> str:
    return f"g-{generation_id.hex()}.cbl"


def generation_path_for_db(db_path: Path | str) -> Path:
    """The exact generation selected by the database singleton."""
    path = Path(db_path)
    _archive_id, generation_id, _committed_end = _archive_state(path)
    return archive_path_for_db(path) / _generation_name(generation_id)


class _Cursor:
    """One block inflated up to the end of some segment, as the product keeps it."""

    def __init__(self, block_offset: int, disk_len: int, raw_len: int) -> None:
        self.block_offset = block_offset
        self.block_end = block_offset + disk_len
        self.raw_limit = raw_len
        self.inflater = zlib.decompressobj(-15)
        self.raw = bytearray()
        self.next_segment_at = block_offset + BLOCK_HEADER_BYTES
        self.finished = False


class SessionArchive:
    """One session archive, with the product reader's block cursor.

    Held across a page of rows rather than rebuilt per body: the bodies of one
    page mostly share a block, and reading the file per body turned a page into
    N opens and N inflates of the same bytes.

    It holds no file descriptor between reads. The archive is opened only when
    a segment has to be inflated, and closed before the read returns. Callers
    keep one of these for a whole test function, well past the loop that needs
    it, and an instance that held the
    file open leaked it into pytest's unraisable-exception check, which fails
    the test that happened to be running when the collector found it.
    """

    def __init__(self, db_path: Path | str) -> None:
        self.db_path = Path(db_path)
        self._archive = archive_path_for_db(self.db_path)
        archive_id, generation_id, self.committed_end = _archive_state(self.db_path)
        self.generation_path = self._archive / _generation_name(generation_id)
        self._expected_archive_id = archive_id
        self._expected_generation_id = generation_id
        self._verify_file_header()
        self._cursor: _Cursor | None = None

    def close(self) -> None:
        """Nothing is held open; drop the cursor."""
        self._cursor = None

    def __enter__(self) -> Self:
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
                SELECT b.block_offset, b.body_offset, b.body_len, b.body_hash,
                       k.disk_len, k.raw_len
                FROM event_body_blobs b
                JOIN body_blocks k ON k.block_offset = b.block_offset
                WHERE b.event_id = ? AND b.source_table = ? AND b.direction = ?
                """,
                (event_id, source_table, direction),
            ).fetchone()
        if row is None:
            return None
        block_offset, body_offset, body_len, body_hash, disk_len, raw_len = (
            int(row[0]), int(row[1]), int(row[2]), row[3], int(row[4]), int(row[5])
        )
        end = body_offset + body_len
        try:
            body = self._span(block_offset, body_offset, end, disk_len, raw_len)
        except AssertionError:
            # A cursor that failed part-way holds an inflater in an unknown
            # state; nothing may read from it again.
            self._cursor = None
            raise
        got = f"blake3:{_blake3(body)}"
        if got != body_hash:
            raise AssertionError(
                f"archived body of {event_id} does not match the row that named it: "
                f"index says {body_hash}, bytes hash to {got}"
            )
        return body

    def security_payload(
        self, event_id: str, source_table: str = "security_rule_events"
    ) -> dict[str, Any]:
        """The forensic payload one security ledger archived for an event, parsed.

        Rule matches, decisions and asks each archive the event they are about
        under their own table, so the table is part of the question: all three
        name the same event with a ``payload``.
        """
        if source_table not in SECURITY_PAYLOAD_TABLES:
            raise AssertionError(f"{source_table} does not archive a security payload")
        body = self.read(event_id, source_table, "payload")
        if body is None:
            raise AssertionError(f"{source_table} has no archived payload for {event_id}")
        return json.loads(body.decode())

    def _verify_file_header(self) -> None:
        with self.generation_path.open("rb") as file:
            header = file.read(FILE_HEADER_BYTES)
        if (
            len(header) != FILE_HEADER_BYTES
            or header[:8] != FILE_MAGIC
            or int.from_bytes(header[8:10], "little") != FILE_VERSION
            or header[10:12] != b"\x00\x00"
            or int.from_bytes(header[12:16], "little") != FILE_HEADER_BYTES
            or header[16:32] != self._expected_archive_id
            or header[32:48] != self._expected_generation_id
            or _blake3(header[:48]) != header[48:80].hex()
            or self.generation_path.stat().st_size < self.committed_end
        ):
            raise AssertionError(f"{self.generation_path} is not the selected v3 archive generation")

    def _span(
        self, block_offset: int, start: int, end: int, disk_len: int, raw_len: int
    ) -> bytes:
        """Raw bytes ``start..end`` of a block, inflating as far as they need."""
        if (
            start < 0
            or end < start
            or end > raw_len
            or raw_len > MAX_BLOCK_RAW_BYTES
            or block_offset < FILE_HEADER_BYTES
            or disk_len < BLOCK_HEADER_BYTES
            or block_offset + disk_len > self.committed_end
        ):
            raise AssertionError(f"a span ending at {end} is past any block's bounds")
        cursor = self._cursor
        if (
            cursor is None
            or cursor.block_offset != block_offset
            or cursor.block_end != block_offset + disk_len
            or cursor.raw_limit != raw_len
        ):
            cursor = self._cursor = self._start_block(block_offset, disk_len, raw_len)
        if len(cursor.raw) < end:
            with self.generation_path.open("rb") as file:
                while len(cursor.raw) < end:
                    if cursor.finished or cursor.next_segment_at >= cursor.block_end:
                        raise AssertionError(
                            f"span ending at {end} runs past the end of block {block_offset}"
                        )
                    self._inflate_segment(file, cursor)
        return bytes(cursor.raw[start:end])

    def _start_block(self, block_offset: int, disk_len: int, raw_len: int) -> _Cursor:
        with self.generation_path.open("rb") as file:
            file.seek(block_offset)
            header = file.read(BLOCK_HEADER_BYTES)
        if len(header) != BLOCK_HEADER_BYTES or header[:4] != BLOCK_MAGIC:
            raise AssertionError(f"no block at offset {block_offset} of {self._archive}")
        if header[4] != CODEC_DEFLATE or header[5:] != b"\x00\x00\x00":
            raise AssertionError(
                f"block at {block_offset} of {self.generation_path} uses codec {header[4]}, "
                "which this reader does not know"
            )
        return _Cursor(block_offset, disk_len, raw_len)

    def _inflate_segment(self, file: BinaryIO, cursor: _Cursor) -> None:
        at = cursor.next_segment_at
        file.seek(at)
        header = file.read(SEGMENT_HEADER_BYTES)
        where = f"segment at {at} of {self.generation_path}"
        if len(header) != SEGMENT_HEADER_BYTES:
            raise AssertionError(f"{where} is truncated: the block was never written this far")
        flags = header[4]
        raw_start = int.from_bytes(header[8:12], "little")
        raw_len = int.from_bytes(header[12:16], "little")
        comp_len = int.from_bytes(header[16:20], "little")
        final = bool(flags & SEGMENT_FINAL)
        # Bounded before anything is sized by them: these lengths come off
        # disk, and the product refuses the same bounds rather than allocating
        # whatever a forged or torn header asks for.
        #
        # A zero raw_len is refused unless the segment is FINAL, and not as a
        # nicety: ``max_length=0`` means *unlimited* to Python's inflater, so a
        # forged zero would turn the bound below into no bound at all. The
        # writer never cuts an empty segment.
        if (
            header[:4] != SEGMENT_MAGIC
            or flags & ~SEGMENT_FINAL
            or header[5:8] != b"\x00\x00\x00"
            or raw_start != len(cursor.raw)
            or raw_start + raw_len > cursor.raw_limit
            or comp_len > raw_len + MAX_SEGMENT_EXPANSION
            or (raw_len == 0 and not final)
            or at + SEGMENT_HEADER_BYTES + comp_len > cursor.block_end
        ):
            raise AssertionError(
                f"{where} declares flags={flags:#x} raw_start={raw_start} raw_len={raw_len} "
                f"comp_len={comp_len}, outside the archive's bounds"
            )
        compressed = file.read(comp_len)
        if len(compressed) != comp_len:
            raise AssertionError(f"{where} is truncated")
        if not final and not compressed.endswith(SYNC_FLUSH_TAIL):
            raise AssertionError(f"{where} does not end on a sync flush")
        # One inflater across the block's segments, bounded by the length the
        # header declared plus one byte, so an overlong segment is seen rather
        # than cut -- and never by zero, which Python reads as no bound at all.
        # `eof` must be reached on the FINAL segment and only there.
        inflater = cursor.inflater
        try:
            raw = inflater.decompress(compressed, raw_len + 1)
        except zlib.error as error:
            raise AssertionError(f"{where} did not inflate: {error}") from error
        if (
            len(raw) != raw_len
            or inflater.unconsumed_tail
            or inflater.eof != final
            or (final and inflater.unused_data)
        ):
            raise AssertionError(f"{where} did not inflate to the {raw_len} bytes it declares")
        expected_hash = header[20:SEGMENT_HEADER_BYTES].hex()
        if _blake3(raw) != expected_hash:
            raise AssertionError(
                f"{where} does not match its own hash: header says {expected_hash}, "
                f"bytes hash to {_blake3(raw)}"
            )
        cursor.raw += raw
        cursor.next_segment_at = at + SEGMENT_HEADER_BYTES + comp_len
        cursor.finished = final
        if cursor.next_segment_at == cursor.block_end and len(cursor.raw) != cursor.raw_limit:
            raise AssertionError(f"{where} ends at {len(cursor.raw)} raw bytes, not {cursor.raw_limit}")
        if final and cursor.next_segment_at != cursor.block_end:
            raise AssertionError(f"{where} is final before the recorded block extent ends")


def archived_bodies(db_path: Path | str) -> list[tuple[str, str, str, bytes]]:
    """Every body the session archived, as (source_table, event_id, direction, bytes).

    For the checks that have to see *everything* a session stored. A raw-secret
    scan that walks SQLite's text columns stopped seeing request and response
    bodies when they moved to the archive, and security payloads when those
    followed; it has to walk this too, or it passes by not looking.
    """
    with contextlib.closing(sqlite3.connect(f"file:{Path(db_path)}?mode=ro", uri=True)) as conn:
        has_index = conn.execute(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'event_body_blobs'"
        ).fetchone()
        if not has_index:
            return []
        keys = conn.execute(
            """
            SELECT source_table, event_id, direction FROM event_body_blobs
            ORDER BY block_offset, body_offset
            """
        ).fetchall()
    if not keys:
        return []
    with SessionArchive(db_path) as archive:
        bodies = []
        for source_table, event_id, direction in keys:
            body = archive.read(event_id, source_table, direction)
            if body is None:
                raise AssertionError(f"{source_table}/{direction} of {event_id} vanished mid-scan")
            bodies.append((source_table, event_id, direction, body))
        return bodies


def read_archived_body(
    db_path: Path | str,
    event_id: str,
    source_table: str,
    direction: str,
) -> bytes | None:
    """One archived body, for a caller reading a single row."""
    with SessionArchive(db_path) as archive:
        return archive.read(event_id, source_table, direction)


def security_payload_at(
    db_path: Path | str, event_id: str, source_table: str = "security_rule_events"
) -> dict[str, Any]:
    """The forensic payload one security ledger archived for an event, parsed.

    The payload is archive-backed rather than a column, so a test that used to
    read ``event_json`` off the row reads it here instead. A match with no
    payload is a ledger that lost one, and fails.

    A caller walking a page of rows should hold a `SessionArchive` instead, so
    the block those rows share is inflated once.
    """
    with SessionArchive(db_path) as archive:
        return archive.security_payload(event_id, source_table)


def _main_db_path(conn: sqlite3.Connection) -> Path:
    for _seq, name, file in conn.execute("PRAGMA database_list"):
        if name == "main":
            if not file:
                raise AssertionError("an in-memory session DB has no body archive")
            return Path(file)
    raise AssertionError("connection has no main database")


def ledger_path(conn: sqlite3.Connection) -> Path:
    """The file a test's open session connection reads, for a path-based helper."""
    return _main_db_path(conn)


def session_archive(conn: sqlite3.Connection) -> SessionArchive:
    """The archive beside the session a test already has open."""
    return SessionArchive(_main_db_path(conn))


def security_payload(
    conn: sqlite3.Connection, event_id: str, source_table: str = "security_rule_events"
) -> dict[str, Any]:
    """`security_payload_at`, for a test that already holds the connection."""
    return security_payload_at(_main_db_path(conn), event_id, source_table)

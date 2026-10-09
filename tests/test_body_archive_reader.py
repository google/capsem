"""The test-side archive reader stays coherent while a writer appends."""

from __future__ import annotations

import os
import shutil
import sqlite3
from contextlib import closing
from pathlib import Path

import pyzstd
from blake3 import blake3
from helpers.body_archive import (
    BLOCK_MAGIC,
    CODEC_ZSTD,
    FILE_HEADER_BYTES,
    SEGMENT_FINAL,
    SEGMENT_MAGIC,
    SessionArchive,
    archived_bodies,
)

FIXTURE_DB = Path(__file__).resolve().parent / "fixtures/session/test.db"


def _ledger(tmp_path: Path) -> Path:
    db = tmp_path / "session.db"
    shutil.copyfile(FIXTURE_DB, db)
    bodies = db.with_suffix(".bodies")
    bodies.mkdir(mode=0o700)
    for generation in FIXTURE_DB.with_suffix(".bodies").iterdir():
        shutil.copyfile(generation, bodies / generation.name)
    os.close(
        os.open(
            db.with_name(db.name + "-archive.lock"), os.O_CREAT | os.O_WRONLY, 0o600
        )
    )
    return db


def _committed_end(db: Path, value: int | None = None) -> int:
    with closing(sqlite3.connect(db)) as conn:
        if value is not None:
            conn.execute(
                "UPDATE archive_state SET committed_end = ? WHERE singleton = 1",
                (value,),
            )
            conn.commit()
        return conn.execute(
            "SELECT committed_end FROM archive_state WHERE singleton = 1"
        ).fetchone()[0]


def test_a_body_committed_after_the_reader_opened_is_read(tmp_path: Path) -> None:
    """A running VM keeps appending: the reader opens, the writer commits more
    of the block and the index row naming it, then the reader asks for it. The
    row and the committed end come from one snapshot, so the read succeeds."""
    db = _ledger(tmp_path)
    expected = {
        (table, event_id, direction): body
        for table, event_id, direction, body in archived_bodies(db)
    }
    committed = _committed_end(db)
    _committed_end(db, FILE_HEADER_BYTES)  # what the reader saw when it opened
    with SessionArchive(db) as archive:
        _committed_end(db, committed)  # the writer commits the rest
        read = {key: archive.read(key[1], key[0], key[2]) for key in expected}
    assert read == expected


def test_zstd_segments_share_one_bounded_decoder_stream(tmp_path: Path) -> None:
    db = _ledger(tmp_path)
    expected = archived_bodies(db)
    with closing(sqlite3.connect(db)) as conn:
        row = conn.execute(
            "SELECT block_offset, disk_len, raw_len FROM body_blocks"
        ).fetchone()
    with SessionArchive(db) as archive:
        block_offset, disk_len, raw_len = map(int, row)
        raw = archive._span(block_offset, 0, raw_len, disk_len, raw_len)
        generation = archive.generation_path

    split = len(raw) // 2
    compressor = pyzstd.ZstdCompressor(3)
    compressed = (
        compressor.compress(raw[:split])
        + compressor.flush(pyzstd.ZstdCompressor.FLUSH_BLOCK),
        compressor.compress(raw[split:])
        + compressor.flush(pyzstd.ZstdCompressor.FLUSH_FRAME),
    )

    def segment(start: int, body: bytes, encoded: bytes, flags: int) -> bytes:
        return b"".join(
            (
                SEGMENT_MAGIC,
                bytes((flags, 0, 0, 0)),
                start.to_bytes(4, "little"),
                len(body).to_bytes(4, "little"),
                len(encoded).to_bytes(4, "little"),
                blake3(body).digest(),
                encoded,
            )
        )

    block = b"".join(
        (
            BLOCK_MAGIC,
            bytes((CODEC_ZSTD, 0, 0, 0)),
            segment(0, raw[:split], compressed[0], 0),
            segment(split, raw[split:], compressed[1], SEGMENT_FINAL),
        )
    )
    header = generation.read_bytes()[:FILE_HEADER_BYTES]
    generation.write_bytes(header + block)
    with closing(sqlite3.connect(db)) as conn:
        conn.execute("UPDATE body_blocks SET disk_len = ?", (len(block),))
        conn.execute(
            "UPDATE archive_state SET committed_end = ?",
            (FILE_HEADER_BYTES + len(block),),
        )
        conn.commit()

    assert archived_bodies(db) == expected

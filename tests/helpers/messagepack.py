"""Decode the MessagePack Capsem archives security forensics in.

`capsem_proto::forensic` writes each security payload as sparse, named
MessagePack (`rmp_serde` over a `serde_json::Value` projection), so decoding it
yields the same JSON-shaped value the ledger once stored as text: maps with
string keys, arrays, strings, numbers, booleans and nil. That is the whole
subset handled here; an extension type or trailing byte is a broken payload
and raises rather than guessing.
"""

from __future__ import annotations

import struct
from typing import Any


class MessagePackError(ValueError):
    """The bytes are not one complete value in the supported subset."""


def decode(data: bytes) -> Any:
    value, end = _value(memoryview(data), 0)
    if end != len(data):
        raise MessagePackError(f"{len(data) - end} trailing bytes after the value")
    return value


def _take(data: memoryview, at: int, size: int) -> tuple[bytes, int]:
    if at + size > len(data):
        raise MessagePackError(f"truncated: needed {size} bytes at offset {at}")
    return bytes(data[at : at + size]), at + size


def _unpack(data: memoryview, at: int, fmt: str) -> tuple[Any, int]:
    raw, end = _take(data, at, struct.calcsize(fmt))
    return struct.unpack(fmt, raw)[0], end


def _str(data: memoryview, at: int, size: int) -> tuple[str, int]:
    raw, end = _take(data, at, size)
    return raw.decode("utf-8"), end


def _array(data: memoryview, at: int, count: int) -> tuple[list[Any], int]:
    items = []
    for _ in range(count):
        item, at = _value(data, at)
        items.append(item)
    return items, at


def _map(data: memoryview, at: int, count: int) -> tuple[dict[Any, Any], int]:
    result: dict[Any, Any] = {}
    for _ in range(count):
        key, at = _value(data, at)
        result[key], at = _value(data, at)
    return result, at


_FIXED = {0xC0: None, 0xC2: False, 0xC3: True}
_NUMBERS = {
    0xCA: ">f", 0xCB: ">d",
    0xCC: ">B", 0xCD: ">H", 0xCE: ">I", 0xCF: ">Q",
    0xD0: ">b", 0xD1: ">h", 0xD2: ">i", 0xD3: ">q",
}
_LENGTHS = {0xD9: ">B", 0xDA: ">H", 0xDB: ">I", 0xC4: ">B", 0xC5: ">H", 0xC6: ">I",
            0xDC: ">H", 0xDD: ">I", 0xDE: ">H", 0xDF: ">I"}


def _value(data: memoryview, at: int) -> tuple[Any, int]:
    marker, at = _unpack(data, at, ">B")
    if marker <= 0x7F:
        return marker, at
    if marker >= 0xE0:
        return marker - 0x100, at
    if 0x80 <= marker <= 0x8F:
        return _map(data, at, marker & 0x0F)
    if 0x90 <= marker <= 0x9F:
        return _array(data, at, marker & 0x0F)
    if 0xA0 <= marker <= 0xBF:
        return _str(data, at, marker & 0x1F)
    if marker in _FIXED:
        return _FIXED[marker], at
    if marker in _NUMBERS:
        return _unpack(data, at, _NUMBERS[marker])
    if marker in _LENGTHS:
        length, at = _unpack(data, at, _LENGTHS[marker])
        if marker in (0xD9, 0xDA, 0xDB):
            return _str(data, at, length)
        if marker in (0xC4, 0xC5, 0xC6):
            return _take(data, at, length)
        if marker in (0xDC, 0xDD):
            return _array(data, at, length)
        return _map(data, at, length)
    raise MessagePackError(f"unsupported MessagePack marker 0x{marker:02x} at offset {at - 1}")

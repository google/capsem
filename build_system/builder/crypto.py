"""Owned Python cryptographic bindings used by verification and build tooling.

OpenSSL implements hashlib SHA; the blake3 package dispatches its Rust SIMD
backend. Algorithms required by wire formats stay explicit at the call site.
Guest EROFS verification calls the Rust owner through capsem-sysutil instead.
"""

from __future__ import annotations

import hashlib as _hashlib
import hmac as _hmac
import re
import secrets as _secrets
import uuid as _uuid
from collections.abc import Callable
from pathlib import Path
from typing import Any, BinaryIO, Protocol

try:
    from blake3 import blake3 as _blake3
except ModuleNotFoundError as error:
    _blake3 = None
    _BLAKE3_IMPORT_ERROR = error
else:
    _BLAKE3_IMPORT_ERROR = None

sha256 = _hashlib.sha256
sha1 = _hashlib.sha1  # Protocol-specific handshake checks only.
blake2b = _hashlib.blake2b
blake2s = _hashlib.blake2s
compare_digest = _hmac.compare_digest
token_hex = _secrets.token_hex
token_bytes = _secrets.token_bytes
token_urlsafe = _secrets.token_urlsafe
uuid_v4 = _uuid.uuid4


class Digest(Protocol):
    """Hash state shared by native digest implementations."""

    def update(self, data: bytes | bytearray | memoryview) -> None: ...

    def hexdigest(self) -> str: ...


def require_blake3() -> Callable[..., Any]:
    """Return the optional BLAKE3 binding or preserve its import failure."""
    if _blake3 is None:
        assert _BLAKE3_IMPORT_ERROR is not None
        raise _BLAKE3_IMPORT_ERROR
    return _blake3


def blake3(*args: Any, **kwargs: Any) -> Any:
    """Construct a BLAKE3 hasher without burdening SHA-only consumers."""
    return require_blake3()(*args, **kwargs)


def digest_file(path: Path, *, algorithm: str = "blake3") -> str:
    """Bounded-memory file hashing; callers retain descriptor/trust policy."""
    if algorithm not in {"blake3", "sha256"}:
        raise ValueError(f"unsupported integrity algorithm: {algorithm}")
    digest = blake3() if algorithm == "blake3" else sha256()
    buffer = bytearray(1 << 20)
    with path.open("rb") as source:
        while count := source.readinto(buffer):
            digest.update(memoryview(buffer)[:count])
    return digest.hexdigest()


def verify_reader(source: BinaryIO, expected: str, size: int) -> None:
    """Verify held content without replacing its descriptor with a path."""
    if type(size) is not int or size < 0 or not isinstance(expected, str):
        raise ValueError("invalid content integrity descriptor")
    match = re.fullmatch(r"(blake3|sha256):([0-9a-f]{64})", expected)
    if match is None:
        raise ValueError("invalid content integrity digest")
    algorithm, digest = match.groups()
    hasher = blake3() if algorithm == "blake3" else sha256()
    count = 0
    while chunk := source.read(min(1 << 20, size - count + 1)):
        count += len(chunk)
        if count > size:
            raise ValueError("content exceeds declared size")
        hasher.update(chunk)
    if count != size or hasher.hexdigest() != digest:
        raise ValueError("content integrity mismatch")

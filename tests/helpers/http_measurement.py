"""Strict single-attempt response drain for the shared HTTP transport owner."""
from __future__ import annotations

import http.client
import math
import socket
import time
from collections.abc import Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Protocol, cast


class DeadlineSocket(socket.socket):
    """Apply an absolute deadline to each raw read, including header trickles."""

    deadline: float | None = None

    def _remaining(self) -> None:
        if self.deadline is not None:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("HTTP measurement total deadline exceeded")
            self.settimeout(remaining)

    def recv_into(self, buffer, nbytes=0, flags=0):
        self._remaining()
        return super().recv_into(buffer, nbytes, flags)

    def sendall(self, data, flags=0):
        self._remaining()
        return super().sendall(data, flags)


class _FramingLimits(Protocol):
    _MAXHEADERS: int
    _MAXLINE: int


class _ChunkReader(Protocol):
    def _read_next_chunk_size(self) -> int: ...


# The stdlib implementation exposes these framing hooks but its stubs omit them.
_FRAMING = cast(_FramingLimits, http.client)


class StrictResponse(http.client.HTTPResponse):
    """The stdlib permits EOF in trailers; measurement requires their terminator."""

    def _read_and_discard_trailer(self) -> None:
        assert self.fp is not None
        for _ in range(_FRAMING._MAXHEADERS):
            line = self.fp.readline(_FRAMING._MAXLINE + 1)
            if not line:
                raise http.client.IncompleteRead(b"")
            if len(line) > _FRAMING._MAXLINE:
                raise http.client.LineTooLong("trailer line")
            if line == b"\r\n":
                return
        raise http.client.HTTPException("too many trailer headers")

    def _read_next_chunk_size(self) -> int:
        size = cast(_ChunkReader, super())._read_next_chunk_size()
        if size < 0:
            raise http.client.IncompleteRead(b"")
        return size


@dataclass(frozen=True)
class MeasuredResponse:
    """Client-observed durations since request start, never wire timestamps."""

    status: int
    headers: Mapping[str, str]
    body: bytes
    headers_ns: int
    first_body_byte_ns: int | None
    complete_ns: int
    attempts: int = 1


def validate_limits(timeout: float, max_response_bytes: int) -> None:
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("measurement timeout must be positive and finite")
    if type(max_response_bytes) is not int or max_response_bytes <= 0:
        raise ValueError("measurement body budget must be positive")


def drain(response: http.client.HTTPResponse, started: int, budget: int) -> MeasuredResponse:
    """Keep bounded original bytes and reject ambiguous/incomplete framing."""
    headers_ns = time.monotonic_ns() - started
    lengths = response.headers.get_all("Content-Length", [])
    encodings = response.headers.get_all("Transfer-Encoding", [])
    if len(lengths) > 1 or (lengths and (not lengths[0].isascii() or not lengths[0].isdigit())):
        raise ValueError("invalid measurement Content-Length")
    if encodings and (len(encodings) != 1 or encodings[0].lower() != "chunked" or lengths):
        raise ValueError("ambiguous or unsupported measurement transfer framing")
    # HTTPResponse owns HEAD/204/304 no-body semantics and decoded chunk framing.
    remaining = response.length
    if remaining is not None and remaining > budget:
        raise ValueError("measurement response exceeds body budget")
    data = bytearray()
    first = None
    while chunk := response.read1(min(64 * 1024, budget + 1 - len(data))):
        if first is None:
            first = time.monotonic_ns() - started
        if len(data) + len(chunk) > budget:
            raise ValueError("measurement response exceeds body budget")
        data.extend(chunk)
    if response.length not in (0, None):
        raise http.client.IncompleteRead(bytes(data), response.length)
    complete = time.monotonic_ns() - started
    return MeasuredResponse(response.status, MappingProxyType({key.lower(): value for key, value in response.getheaders()}), bytes(data),
                            headers_ns, first, complete)

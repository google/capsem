"""Real HTTP byte/framing/deadline contracts for strict measurement."""
from __future__ import annotations

import operator
import socket
import struct
import tempfile
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import FrozenInstanceError
from pathlib import Path
from typing import cast

import pytest
from helpers.http_transport import Transport


@contextmanager
def wire(parts: list[tuple[float, bytes]], *, requests: int = 1, uds: Path | None = None, reset: bool = False) -> Iterator[tuple[Transport, list[bytes]]]:
    listener = socket.socket(socket.AF_UNIX if uds else socket.AF_INET)
    listener.bind(str(uds) if uds else ("127.0.0.1", 0))
    listener.listen()
    listener.settimeout(2)
    seen: list[bytes] = []
    failures: list[BaseException] = []

    def serve() -> None:
        try:
            with listener.accept()[0] as conn:
                if reset:
                    conn.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
                conn.settimeout(2)
                for _ in range(requests):
                    head = b""
                    while b"\r\n\r\n" not in head:
                        chunk = conn.recv(4096)
                        if not chunk:
                            return
                        head += chunk
                    seen.append(head)
                    for delay, data in parts:
                        time.sleep(delay)
                        try:
                            conn.sendall(data)
                        except (BrokenPipeError, ConnectionResetError):
                            return
        except (OSError, AssertionError) as error:
            failures.append(error)

    thread = threading.Thread(target=serve)
    thread.start()
    transport = Transport(socket_path=str(uds)) if uds else Transport(host="127.0.0.1", port=listener.getsockname()[1])
    try:
        yield transport, seen
    finally:
        transport.close()
        listener.close()
        thread.join(timeout=3)
        assert not thread.is_alive()
        assert not failures


def test_strict_receipt_has_original_bytes_observed_clocks_and_no_replay() -> None:
    parts = [(0, b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n"), (0.015, b"a"), (0.015, b"bc")]
    with wire(parts, requests=2) as (transport, seen):
        for _ in range(2):
            value = transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
            assert value.body == b"abc" and value.status == 200 and value.attempts == 1
            assert value.first_body_byte_ns is not None
            assert 0 <= value.headers_ns < value.first_body_byte_ns < value.complete_ns
            with pytest.raises(FrozenInstanceError):
                value.__setattr__("attempts", 2)
            with pytest.raises(TypeError):
                operator.setitem(cast(dict[str, str], value.headers), "x", "bad")
        assert len(seen) == 2 and len(transport._connections) == 1


@pytest.mark.parametrize("response", [
    b"",
    b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc",
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n",
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nabc",
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n",
    b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\nabc",
    b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
])
def test_partial_or_invalid_response_fails_without_retry(response: bytes) -> None:
    with wire([(0, response)]) as (transport, seen):
        with pytest.raises(ConnectionError):
            transport.measure_request("GET", "/", timeout=0.5, max_response_bytes=32)
        assert len(seen) == 1 and not transport._connections


@pytest.mark.parametrize("parts", [
    [(0.1, b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx")],
    [(0, b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n"), (0.1, b"x")],
    [(0, b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n")] + [(0.02, b"x")] * 9,
    [(0, b"HTTP/1.1 200 OK\r\nX-Trickle: ")] + [(0.02, b"x")] * 9 + [(0, b"\r\nContent-Length: 0\r\n\r\n")],
])
def test_total_deadline_bounds_headers_body_and_trickle(parts: list[tuple[float, bytes]]) -> None:
    with wire(parts) as (transport, _):
        start = time.monotonic()
        with pytest.raises(ConnectionError, match=r"deadline|timed out"):
            transport.measure_request("GET", "/", timeout=0.05, max_response_bytes=32)
        assert time.monotonic() - start < 0.3
        assert not transport._connections


@pytest.mark.parametrize("framing", [b"Content-Length: 4", b"Transfer-Encoding: chunked", b"Connection: close"])
def test_body_budget_refuses_overflow_with_all_framings(framing: bytes) -> None:
    body = b"4\r\nabcd\r\n0\r\n\r\n" if framing.startswith(b"Transfer") else b"abcd"
    with wire([(0, b"HTTP/1.1 200 OK\r\n" + framing + b"\r\n\r\n" + body)]) as (transport, _):
        with pytest.raises(ConnectionError, match="budget"):
            transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
        assert not transport._connections


@pytest.mark.parametrize("method,status,head", [
    ("GET", 204, b""), ("GET", 200, b"Content-Length: 0\r\n"),
    ("HEAD", 200, b"Content-Length: 999\r\n"),
])
def test_empty_body_has_no_invented_first_byte(method: str, status: int, head: bytes) -> None:
    with wire([(0, f"HTTP/1.1 {status} OK\r\n".encode() + head + b"\r\n")]) as (transport, _):
        value = transport.measure_request(method, "/", timeout=1, max_response_bytes=3)
        assert value.body == b"" and value.first_body_byte_ns is None
        assert value.headers_ns <= value.complete_ns


@pytest.mark.parametrize("timeout,budget", [(0, 1), (float("inf"), 1), (float("nan"), 1), (1, 0), (1, True), (1, 1.5)])
def test_invalid_limits_fail_before_connect(timeout: float, budget: int) -> None:
    transport = Transport(host="127.0.0.1", port=1)
    with pytest.raises(ValueError):
        transport.measure_request("GET", "/", timeout=timeout, max_response_bytes=budget)
    assert not transport._connections


def test_chunked_complete_body_is_decoded_without_altering_original_bytes() -> None:
    with wire([(0, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\nX-End: yes\r\n\r\n")]) as (transport, _):
        receipt = transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
        assert receipt.body == b"abc" and receipt.headers["transfer-encoding"] == "chunked"


def test_strict_failure_does_not_open_a_retry_connection(monkeypatch: pytest.MonkeyPatch) -> None:
    with wire([(0, b"")]) as (transport, _):
        created = []
        original = transport._new_connection
        def connection(timeout: float):
            created.append(timeout)
            return original(timeout)
        monkeypatch.setattr(transport, "_new_connection", connection)
        with pytest.raises(ConnectionError):
            transport.measure_request("GET", "/", timeout=0.1, max_response_bytes=3)
        assert len(created) == 1


def test_cancellation_retires_connection(monkeypatch: pytest.MonkeyPatch) -> None:
    import helpers.http_transport as owner
    def cancel(*_args):
        raise KeyboardInterrupt
    with wire([(0, b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc")]) as (transport, _):
        monkeypatch.setattr(owner, "drain", cancel)
        with pytest.raises(KeyboardInterrupt):
            transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
        assert not transport._connections


def test_strict_uds_and_functional_reuse_share_the_same_transport_owner() -> None:
    # AF_UNIX on macOS needs a short path, independent of pytest temp-root length.
    with (
        tempfile.TemporaryDirectory(dir="/tmp", prefix="capsem-http-") as directory,
        wire([(0, b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc")], requests=2, uds=Path(directory) / "s") as (transport, seen),
    ):
        value = transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
        assert value.body == b"abc"
        assert transport.request("GET", "/")[2] == b"abc"
        assert len(seen) == 2


def test_unknown_hostname_is_refused_before_unbounded_dns_resolution() -> None:
    transport = Transport(host="fixture.invalid", port=1)
    with pytest.raises(ValueError, match="literal IP"):
        transport.measure_request("GET", "/", timeout=1, max_response_bytes=3)
    assert not transport._connections


@pytest.mark.parametrize("body", [b"", b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc"])
def test_connection_reset_never_produces_a_partial_receipt(body: bytes) -> None:
    with wire([(0, body)], reset=True) as (transport, seen):
        with pytest.raises(ConnectionError):
            transport.measure_request("GET", "/", timeout=1, max_response_bytes=32)
        assert len(seen) == 1 and not transport._connections


def test_first_body_timestamp_is_after_an_actual_client_read(monkeypatch: pytest.MonkeyPatch) -> None:
    import http.client

    import helpers.http_measurement as owner
    original_clock = time.monotonic_ns
    original_read = http.client.HTTPResponse.read1
    clocks: list[int] = []
    observed: list[int] = []
    caller = threading.get_ident()

    def clock() -> int:
        value = original_clock()
        if threading.get_ident() == caller:
            clocks.append(value)
        return value

    def read(response, *args):
        body = original_read(response, *args)
        if body and not observed:
            observed.append(original_clock())
        return body

    with wire([(0, b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n"), (0.01, b"x")]) as (transport, _):
        with monkeypatch.context() as trace:
            trace.setattr(owner.time, "monotonic_ns", clock)
            trace.setattr(http.client.HTTPResponse, "read1", read)
            receipt = transport.measure_request("GET", "/", timeout=1, max_response_bytes=1)
        assert len(clocks) == 4 and len(observed) == 1
        assert receipt.first_body_byte_ns == clocks[2] - clocks[0]
        assert clocks[2] >= observed[0] > clocks[1]

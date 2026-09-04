"""Correctness contracts for the shared kept-alive test transport."""

from __future__ import annotations

import http.client
import threading
from collections.abc import Callable
from typing import cast

import pytest
from helpers.http_transport import Transport


class Response:
    status = 200
    will_close = False

    def getheaders(self) -> list[tuple[str, str]]:
        return [("content-type", "application/json")]

    def read(self) -> bytes:
        return b"{}"


class Connection:
    def __init__(
        self,
        *,
        request_error: Exception | None = None,
        response_error: Exception | None = None,
        request_barrier: threading.Barrier | None = None,
    ) -> None:
        self.timeout: float | None = None
        self.sock = None
        self.closed = False
        self.request_error = request_error
        self.response_error = response_error
        self.request_barrier = request_barrier

    def request(self, *_args: object, **_kwargs: object) -> None:
        if self.request_barrier is not None:
            self.request_barrier.wait(timeout=5)
        if self.request_error is not None:
            raise self.request_error

    def getresponse(self) -> Response:
        if self.response_error is not None:
            raise self.response_error
        return Response()

    def close(self) -> None:
        self.closed = True


class ScriptedTransport(Transport):
    def __init__(self, factory: Callable[[], Connection]) -> None:
        super().__init__(host="127.0.0.1", port=1)
        self.factory = factory
        self.created: list[Connection] = []

    def _new_connection(self, timeout: float) -> http.client.HTTPConnection:
        del timeout
        connection = self.factory()
        self.created.append(connection)
        return cast(http.client.HTTPConnection, connection)


def test_shared_transport_owns_one_connection_per_concurrent_thread() -> None:
    workers = 8
    request_barrier = threading.Barrier(workers)
    transport = ScriptedTransport(lambda: Connection(request_barrier=request_barrier))
    errors: list[Exception] = []

    def request() -> None:
        try:
            transport.request("GET", "/status")
        except ConnectionError as error:  # pragma: no cover - asserted below
            errors.append(error)

    threads = [threading.Thread(target=request) for _ in range(workers)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)

    assert not errors
    assert all(not thread.is_alive() for thread in threads)
    assert len(transport.created) == workers
    transport.close()
    assert all(connection.closed for connection in transport.created)


def test_ambiguous_post_failure_is_never_replayed() -> None:
    transport = ScriptedTransport(
        lambda: Connection(
            request_error=BrokenPipeError("closed while sending"),
            response_error=http.client.RemoteDisconnected("no response"),
        )
    )

    with pytest.raises(ConnectionError, match="POST /mutation failed"):
        transport.request("POST", "/mutation", body=b"{}")

    assert len(transport.created) == 1


def test_idempotent_get_reconnects_once_after_stale_keep_alive() -> None:
    connections = iter(
        [
            Connection(response_error=http.client.RemoteDisconnected("stale")),
            Connection(),
        ]
    )
    transport = ScriptedTransport(lambda: next(connections))

    status, _, body = transport.request("GET", "/status")

    assert status == 200
    assert body == b"{}"
    assert len(transport.created) == 2

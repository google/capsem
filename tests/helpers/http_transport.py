"""HTTP for the test clients: one kept-alive connection, no subprocess.

Every test client used to run a `curl` process per request. Launching that
process cost about 14 ms here against a service that answers `/vms/list` in
under a millisecond, so a suite making thousands of calls spent minutes in
fork/exec, and every route timing anyone took through these helpers was the
cost of starting curl. This is the standard library's client over a Unix
socket or TCP, reusing the connection between requests.

Behaviour kept from curl: a 4xx or 5xx is a normal response, not an error;
only a transport failure raises `ConnectionError`; the timeout bounds each
socket operation.
"""

from __future__ import annotations

import http.client
import socket


class UdsConnection(http.client.HTTPConnection):
    """HTTP/1.1 over a Unix socket."""

    def __init__(self, socket_path: str, timeout: float):
        super().__init__("localhost", timeout=timeout)
        self.socket_path = socket_path

    def connect(self) -> None:
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.socket_path)


_RETRIABLE = (http.client.RemoteDisconnected, BrokenPipeError, ConnectionResetError, http.client.CannotSendRequest)


class Transport:
    """A kept-alive connection to one server.

    A server may close an idle kept-alive connection at any time; the next
    request then fails before anything reached the server, and is sent once
    more on a fresh connection. A request that failed after being sent is
    not retried: the server may have acted on it.
    """

    def __init__(self, *, socket_path: str | None = None, host: str | None = None, port: int | None = None):
        self._socket_path = socket_path
        self._host = host
        self._port = port
        self._conn: http.client.HTTPConnection | None = None

    def _connection(self, timeout: float) -> http.client.HTTPConnection:
        if self._conn is None:
            if self._socket_path is not None:
                self._conn = UdsConnection(self._socket_path, timeout)
            else:
                host = self._host
                if host is None:
                    raise ValueError("TCP transport requires a host")
                self._conn = http.client.HTTPConnection(host, self._port, timeout=timeout)
        self._conn.timeout = timeout
        if self._conn.sock is not None:
            self._conn.sock.settimeout(timeout)
        return self._conn

    def close(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None

    def __del__(self) -> None:
        # Tests create clients ad hoc and drop them; closing here keeps a
        # kept-alive socket from surfacing as a ResourceWarning at collection.
        self.close()

    def request(
        self,
        method: str,
        path: str,
        *,
        headers: dict[str, str] | None = None,
        body: bytes | None = None,
        timeout: float = 60,
    ) -> tuple[int, dict[str, str], bytes]:
        """One request; returns (status, lower-cased response headers, body bytes)."""
        for attempt in (0, 1):
            conn = self._connection(timeout)
            sent = False
            try:
                try:
                    conn.request(method, path, body=body, headers=headers or {})
                except (BrokenPipeError, ConnectionResetError):
                    # A server that refuses a body (413) answers and closes
                    # before the client finished sending; the answer is there.
                    response = conn.getresponse()
                else:
                    sent = True
                    response = conn.getresponse()
                data = response.read()
                result = (response.status, {k.lower(): v for k, v in response.getheaders()}, data)
                if response.will_close:
                    self.close()
                return result
            except _RETRIABLE as error:
                self.close()
                if attempt == 1 or (sent and method not in {"GET", "HEAD"}):
                    raise ConnectionError(f"{method} {path} failed: {error}") from error
            except TimeoutError as error:
                self.close()
                raise ConnectionError(f"{method} {path} timed out after {timeout}s") from error
            except OSError as error:
                self.close()
                raise ConnectionError(f"{method} {path} failed: {error}") from error
        raise ConnectionError(f"{method} {path} failed after retry")

    def status_line(self, path: str, headers: dict[str, str], timeout: float = 5) -> int:
        """The status code of a raw GET, read straight off the socket.

        For requests whose response the HTTP client would not hand back --
        a WebSocket `101 Switching Protocols` is consumed as an interim
        response and the client waits for the final one that never comes.
        """
        if self._socket_path is not None:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            target = self._socket_path
        else:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            host = self._host
            if host is None:
                raise ValueError("TCP transport requires a host")
            target = (host, self._port)
        sock.settimeout(timeout)
        try:
            sock.connect(target)
            lines = [f"GET {path} HTTP/1.1", "Host: localhost"] + [f"{k}: {v}" for k, v in headers.items()]
            sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
            head = b""
            while b"\r\n" not in head:
                chunk = sock.recv(4096)
                if not chunk:
                    return 0
                head += chunk
            parts = head.split(b" ", 2)
            return int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
        except OSError:
            return 0
        finally:
            sock.close()

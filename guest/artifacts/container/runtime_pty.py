"""Own runc's workload PTY and serve both directions without blocking."""

import array
import collections
import contextlib
import ctypes
import fcntl
import os
import runpy
import selectors
import socket
import struct
import subprocess
import termios
import threading
import time
from pathlib import Path

_wire = runpy.run_path(str(Path(__file__).with_name("runtime_console.py")))
FrameReader, WriteQueue, encode_output, watch, FRAME_LIMIT, DATA_LIMIT = (
    _wire[name] for name in ("FrameReader", "WriteQueue", "encode_output", "watch", "FRAME_LIMIT", "DATA_LIMIT")
)

def receive_terminal(connection):
    """Accept exactly one non-inheritable terminal FD; close every refused FD."""
    descriptors = array.array("i")
    # Linux permits at most 253 SCM_RIGHTS FDs. Receive all of that bounded
    # message so even an invalid multi-FD handoff can be closed completely.
    data, ancillary, flags, _ = connection.recvmsg(4096, socket.CMSG_SPACE(256 * descriptors.itemsize), getattr(socket, "MSG_CMSG_CLOEXEC", 0))
    valid = bool(data) and not flags & (socket.MSG_CTRUNC | socket.MSG_TRUNC)
    for level, kind, value in ancillary:
        if (level, kind) != (socket.SOL_SOCKET, socket.SCM_RIGHTS):
            valid = False
            continue
        complete = len(value) - len(value) % descriptors.itemsize
        descriptors.frombytes(value[:complete])
        valid &= complete == len(value)
    try:
        if not valid or len(descriptors) != 1 or not os.isatty(descriptors[0]):
            raise ValueError("runc must hand off exactly one terminal descriptor")
        os.set_inheritable(descriptors[0], False)
        return descriptors.pop()
    finally:
        for fd in descriptors:
            os.close(fd)


def become_subreaper():
    # PR_SET_CHILD_SUBREAPER: runc exits after handoff; the workload and its
    # descendants must become this launcher's children, not orphaned daemons.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


class RuntimeConsole:
    def __init__(self):
        self.master = None
        self.pid = None
        self.returncode = None

    def start(self, argv, container, path, pid_file, timeout):
        become_subreaper()
        deadline = time.monotonic() + timeout
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(path))
            path.chmod(0o600)
            listener.listen(1)
            listener.settimeout(min(0.05, timeout))
            runtime = subprocess.Popen(
                [*argv, "--detach", "--console-socket", str(path), container], stdin=subprocess.DEVNULL
            )
            try:
                while True:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("runc terminal handoff timed out")
                    try:
                        connection, _ = listener.accept()
                        break
                    except TimeoutError:
                        if runtime.poll() is not None:
                            raise RuntimeError(f"runc exited before terminal handoff: {runtime.returncode}") from None
                with connection:
                    connection.settimeout(max(0.001, deadline - time.monotonic()))
                    self.master = receive_terminal(connection)
                result = runtime.wait(timeout=max(0.001, deadline - time.monotonic()))
                if result != 0:
                    raise RuntimeError(f"runc failed after terminal handoff: {result}")
                pid = int(pid_file.read_text())
                if pid <= 1:
                    raise ValueError("runc supplied an invalid workload pid")
                self.pid = pid
                return self.master
            finally:
                if runtime.poll() is None:
                    runtime.kill()
                runtime.wait(timeout=5)
                path.unlink()

    def wait(self, timeout=None):
        """Reap all adopted children, retaining the workload's actual exit code.

        Only the dedicated launcher calls this, after runc itself was reaped.
        PID-namespace teardown ends the workload's remaining descendants.
        """
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            try:
                pid, status = os.waitpid(-1, 0 if deadline is None else os.WNOHANG)
            except ChildProcessError:
                if self.pid is not None and self.returncode is None:
                    raise RuntimeError("runc workload was not adopted by its launcher") from None
                return self.returncode
            if pid == self.pid:
                self.returncode = os.waitstatus_to_exitcode(status)
                if self.returncode < 0:
                    self.returncode = 128 - self.returncode
            if pid == 0:
                # Only WNOHANG can return zero; that branch always has a deadline.
                assert deadline is not None and timeout is not None
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired("workload exit", timeout)
                time.sleep(min(0.01, max(0, deadline - time.monotonic())))

    def close(self):
        if self.master is not None:
            os.close(self.master)
            self.master = None


class Console:
    def __init__(self, master, listener, *, replay_limit):
        self._master, self._listener = master, listener
        self._replay = collections.deque()
        self._replay_size, self._limit = 0, replay_limit
        self._clients = {}
        self._input = WriteQueue()
        self._lock = threading.Lock()
        os.set_blocking(master, False)
        listener.setblocking(False)

    def replayed(self):
        with self._lock:
            return b"".join(self._replay)

    def _remember(self, data):
        with self._lock:
            self._replay.append(data)
            self._replay_size += len(data)
            while self._replay_size > self._limit:
                excess = self._replay_size - self._limit
                head = self._replay[0]
                if len(head) <= excess:
                    self._replay.popleft()
                    self._replay_size -= len(head)
                else:
                    self._replay[0] = head[excess:]
                    self._replay_size -= excess

    def _drop(self, selector, client):
        with contextlib.suppress(KeyError, OSError):
            selector.unregister(client)
        self._clients.pop(client, None)
        client.close()

    def serve(self):
        selector = selectors.DefaultSelector()
        live = True
        try:
            while live or any(output for _, output in self._clients.values()):
                # Admission stops at the existing data/frame budgets. Only
                # the current bounded record may overshoot; readiness resumes
                # it. Backpressure never discards an accepted byte suffix.
                can_read = live and all(output.size < FRAME_LIMIT for _, output in self._clients.values())
                watch(selector, self._master, (selectors.EVENT_READ if can_read else 0)
                      | (selectors.EVENT_WRITE if live and self._input else 0), "master")
                watch(selector, self._listener, selectors.EVENT_READ if live else 0, "listener")
                for client, (_, output) in self._clients.items():
                    watch(selector, client, (selectors.EVENT_READ if live and self._input.size < DATA_LIMIT else 0)
                          | (selectors.EVENT_WRITE if output else 0), client)
                for key, mask in selector.select():
                    if key.data == "master":
                        if mask & selectors.EVENT_WRITE:
                            try:
                                self._input.drain(lambda data: os.write(self._master, data))
                            except OSError:
                                live = False
                        if mask & selectors.EVENT_READ and live:
                            try:
                                data = os.read(self._master, 65536)
                            except BlockingIOError:
                                continue
                            except OSError:
                                data = b""
                            if not data:
                                live = False
                                continue
                            self._remember(data)
                            encoded = encode_output(data)
                            for _, output in self._clients.values():
                                output.append(encoded)
                    elif key.data == "listener" and live:
                        try:
                            client, _ = self._listener.accept()
                        except BlockingIOError:
                            continue
                        client.setblocking(False)
                        output = WriteQueue()
                        output.append(encode_output(self.replayed()))
                        self._clients[client] = (FrameReader(), output)
                    else:
                        client = key.data
                        if client not in self._clients:
                            continue
                        reader, output = self._clients[client]
                        try:
                            if mask & selectors.EVENT_WRITE:
                                output.drain(client.send)
                            if mask & selectors.EVENT_READ and live and self._input.size < DATA_LIMIT:
                                data = client.recv(65536)
                                if not data:
                                    reader.finish()
                                    self._drop(selector, client)
                                    continue
                                for kind, value in reader.feed(data):
                                    if kind == "input":
                                        self._input.append(value)
                                    else:
                                        rows, cols = value
                                        fcntl.ioctl(self._master, termios.TIOCSWINSZ,
                                                    struct.pack("HHHH", rows, cols, 0, 0))
                        except BlockingIOError:
                            continue
                        except (OSError, ValueError):
                            self._drop(selector, client)
        finally:
            for client in list(self._clients):
                self._drop(selector, client)
            selector.close()

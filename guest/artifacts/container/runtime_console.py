"""Own runc's actual workload terminal and reap its detached workload.

runc's detached console protocol transfers the PTY master over SCM_RIGHTS.
There is no outer terminal whose modes can buffer keys or translate output.
This module runs only in the dedicated Linux guest launcher.
"""

import array
import ctypes
import os
import socket
import struct
import subprocess
import time

import msgpack

# capsem-proto::stream; the cross-language fixtures prove these shared limits.
FRAME_LIMIT = 512 * 1024
DATA_LIMIT = 256 * 1024 - 1
FRAME_HEADER = struct.Struct(">I")


def frame(message):
    payload = msgpack.packb(message, use_bin_type=True)
    if not payload or len(payload) > FRAME_LIMIT:
        raise ValueError("console frame exceeds its budget")
    return FRAME_HEADER.pack(len(payload)) + payload


def encode_input(data):
    if not isinstance(data, bytes) or len(data) > DATA_LIMIT:
        raise ValueError("console input exceeds its binary data budget")
    return frame({"stdin": data})


def encode_resize(rows, cols):
    size(rows, cols)
    return frame({"control": {"type": "resize", "cols": cols, "rows": rows}})


def encode_output(data):
    return b"".join(frame({"stdout": data[offset:offset + DATA_LIMIT]})
                    for offset in range(0, len(data), DATA_LIMIT))


def size(rows, cols):
    if (type(rows) is not int or type(cols) is not int or not 1 <= rows <= 256
            or not 1 <= cols <= 512 or rows * cols > 65536):
        raise ValueError("console frame dimensions exceed the supported grid")


def event(payload, direction):
    message = msgpack.unpackb(payload, raw=False, strict_map_key=True,
                             max_bin_len=DATA_LIMIT, max_str_len=FRAME_LIMIT,
                             max_map_len=32, max_array_len=32, max_ext_len=0)
    if not isinstance(message, dict) or len(message) != 1:
        raise ValueError("console frame must be one named record")
    lane = "stdin" if direction == "client" else "stdout"
    if lane in message and isinstance(message[lane], bytes):
        return ("input" if direction == "client" else "output"), message[lane]
    control = message.get("control")
    if direction == "client" and isinstance(control, dict) and control.get("type") == "resize":
        rows, cols = control.get("rows"), control.get("cols")
        size(rows, cols)
        return "resize", (rows, cols)
    raise ValueError("console frame has the wrong type or direction")


class FrameReader:
    """Bounded u32-BE named MessagePack records, cut at any read boundary."""

    def __init__(self, direction="client"):
        self._buffer = bytearray()
        self._direction = direction

    def feed(self, data):
        self._buffer.extend(data)
        events = []
        while len(self._buffer) >= FRAME_HEADER.size:
            length, = FRAME_HEADER.unpack_from(self._buffer)
            if not 0 < length <= FRAME_LIMIT:
                raise ValueError("console frame exceeds its budget")
            end = FRAME_HEADER.size + length
            if len(self._buffer) < end:
                break
            payload = bytes(self._buffer[FRAME_HEADER.size:end])
            del self._buffer[:end]
            events.append(event(payload, self._direction))
        return events

    def finish(self):
        if self._buffer:
            raise ValueError("truncated console frame")


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

"""Trusted startup configuration and a bounded private guest-controller actor."""

import contextlib
import fcntl
import os
import re
import selectors
import struct
import subprocess
import time
import uuid

import msgpack

STATUS_LIMIT = 256  # capsem-proto::preparation
OUTPUT_LIMIT = 32 * 1024
HELPER = "/run/capsem-prepare"
FAILURES = {"invalid_input", "shell_unavailable", "spawn", "wait", "exit", "signal", "timed_out", "entrypoint_exec"}


def checked(startup):
    if (not isinstance(startup, dict) or set(startup) != {"script", "timeout_seconds"}
            or not isinstance(startup["script"], str) or "\0" in startup["script"]
            or len(startup["script"].encode()) > 65536
            or type(startup["timeout_seconds"]) is not int or not 0 < startup["timeout_seconds"] <= 2**32 - 1):
        raise ValueError("invalid approved startup inputs")
    return startup


def configure(config, startup):
    if startup is None:
        return config
    checked(startup)
    config["process"]["args"] = [HELPER, str(startup["timeout_seconds"]), startup["script"], "--", *config["process"]["args"]]
    config["mounts"].append({"destination": HELPER, "type": "bind", "source": "/run/capsem-sysutil",
                             "options": ["bind", "ro", "nosuid", "nodev"]})
    # A wrapper process is still preparing. Only its private result + exec EOF
    # lets the controller publish running, after required preparation succeeds.
    config["hooks"].pop("poststart", None)
    return config


def status_record(payload):
    value = msgpack.unpackb(payload, raw=False, strict_map_key=True, max_str_len=64,
                           max_map_len=3, max_array_len=0, max_bin_len=0, max_ext_len=0)
    if not isinstance(value, dict):
        raise ValueError("invalid startup status")
    kind = value.get("type")
    if not isinstance(kind, str):
        raise ValueError("invalid startup status type")
    if kind in {"preparing", "entrypoint"} and set(value) == {"type"}:
        return value
    failure = value.get("failure")
    if kind != "failed" or set(value) != {"type", "failure"} or not isinstance(failure, dict):
        raise ValueError("invalid startup status")
    reason = failure.get("kind")
    if not isinstance(reason, str) or reason not in FAILURES:
        raise ValueError("invalid startup failure")
    field = {"exit": "code", "signal": "signal"}.get(reason)
    if field:
        number = failure.get(field)
        if set(failure) != {"kind", field} or type(number) is not int or not 0 < number <= (255 if field == "code" else 64):
            raise ValueError("invalid startup failure result")
    elif set(failure) != {"kind"}:
        raise ValueError("invalid startup failure fields")
    return value


class Bridge:
    def __init__(self, stage, startup, kill, *, boot_timeout=30, clock=time.monotonic):
        self.startup, self.kill, self.clock = checked(startup), kill, clock
        self.deadline = clock() + boot_timeout
        with contextlib.ExitStack() as cleanup:
            self.stage = os.open(stage, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            cleanup.callback(os.close, self.stage)
            self.output, self.output_write = os.pipe()
            for fd in (self.output, self.output_write):
                cleanup.callback(os.close, fd)
            self.status, self.status_write = os.pipe()
            for fd in (self.status, self.status_write):
                cleanup.callback(os.close, fd)
            for fd in (self.output, self.status):
                os.set_blocking(fd, False)
            cleanup.pop_all()
        self.buffer, self.tail = bytearray(), bytearray()
        self.phase, self.failure, self.truncated = None, None, False
        self.selector = None
        self.remember = lambda _data: None
        self.can_output = lambda: True

    def marker(self, name, value):
        temporary = f".startup-{uuid.uuid4().hex}"
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=self.stage)
        try:
            with os.fdopen(fd, "wb") as output:
                output.write(value)
            os.rename(temporary, name, src_dir_fd=self.stage, dst_dir_fd=self.stage)
        finally:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(temporary, dir_fd=self.stage)

    def forget(self, name):
        with contextlib.suppress(FileNotFoundError):
            os.unlink(name, dir_fd=self.stage)

    def spawn(self, argv, **kwargs):
        # Only dup2/fcntl run after fork. The controller starts no worker
        # threads; all reads and deadlines belong to its selector actor.
        sources = []
        def inherit():
            for source, target in zip(sources, (3, 4), strict=True):
                os.dup2(source, target)
                os.set_inheritable(source, False)
        try:
            for fd in (self.output_write, self.status_write):
                sources.append(fcntl.fcntl(fd, fcntl.F_DUPFD_CLOEXEC, 10))
            return subprocess.Popen(argv, pass_fds=(3, 4, *sources), preexec_fn=inherit, **kwargs)
        finally:
            for fd in sources:
                os.close(fd)
            for name in ("output_write", "status_write"):
                self.close_fd(name)

    def close_fd(self, name):
        fd = getattr(self, name)
        if fd is not None:
            if self.selector is not None:
                with contextlib.suppress(KeyError, ValueError, OSError):
                    self.selector.unregister(fd)
            os.close(fd)
            setattr(self, name, None)

    def watch(self, selector):
        self.selector = selector
        for fd in (self.output, self.status):
            if fd is None:
                continue
            enabled = fd == self.status or self.can_output()
            try:
                selector.get_key(fd)
                present = True
            except KeyError:
                present = False
            if enabled and not present:
                selector.register(fd, selectors.EVENT_READ, self)
            elif not enabled and present:
                selector.unregister(fd)

    def delay(self):
        return max(0, self.deadline - self.clock()) if self.deadline is not None else None

    def check_deadline(self):
        if self.deadline is not None and self.clock() >= self.deadline:
            self.fail({"kind": "timed_out"})

    def output_ready(self):
        for _ in range(4):
            if not self.can_output():
                return
            try:
                data = os.read(self.output, 16384) if self.output is not None else b""
            except BlockingIOError:
                return
            if not data:
                self.close_fd("output")
                return
            self.remember(data)
            self.truncated |= len(self.tail) + len(data) > OUTPUT_LIMIT
            self.tail.extend(data)
            del self.tail[:-OUTPUT_LIMIT]

    def step(self):
        self.output_ready()
        if self.status is not None:
            try:
                data = os.read(self.status, STATUS_LIMIT + 4 - len(self.buffer))
            except BlockingIOError:
                data = None
            try:
                if data:
                    self.buffer.extend(data)
                    while len(self.buffer) >= 4:
                        length, = struct.unpack(">I", self.buffer[:4])
                        if not 0 < length <= STATUS_LIMIT:
                            raise ValueError("invalid startup frame length")
                        if len(self.buffer) < length + 4:
                            break
                        event = status_record(self.buffer[4:4 + length])
                        del self.buffer[:4 + length]
                        self.event(event)
                        if self.failure:
                            self.buffer.clear()
                            break
                elif data == b"":
                    self.close_fd("status")
                    if self.buffer or self.phase != "entrypoint" or self.failure:
                        if not self.failure:
                            self.fail({"kind": "wait"})
                    else:
                        self.deadline = None
                        self.forget("preparing")
                        self.marker("running", b"1\n")
            except (ValueError, msgpack.UnpackException):
                self.fail({"kind": "wait"})
        self.check_deadline()

    def event(self, event):
        kind = event["type"]
        if kind == "failed":
            self.fail(event["failure"])
        elif kind == "preparing" and self.phase is None:
            self.phase = kind
            self.deadline = self.clock() + self.startup["timeout_seconds"]
            self.marker("preparing", b"1\n")
        elif kind == "entrypoint" and self.phase == "preparing" and not self.failure:
            self.phase = kind
        else:
            raise ValueError("invalid startup status order")

    def fail(self, failure):
        if self.failure:
            return
        self.failure, self.deadline = failure, None
        self.close_fd("status")
        with contextlib.suppress(OSError, subprocess.SubprocessError):
            self.kill()
        self.output_ready()
        output = self.tail.decode("utf-8", "replace")
        output = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", output)
        output = "".join(char for char in output if char.isprintable() or char in "\n\t")
        encoded = output.encode()
        self.truncated |= len(encoded) > OUTPUT_LIMIT
        output = encoded[:OUTPUT_LIMIT].decode("utf-8", "ignore")
        result = failure["kind"]
        number = failure.get("code", failure.get("signal"))
        detail = f" ({number})" if number is not None else ""
        message = f"startup {result}{detail}\n{output}"
        if self.truncated:
            message += "\n[startup output truncated]"
        self.forget("preparing")
        self.marker("failed", message.encode())

    def wait(self, process):
        with selectors.DefaultSelector() as selector:
            self.watch(selector)
            while process.poll() is None:
                self.check_deadline()
                for _key, _mask in selector.select(min(0.05, self.delay() or 0.05)):
                    self.step()
            for _ in range(4):
                self.step()
        return process.wait()

    def close(self):
        for name in ("output", "status", "output_write", "status_write"):
            self.close_fd(name)
        os.close(self.stage)

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()

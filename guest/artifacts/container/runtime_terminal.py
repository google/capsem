"""Duplex attach transport; signal-driven resizes share the framed write queue."""

import collections
import contextlib
import os
import runpy
import selectors
from pathlib import Path

_wire = runpy.run_path(str(Path(__file__).with_name("runtime_console.py")))
FrameReader, WriteQueue, encode_input, encode_resize, watch, FRAME_LIMIT = (
    _wire[name] for name in ("FrameReader", "WriteQueue", "encode_input", "encode_resize", "watch", "FRAME_LIMIT")
)


class Relay:
    def __init__(self, console, keys, screen):
        self._console, self._keys, self._screen = console, keys, screen
        self._resizes = collections.deque(maxlen=1)
        self._wake_read, self._wake_write = os.pipe()
        os.set_blocking(self._wake_read, False)
        os.set_blocking(self._wake_write, False)
        self._closed = False

    def resize(self, rows, cols):
        # A Python signal may interrupt a socket write. It must never write
        # a second frame into the middle of the first. Only enqueue a size
        # and wake the actor; the newest unadmitted resize supersedes older ones.
        self._resizes.append((rows, cols))
        with contextlib.suppress(BlockingIOError):
            os.write(self._wake_write, b"r")

    def close(self):
        if not self._closed:
            self._closed = True
            os.close(self._wake_read)
            os.close(self._wake_write)

    def run(self):
        selector = selectors.DefaultSelector()
        reader = FrameReader("server")
        to_console, to_screen = WriteQueue(), WriteQueue()
        console, keys, screen = self._console, self._keys, self._screen
        flags = {fd: os.get_blocking(fd) for fd in (keys, screen)}
        timeout = console.gettimeout()
        live = input_open = True
        try:
            console.setblocking(False)
            for fd in flags:
                os.set_blocking(fd, False)
            watch(selector, self._wake_read, selectors.EVENT_READ, "resize")
            while live or to_screen:
                if self._resizes and to_console.size < FRAME_LIMIT:
                    to_console.append(encode_resize(*self._resizes.popleft()))
                watch(selector, console, (selectors.EVENT_READ if live and to_screen.size < FRAME_LIMIT else 0)
                      | (selectors.EVENT_WRITE if live and to_console else 0), "console")
                watch(selector, keys, selectors.EVENT_READ if live and input_open and to_console.size < FRAME_LIMIT else 0, "keys")
                watch(selector, screen, selectors.EVENT_WRITE if to_screen else 0, "screen")
                for key, mask in selector.select():
                    try:
                        if key.data == "console":
                            if mask & selectors.EVENT_WRITE:
                                to_console.drain(console.send)
                            if mask & selectors.EVENT_READ:
                                data = console.recv(65536)
                                if not data:
                                    reader.finish()
                                    live = False
                                    continue
                                for _, output in reader.feed(data):
                                    to_screen.append(output)
                        elif key.data == "keys":
                            data = os.read(keys, 65536)
                            if data:
                                to_console.append(encode_input(data))
                            else:
                                input_open = False
                        elif key.data == "screen":
                            to_screen.drain(lambda data: os.write(screen, data))
                        else:
                            os.read(self._wake_read, 65536)
                    except (BlockingIOError, InterruptedError):
                        continue
                    except BrokenPipeError:
                        return
        finally:
            selector.close()
            with contextlib.suppress(OSError):
                console.settimeout(timeout)
            for fd, blocking in flags.items():
                with contextlib.suppress(OSError):
                    os.set_blocking(fd, blocking)


def relay(console, keys, screen):
    transport = Relay(console, keys, screen)
    try:
        transport.run()
    finally:
        transport.close()

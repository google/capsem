"""Shared named MessagePack records and exact bounded-write cursors."""

import collections
import struct

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



class WriteQueue:
    def __init__(self):
        self._chunks = collections.deque()
        self._offset = 0
        self.size = 0

    def __bool__(self):
        return bool(self._chunks)

    def append(self, data):
        if data:
            self._chunks.append(data)
            self.size += len(data)

    def drain(self, write):
        if not self:
            return
        try:
            count = write(memoryview(self._chunks[0])[self._offset:])
        except (BlockingIOError, InterruptedError):
            return
        if count <= 0:
            raise BrokenPipeError("console stream stopped accepting bytes")
        self.size -= count
        self._offset += count
        if self._offset == len(self._chunks[0]):
            self._chunks.popleft()
            self._offset = 0


def watch(selector, file, events, data):
    try:
        key = selector.get_key(file)
    except KeyError:
        if events:
            selector.register(file, events, data)
    else:
        if not events:
            selector.unregister(file)
        elif key.events != events or key.data != data:
            selector.modify(file, events, data)

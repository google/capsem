"""Guest console framing conforms to the shared Rust MessagePack records."""

import json
import struct
from pathlib import Path

import msgpack
import pytest

from tests.fixtures.oci.test_launch_config import launcher

__all__ = ["launcher"]
GOLDEN = json.loads((Path(__file__).resolve().parents[3] / "sdk/specification/stream-v3.json").read_text())


def test_console_uses_the_rust_records_with_standard_byte_stream_framing(launcher):
    assert launcher._console["FRAME_LIMIT"] == GOLDEN["max_frame_bytes"]
    assert launcher._console["DATA_LIMIT"] == GOLDEN["max_data_bytes"]
    for entry in GOLDEN["client"]:
        if "stdin" in entry:
            encoded = launcher.encode_input(bytes(entry["stdin"]))
            expected = "input", bytes(entry["stdin"])
        elif entry["control"]["type"] == "resize":
            control = entry["control"]
            encoded = launcher.encode_resize(control["rows"], control["cols"])
            expected = "resize", (control["rows"], control["cols"])
        else:
            continue
        assert struct.unpack(">I", encoded[:4])[0] == len(encoded) - 4
        assert msgpack.unpackb(encoded[4:]) == msgpack.unpackb(bytes(entry["frame"]))
        wire = struct.pack(">I", len(entry["frame"])) + bytes(entry["frame"])
        reader = launcher.FrameReader()
        assert [event for byte in wire for event in reader.feed(bytes([byte]))] == [expected]
        reader.finish()
    output = b"\x00\xff\x1b[2J" * 100000
    reader = launcher.FrameReader("server")
    assert b"".join(data for _, data in reader.feed(launcher.encode_output(output))) == output
    reader.finish()


@pytest.mark.parametrize("value", [
    {"stdout": b"wrong direction"}, {"stdin": [0, 255]}, {"stdin": "text"},
    {"stdin": b"x", "control": {"type": "resize", "cols": 80, "rows": 24}},
    {"control": {"type": "resize", "cols": True, "rows": 24}},
    {"control": {"type": "resize", "cols": 512, "rows": 256}},
])
def test_console_refuses_nonbinary_wrong_direction_and_invalid_grid(launcher, value):
    payload = msgpack.packb(value, use_bin_type=True)
    with pytest.raises(ValueError):
        launcher.FrameReader().feed(struct.pack(">I", len(payload)) + payload)


def test_console_refuses_truncated_oversized_and_trailing_records(launcher):
    reader = launcher.FrameReader()
    reader.feed(launcher.encode_input(b"x")[:-1])
    with pytest.raises(ValueError, match="truncated"):
        reader.finish()
    for length in (0, GOLDEN["max_frame_bytes"] + 1):
        with pytest.raises(ValueError):
            launcher.FrameReader().feed(struct.pack(">I", length))
    payload = msgpack.packb({"stdin": b"x"}, use_bin_type=True) + b"\x00"
    with pytest.raises(ValueError):
        launcher.FrameReader().feed(struct.pack(">I", len(payload)) + payload)

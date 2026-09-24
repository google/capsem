"""The test-side MessagePack decoder reads what capsem_proto::forensic writes."""

from __future__ import annotations

import re
import struct
from pathlib import Path

import pytest
from helpers.body_archive import FORENSIC_LIST_FIELDS, FORENSIC_OPTION_FIELDS, forensic_payload
from helpers.messagepack import MessagePackError, decode


def test_a_sparse_forensic_payload_decodes_to_its_json_shape() -> None:
    # {"event_type": "mcp.request", "mcp": {"method": "tools/call", "ids": [1, -3]}}
    payload = (
        b"\x82"
        b"\xaaevent_type\xabmcp.request"
        b"\xa3mcp\x82\xa6method\xaatools/call\xa3ids\x92\x01\xfd"
    )
    assert decode(payload) == {"event_type": "mcp.request", "mcp": {"method": "tools/call", "ids": [1, -3]}}


@pytest.mark.parametrize(
    ("encoded", "value"),
    [
        (b"\xc0", None),
        (b"\xc2", False),
        (b"\xc3", True),
        (b"\x7f", 127),
        (b"\xe0", -32),
        (b"\xcc\xff", 255),
        (b"\xcd\x01\x00", 256),
        (b"\xce\x00\x01\x00\x00", 65536),
        (b"\xcf" + (2**40).to_bytes(8, "big"), 2**40),
        (b"\xd0\x80", -128),
        (b"\xd1\x80\x00", -32768),
        (b"\xd2\x80\x00\x00\x00", -(2**31)),
        (b"\xd3" + (-(2**40)).to_bytes(8, "big", signed=True), -(2**40)),
        (b"\xcb" + struct.pack(">d", 1.5), 1.5),
        (b"\xca" + struct.pack(">f", 0.5), 0.5),
        (b"\xd9\x03abc", "abc"),
        (b"\xda\x00\x02\xc3\xa9", "é"),
        (b"\xc4\x02\x00\xff", b"\x00\xff"),
        (b"\xdc\x00\x02\x01\x02", [1, 2]),
        (b"\xde\x00\x01\xa1k\xa1v", {"k": "v"}),
    ],
)
def test_every_marker_in_the_subset(encoded: bytes, value: object) -> None:
    assert decode(encoded) == value


@pytest.mark.parametrize(
    "broken",
    [
        b"",  # nothing
        b"\xa3ab",  # string shorter than its length
        b"\x92\x01",  # array missing an item
        b"\xc0\xc0",  # trailing value
        b"\xd4\x01\x00",  # fixext: not in the subset
    ],
)
def test_a_broken_payload_raises_instead_of_guessing(broken: bytes) -> None:
    with pytest.raises(MessagePackError):
        decode(broken)


def test_the_helper_restores_exactly_the_rust_sparse_defaults() -> None:
    """The helper's defaults match `SecurityForensicEvent`, field for field."""
    source = (Path(__file__).resolve().parents[1] / "crates/capsem-proto/src/forensic.rs").read_text()
    struct_body = source.split("pub struct SecurityForensicEvent {", 1)[1].split("\n}", 1)[0]
    fields = re.findall(r"pub (\w+): (Vec|Option)<", struct_body)
    assert {name for name, kind in fields if kind == "Vec"} == set(FORENSIC_LIST_FIELDS)
    assert {name for name, kind in fields if kind == "Option"} == set(FORENSIC_OPTION_FIELDS)

    payload = forensic_payload(b"\x81\xaaevent_type\xa3dns")
    assert payload["event_type"] == "dns"
    assert payload["detections"] == []
    assert payload["decision"] is None

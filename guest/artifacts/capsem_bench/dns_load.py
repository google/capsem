"""dns-load: concurrency-driven load test against the capsem DNS proxy.

Measures DNS resolution rps + tail latency through the full path:

  guest libc / our wire-format query
    -> 127.0.0.1:53 (UDP)
    -> iptables nat redirect to 127.0.0.1:1053
    -> capsem-dns-proxy (T3.2)
    -> vsock 5007 framed envelope
    -> capsem-process serve_dns_session (T3.2 + T3.3)
    -> DnsHandler::handle (T3.1 / T3.d)
    -> SecurityRuleSet evaluation OR find_dns_redirect OR
       UdpSocket forward to 1.1.1.1:53
    -> response wire bytes back over the same path

Output schema mirrors mitm-load so post-T5 regression checks can
share machinery:

  {
    "version": "1.0",
    "qname": "api.openai.com",
    "qtype": 1,
    "concurrency_levels": [
      {
        "concurrency": 1,
        "duration_s": 10.0,
        "total_requests": 1234,
        "errors": 0,
        "rps": 123.4,
        "p50_ms": 1.2,
        "p95_ms": 3.4,
        "p99_ms": 5.6,
        "p999_ms": 12.0,
        "rss_peak_mb": 12.0,
        "decision_distribution": {"denied": 1234}
      },
      ...
    ]
  }

Default qname is `api.openai.com` so the benchmark exercises the
security-rule evaluation path. Override via `CAPSEM_BENCH_DNS_QNAME`
to benchmark another domain or the upstream-forward path (e.g. `elie.net`).
"""

import os
import random
import socket
import struct
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

from .load_harness import (
    DurationLoadConfig,
    render_load_table,
    summarize_load_level,
)

# `rich` and `.helpers` are imported lazily inside `dns_load_bench`
# so the encoder helpers + their unittest module-level tests can run
# host-side via `python3 -m unittest` without needing rich installed
# on the developer machine. The benches themselves only run inside
# the guest where rich + the helpers package are available.

def _percentile(sorted_values, q):
    """Return the q-th percentile (q in [0, 100]) from a pre-sorted list.
    Local copy so this module doesn't need `.helpers` until the bench
    entry point itself runs (keeps `python3 -m unittest` host-friendly).
    The `.helpers.percentile` shape is identical; we don't import
    that one here to avoid a transitive `rich` import.
    """
    if not sorted_values:
        return 0.0
    k = (len(sorted_values) - 1) * (q / 100.0)
    f = int(k)
    c = min(f + 1, len(sorted_values) - 1)
    return sorted_values[f] + (sorted_values[c] - sorted_values[f]) * (k - f)


DEFAULT_QNAME = "api.openai.com"
DEFAULT_QTYPE = 1  # A
DEFAULT_CONCURRENCY = (1, 10, 50, 200)
DEFAULT_DURATION_S = 10.0
DEFAULT_TIMEOUT_S = 5.0


def _encode_qname(name: str) -> bytes:
    """Encode a dotted DNS name as length-prefixed labels + root."""
    out = bytearray()
    for label in name.split("."):
        if not label:
            continue
        if len(label) > 63:
            raise ValueError(f"label too long: {label!r}")
        out.append(len(label))
        out.extend(label.encode("ascii"))
    out.append(0)  # root
    return bytes(out)


def _build_query(qname: str, qtype: int, qid: int) -> bytes:
    """Build a wire-format DNS query for `qname` qtype `qtype`.

    Standard query, RD=1, qdcount=1, no answer/authority/additional.
    """
    header = struct.pack(
        ">HHHHHH",
        qid,
        0x0100,  # flags: standard query, RD=1
        1,  # qdcount
        0,  # ancount
        0,  # nscount
        0,  # arcount
    )
    qname_bytes = _encode_qname(qname)
    question = qname_bytes + struct.pack(">HH", qtype, 1)  # qtype + IN
    return header + question


def _decode_rcode(resp: bytes) -> int:
    """RFC 1035 sec 4.1.1: rcode is the low 4 bits of byte 3."""
    if len(resp) < 4:
        return 2  # ServFail-shaped on a truncated reply
    return resp[3] & 0x0F


# Map DNS rcode -> the matching capsem `Decision::as_str` shape so
# the per-level summary distribution lines up with what shows up in
# `dns_events.decision`.
_RCODE_DECISION = {
    0: "allowed",  # NoError -- forwarded successfully OR redirect
    2: "error",  # ServFail -- upstream unreachable
    3: "denied",  # NXDomain -- policy block
}


def _do_query(qname: str, qtype: int, timeout_s: float):
    """Single UDP DNS query; returns (latency_ms, rcode, error)."""
    qid = random.randint(0, 0xFFFF)
    pkt = _build_query(qname, qtype, qid)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(timeout_s)
    start = time.monotonic()
    try:
        # iptables nat redirects :53 -> :1053; libc would resolve via
        # /etc/resolv.conf -> 127.0.0.1, hitting the same path. We
        # bypass libc to keep the timing tight + avoid getaddrinfo
        # caching.
        sock.sendto(pkt, ("127.0.0.1", 53))
        resp, _peer = sock.recvfrom(4096)
        elapsed_ms = (time.monotonic() - start) * 1000
        # Sanity: response id must match query id.
        if len(resp) >= 2 and (resp[0] << 8 | resp[1]) != qid:
            return (elapsed_ms, 0, f"id mismatch: query=0x{qid:04x}")
        return (elapsed_ms, _decode_rcode(resp), None)
    except socket.timeout:
        elapsed_ms = (time.monotonic() - start) * 1000
        return (elapsed_ms, 0, "timeout")
    except OSError as exc:
        elapsed_ms = (time.monotonic() - start) * 1000
        return (elapsed_ms, 0, str(exc))
    finally:
        sock.close()


def _drive_at_concurrency(qname, qtype, concurrency, duration_s, timeout_s):
    """Spawn `concurrency` workers each looping for `duration_s`.

    Each worker holds its own UDP socket per query (DNS is
    connectionless; pooling a socket would be safe but it adds no
    real-world signal). Returns the flat list of (latency_ms, rcode,
    error) tuples.
    """
    deadline = time.monotonic() + duration_s

    def worker():
        out = []
        while time.monotonic() < deadline:
            out.append(_do_query(qname, qtype, timeout_s))
        return out

    all_results = []
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(worker) for _ in range(concurrency)]
        for fut in as_completed(futures):
            all_results.extend(fut.result())
    return all_results


def _summarize(results, concurrency, duration_s):
    latencies = [r[0] for r in results]
    errors = sum(1 for r in results if r[2] is not None)
    decisions = {}
    for _lat, rcode, err in results:
        if err is not None:
            decisions["transport_error"] = decisions.get("transport_error", 0) + 1
        else:
            label = _RCODE_DECISION.get(rcode, f"rcode_{rcode}")
            decisions[label] = decisions.get(label, 0) + 1
    return summarize_load_level(
        latencies,
        errors,
        concurrency,
        duration_s,
        extra={"decision_distribution": decisions},
    )


def dns_load_bench(qname=None, qtype=None, concurrency_levels=None, duration_s=None):
    """Drive the DNS proxy at each concurrency level; return result dict."""
    # Lazy imports -- only the bench entry point needs rich + helpers.
    # Keeps `python3 -m unittest dns_load` working host-side.
    from .helpers import console

    qname = qname or os.environ.get("CAPSEM_BENCH_DNS_QNAME", DEFAULT_QNAME)
    qtype = qtype or int(os.environ.get("CAPSEM_BENCH_DNS_QTYPE", DEFAULT_QTYPE))
    config = DurationLoadConfig.from_inputs(
        "dns-load",
        default_concurrency=DEFAULT_CONCURRENCY,
        default_duration_s=DEFAULT_DURATION_S,
        concurrency_levels=concurrency_levels,
        duration_s=duration_s,
    )
    timeout_s = float(
        os.environ.get("CAPSEM_BENCH_DNS_TIMEOUT", DEFAULT_TIMEOUT_S)
    )

    console.print(
        f"[bold]dns-load[/bold] qname={qname} qtype={qtype} "
        f"duration={config.duration_s}s "
        f"concurrency={','.join(str(c) for c in config.concurrency_levels)}"
    )

    rows = []
    for c in config.concurrency_levels:
        console.print(f"  concurrency={c} ...")
        results = _drive_at_concurrency(qname, qtype, c, config.duration_s, timeout_s)
        rows.append(_summarize(results, c, config.duration_s))

    out = {
        "version": "1.0",
        "qname": qname,
        "qtype": qtype,
        "concurrency_levels": rows,
    }

    render_load_table(
        f"dns-load (qname={qname}, qtype={qtype}, {config.duration_s}s per level)",
        rows,
        extra_columns=[
            ("decisions", lambda row: ",".join(
                f"{k}={v}" for k, v in row["decision_distribution"].items()
            )),
        ],
    )

    return out


# -------------------------------------------------------------------
# Self-tests (host-side; do not require a guest kernel).
#
# Run via:
#   python -m unittest guest.artifacts.capsem_bench.dns_load
# -------------------------------------------------------------------

import unittest


class DnsLoadEncodingTests(unittest.TestCase):
    def test_encode_qname_simple(self):
        self.assertEqual(
            _encode_qname("anthropic.com"),
            b"\x09anthropic\x03com\x00",
        )

    def test_encode_qname_strips_trailing_dot(self):
        # Trailing dot produces an empty label; we skip empties so
        # "example.com." encodes the same as "example.com".
        self.assertEqual(
            _encode_qname("example.com."),
            _encode_qname("example.com"),
        )

    def test_encode_qname_rejects_oversize_label(self):
        with self.assertRaises(ValueError):
            _encode_qname("a" * 64 + ".com")

    def test_build_query_header_shape(self):
        pkt = _build_query("anthropic.com", 1, 0x1234)
        # 12-byte header: id, flags, qdcount=1, an/ns/ar=0
        self.assertEqual(pkt[:2], b"\x12\x34")
        self.assertEqual(pkt[2:4], b"\x01\x00")  # standard query, RD
        self.assertEqual(pkt[4:6], b"\x00\x01")  # qdcount=1
        self.assertEqual(pkt[6:8], b"\x00\x00")  # ancount=0
        self.assertEqual(pkt[8:10], b"\x00\x00")  # nscount=0
        self.assertEqual(pkt[10:12], b"\x00\x00")  # arcount=0
        # Question section: qname + qtype=1 + qclass=1
        self.assertEqual(pkt[-4:], b"\x00\x01\x00\x01")

    def test_decode_rcode_nxdomain(self):
        # Build a fake response with rcode=3 (NXDomain).
        # Byte 3 low 4 bits = rcode.
        resp = b"\x12\x34\x81\x83" + b"\x00" * 8
        self.assertEqual(_decode_rcode(resp), 3)

    def test_decode_rcode_truncated_returns_servfail_shape(self):
        self.assertEqual(_decode_rcode(b"\x00\x00"), 2)

    def test_rcode_decision_mapping(self):
        # Pinned so this stays in lock-step with capsem_logger::Decision.
        self.assertEqual(_RCODE_DECISION[0], "allowed")
        self.assertEqual(_RCODE_DECISION[2], "error")
        self.assertEqual(_RCODE_DECISION[3], "denied")

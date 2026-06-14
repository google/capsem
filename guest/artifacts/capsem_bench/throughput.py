"""Proxy throughput benchmark through the MITM proxy."""

import subprocess

from rich.table import Table
from rich.text import Text

from .helpers import (
    LOCAL_MOCK_SERVER_ENV,
    console,
    fmt_bytes,
    local_mock_server_url,
    public_network_allowed,
)

# cdn.elie.net 301-redirects to elie.net, so curl runs with -L and both hosts
# appear in net_events.
PUBLIC_THROUGHPUT_URL = "https://cdn.elie.net/static/files/i-am-a-legend/i-am-a-legend-slides.pdf"
PUBLIC_THROUGHPUT_DOMAIN = "cdn.elie.net"
# Conservative floor; the PDF is ~9.5 MB today but may drift on re-publish.
PUBLIC_THROUGHPUT_EXPECTED_BYTES = 9 * 1024 * 1024
LOCAL_THROUGHPUT_PATH = "/bytes/10mb"
LOCAL_THROUGHPUT_EXPECTED_BYTES = 10 * 1024 * 1024


def throughput_bench():
    """Download deterministic bytes through the MITM proxy and report throughput."""
    target = _throughput_target()
    if target is None:
        stats = {
            "skipped": True,
            "reason": (
                f"set {LOCAL_MOCK_SERVER_ENV} for local lab or "
                "CAPSEM_BENCH_ALLOW_PUBLIC_NETWORK=1 for explicit public smoke"
            ),
        }
        table = Table(title=Text("Proxy Throughput"))
        table.add_column("Metric", style="bold")
        table.add_column("Value", justify="right")
        table.add_row("Skipped", stats["reason"])
        console.print(table)
        return stats

    url, expected_bytes, source = target
    table = Table(title=Text(f"Proxy Throughput  [{url}]"))
    table.add_column("Metric", style="bold")
    table.add_column("Value", justify="right")

    result = subprocess.run(
        [
            "curl", "-sL", "-o", "/dev/null",
            "-w", "%{http_code} %{speed_download} %{size_download} %{time_total}",
            "--connect-timeout", "15",
            url,
        ],
        capture_output=True,
        text=True,
        timeout=180,
    )

    if result.returncode != 0:
        stats = {"error": f"curl failed (exit {result.returncode}): {result.stderr.strip()}"}
        table.add_row("Error", stats["error"])
        console.print(table)
        return stats

    parts = result.stdout.strip().split()
    if len(parts) != 4:
        stats = {"error": f"unexpected curl output: {result.stdout!r}"}
        table.add_row("Error", stats["error"])
        console.print(table)
        return stats

    http_code = int(parts[0])
    speed_bps = float(parts[1])
    size_bytes = int(parts[2])
    time_s = float(parts[3])
    speed_mbps = round(speed_bps / (1024 * 1024), 2)

    if http_code != 200:
        stats = {"error": f"HTTP {http_code} (domain may not be in allow list)"}
        table.add_row("HTTP status", str(http_code))
        table.add_row("Error", stats["error"])
        console.print(table)
        return stats

    stats = {
        "url": url,
        "source": source,
        "http_code": http_code,
        "size_bytes": size_bytes,
        "duration_s": round(time_s, 3),
        "throughput_mbps": speed_mbps,
    }

    table.add_row("URL", url)
    table.add_row("Downloaded", fmt_bytes(size_bytes))
    table.add_row("Duration", f"{time_s:.2f}s")
    table.add_row("Throughput", f"{speed_mbps} MB/s")

    if size_bytes < expected_bytes:
        table.add_row("Warning", f"incomplete: expected {fmt_bytes(expected_bytes)}")

    console.print(table)
    return stats


def _throughput_target():
    local_url = local_mock_server_url(LOCAL_THROUGHPUT_PATH)
    if local_url:
        return (local_url, LOCAL_THROUGHPUT_EXPECTED_BYTES, "local")
    if public_network_allowed():
        return (PUBLIC_THROUGHPUT_URL, PUBLIC_THROUGHPUT_EXPECTED_BYTES, "public")
    return None

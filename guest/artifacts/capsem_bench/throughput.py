"""Proxy throughput benchmark (~10 MB PDF download through MITM proxy)."""

import subprocess

from rich.table import Table
from rich.text import Text

from .helpers import console, fmt_bytes

# cdn.elie.net 301-redirects to elie.net, so curl runs with -L and both hosts
# appear in net_events.
THROUGHPUT_URL = "https://cdn.elie.net/static/files/i-am-a-legend/i-am-a-legend-slides.pdf"
THROUGHPUT_DOMAIN = "cdn.elie.net"
# Conservative floor; the PDF is ~9.5 MB today but may drift on re-publish.
THROUGHPUT_EXPECTED_BYTES = 9 * 1024 * 1024


def throughput_bench():
    """Download a ~10 MB PDF through the MITM proxy and report end-to-end throughput."""
    table = Table(title=Text(f"Proxy Throughput  [{THROUGHPUT_URL}]"))
    table.add_column("Metric", style="bold")
    table.add_column("Value", justify="right")

    result = subprocess.run(
        [
            "curl", "-sL", "-o", "/dev/null",
            "-w", "%{http_code} %{speed_download} %{size_download} %{time_total}",
            "--connect-timeout", "15",
            THROUGHPUT_URL,
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
        "url": THROUGHPUT_URL,
        "http_code": http_code,
        "size_bytes": size_bytes,
        "duration_s": round(time_s, 3),
        "throughput_mbps": speed_mbps,
    }

    table.add_row("URL", THROUGHPUT_URL)
    table.add_row("Downloaded", fmt_bytes(size_bytes))
    table.add_row("Duration", f"{time_s:.2f}s")
    table.add_row("Throughput", f"{speed_mbps} MB/s")

    if size_bytes < THROUGHPUT_EXPECTED_BYTES:
        table.add_row("Warning", f"incomplete: expected {fmt_bytes(THROUGHPUT_EXPECTED_BYTES)}")

    console.print(table)
    return stats

"""Map existing recorded results into functional chart data without running benchmarks."""

from __future__ import annotations

import math

SECTIONS = {
    "images": ("Image performance", "Boot readiness, fork and prepared-image size."),
    "local": ("Local network performance", "VM-to-VM private network latency and throughput."),
    "external": (
        "External network path",
        "Controlled HTTP fixtures exercise the request path; these are not Internet speed tests.",
    ),
    "io": ("Hypervisor I/O", "Sequential and random reads and writes inside the guest."),
    "resources": (
        "Hypervisor CPU and memory",
        "Loaded CPU, memory and VM-count scaling require matching recorded resource samples.",
    ),
    "logging": (
        "Logging and query performance",
        "Service reads during concurrent mutation; distinct from event ingestion and flush throughput.",
    ),
    "security": (
        "Security inspection",
        "Model and credential response paths; distinct from isolated plugin, reconstruction or rule evaluation cost.",
    ),
}


def _eligible(source: dict) -> None:
    classification = str(source.get("classification", "")).lower()
    if (
        source.get("quick")
        or source.get("doctor_exit", 0)
        or any(
            word in classification for word in ["exploratory", "diagnostic", "prototype", "spike"]
        )
    ):
        raise ValueError("Benchmark snapshot is not eligible for published performance charts")
    commit = source.get("source_commit", "")
    if len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
        raise ValueError("Benchmark source must identify a committed source")


def normalize(snapshot: dict) -> dict:
    source, records = snapshot["source"], snapshot["records"]
    _eligible(source)
    sections = {
        key: {"title": title, "description": desc, "charts": [], "available": False}
        for key, (title, desc) in SECTIONS.items()
    }

    def chart(section, identifier, title, unit, labels, series, note=""):
        for item in series:
            if len(item["values"]) != len(labels):
                raise ValueError("Chart labels and measurements differ in length")
            if any(not math.isfinite(float(v)) for v in item["values"]):
                raise ValueError("Benchmark measurements must be finite")
        sections[section]["charts"].append(
            {
                "id": identifier,
                "title": title,
                "unit": unit,
                "labels": labels,
                "series": series,
                "note": note,
            }
        )
        sections[section]["available"] = True

    def series(name, values):
        return {"name": name, "values": values}

    lifecycle = [(name, value) for name, value in records.items() if name.startswith("lifecycle/")]
    for key, title in [
        ("exec_ready_ms", "Create to exec-ready"),
        ("provision_ms", "VM provisioning"),
    ]:
        available = [
            (d.get("profile", n), d["operations"][key])
            for n, d in lifecycle
            if key in d.get("operations", {})
        ]
        if available:
            chart(
                "images",
                key,
                title,
                "ms",
                [p for p, _ in available],
                [series("Mean", [s["mean"] for _, s in available])],
                "Three recorded trials per image profile. See the downloadable records for their observed ranges.",
            )
    forks = [(d.get("profile", n), d["fork"]) for n, d in records.items() if n.startswith("fork/")]
    for key, title, unit in [
        ("fork_ms", "Fork a prepared image", "ms"),
        ("boot_ready_ms", "Boot a forked image to readiness", "ms"),
        ("image_size_mb", "Forked image size", "MiB"),
    ]:
        available = [(p, d[key]) for p, d in forks if key in d]
        if available:
            chart(
                "images",
                key,
                title,
                unit,
                [p for p, _ in available],
                [series("Mean", [s["mean"] for _, s in available])],
            )

    guest = next((d for n, d in records.items() if n.startswith("capsem-bench/")), {})
    disk = guest.get("disk", {})
    if "seq_read" in disk and "seq_write" in disk:
        chart(
            "io",
            "disk-sequential",
            "Sequential disk throughput",
            "MiB/s",
            ["Read", "Write"],
            [series("Guest disk", [disk[k]["throughput_mbps"] for k in ["seq_read", "seq_write"]])],
        )
    if "rand_read_4k" in disk and "rand_write_4k" in disk:
        chart(
            "io",
            "disk-random",
            "Random 4 KiB disk I/O",
            "IOPS",
            ["Read", "Write"],
            [series("Guest disk", [disk[k]["iops"] for k in ["rand_read_4k", "rand_write_4k"]])],
        )
    rootfs = guest.get("rootfs", {}).get("large_binary_seq_read", {})
    if "cold_throughput_mbps" in rootfs and "warm_throughput_mbps" in rootfs:
        chart(
            "io",
            "rootfs-cache",
            "Large-binary read cache effects",
            "MiB/s",
            ["Cold", "Warm"],
            [
                series(
                    "Root filesystem",
                    [rootfs[k] for k in ["cold_throughput_mbps", "warm_throughput_mbps"]],
                )
            ],
        )
    http = guest.get("http", {})
    if "latency_ms" in http:
        chart(
            "external",
            "http-latency",
            "HTTP latency through the controlled fixture",
            "ms",
            ["p50", "p95", "p99"],
            [series("Latency", [http["latency_ms"][k] for k in ["p50", "p95", "p99"]])],
            f"{http.get('successful', 0):,} successful requests; {http.get('failed', 0):,} failures. Fixture source: {http.get('source', 'recorded')}.",
        )
    startup = guest.get("startup", {}).get("commands", {})
    if startup:
        chart(
            "images",
            "command-startup",
            "Developer-tool process launch",
            "ms",
            list(startup),
            [series("Mean", [r["mean_ms"] for r in startup.values()])],
            "Commands use --version: these timings measure process launch, not model inference readiness.",
        )
    host = next((d for n, d in records.items() if n.startswith("mock-server-protocol/")), {})
    lanes = [d.get("mock_server_protocol", {}) for d in [host, guest]]
    if all(d.get("scenarios") for d in lanes):
        if any(lanes[0].get(k) != lanes[1].get(k) for k in ["total_requests", "concurrency"]):
            raise ValueError("Host and guest benchmark workloads do not match")
        names = [s["name"] for s in lanes[0]["scenarios"]]
        matched = [{s["name"]: s for s in d["scenarios"]} for d in lanes]
        if all(set(d) == set(names) for d in matched):
            for key, title, unit in [
                ("requests_per_sec", "Model and credential response throughput", "requests/s"),
                ("p95", "Model and credential response p95 latency", "ms"),
            ]:
                values = [
                    [
                        d[n][key] if key == "requests_per_sec" else d[n]["latency_ms"][key]
                        for n in names
                    ]
                    for d in matched
                ]
                chart(
                    "security",
                    key,
                    title,
                    unit,
                    [n.replace("_", " ") for n in names],
                    [
                        series(label, v)
                        for label, v in zip(["Host direct", "Through Capsem"], values, strict=True)
                    ],
                    "End-to-end observed paths, not isolated reconstruction, plugin or rule cost. Compatible payloads and concurrency are required for comparison.",
                )
    route = next((d for n, d in records.items() if n.startswith("route-latency/")), {})
    summary = route.get("reader", {}).get("summary", {})
    if all(k in summary for k in ["p50_ms", "p95_ms", "p99_ms"]):
        chart(
            "logging",
            "stats-reads",
            "Stats reads during profile mutation",
            "ms",
            ["p50", "p95", "p99"],
            [series("Service UDS", [summary[k] for k in ["p50_ms", "p95_ms", "p99_ms"]])],
            f"{summary.get('samples', 0)} samples. Read/API latency; not an ingestion or flush benchmark.",
        )
    return {
        "release": source["release_label"],
        "architecture": source["architecture"],
        "recordedAt": source["artifact_created_at_utc"],
        "commit": source["source_commit"],
        "sourceUrl": source["workflow_run_url"],
        "sections": sections,
    }

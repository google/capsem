"""Validate original network samples; statistics remain the Rust owner."""

from __future__ import annotations

import json
import math

from capsem_builder.gate.benchmarkschema import BenchmarkRunConfig


def counter(value):
    if type(value) is not int or not 0 <= value <= 2**53:
        raise ValueError("network counter must be an exact nonnegative f64 integer")
    return value


class NetworkObservations:
    def __init__(self, limits: BenchmarkRunConfig) -> None:
        self.limits = limits
        self.metrics: dict = {}
        self.windows: list = []
        self._samples = self._window_bytes = 0

    def add(self, key, unit, value):
        if unit in {"count", "bytes"}:
            counter(value)
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ValueError("invalid original network sample")
        metric = self.metrics.setdefault(key, {"unit": unit, "samples": []})
        if metric["unit"] != unit:
            raise ValueError("network sample unit changed")
        if (
            len(metric["samples"]) >= self.limits.max_samples_per_metric
            or self._samples >= self.limits.max_samples_per_run
        ):
            raise ValueError("network sample budget exceeded")
        metric["samples"].append(value)
        self._samples += 1

    def window(self, lane, trial, receipt, *, guest_tcp=None, producer=None):
        window = {"lane": lane, "trial": trial, "host": receipt}
        if guest_tcp is not None:
            window["guest_tcp_retransmits"] = guest_tcp
        if producer is not None:
            window["producer"] = producer
        # Count before retaining; iterator avoids a second retained encoded tree.
        size = 0
        for part in json.JSONEncoder(allow_nan=False).iterencode(window):
            size += len(part.encode("utf-8"))
            if size > self.limits.max_collector_bytes - self._window_bytes:
                raise ValueError("network resource receipt budget exceeded")
        self._window_bytes += size
        self.windows.append(window)

    def _values(self, metrics, key, unit, *, scalar=True, integral=False):
        value = metrics.get(key)
        if not isinstance(value, dict) or value.get("unit") != unit:
            raise ValueError("missing or invalid original network metric: " + key)
        samples = value.get("samples")
        if not isinstance(samples, list) or not samples or (scalar and len(samples) != 1):
            raise ValueError("incomplete original network samples: " + key)
        if len(samples) > self.limits.max_samples_per_metric:
            raise ValueError("network raw sample budget exceeded")
        for sample in samples:
            if type(sample) not in (int, float) or not math.isfinite(sample) or sample < 0:
                raise ValueError("invalid original network metric: " + key)
            if integral:
                counter(sample)
        return samples

    def tcp(self, lane, report, *, direction, streams, seconds):
        declared = report.get("throughput", {})
        if any(type(declared.get(key)) is not type(value) or declared.get(key) != value for key, value in (
            ("direction", direction), ("streams", streams), ("seconds", seconds)
        )):
            raise ValueError("original throughput invocation differs")
        metrics = report["metrics"]
        facts = {}
        for key, unit in (
            ("bytes_sent", "bytes"), ("bytes_received", "bytes"),
            ("elapsed_seconds", "seconds"), ("streams", "count"),
            ("send_megabits_per_sec", "megabits_per_second"),
            ("receive_megabits_per_sec", "megabits_per_second"),
        ):
            value = self._values(metrics, key, unit, integral=key in {"bytes_sent", "bytes_received", "streams"})[0]
            if key == "streams" and value != streams:
                raise ValueError("original throughput stream count differs")
            if key == "elapsed_seconds" and value <= 0:
                raise ValueError("original throughput elapsed time is empty")
            if key in {"bytes_sent", "bytes_received", "streams"}:
                facts[key] = value
            elif key == "elapsed_seconds":
                self.add(lane + "." + key, unit, value)
        selected = "send_megabits_per_sec" if direction == "upload" else "receive_megabits_per_sec"
        if direction != "latency":
            value = metrics[selected]["samples"][0]
            moved = metrics["bytes_sent" if direction == "upload" else "bytes_received"]["samples"][0]
            if value <= 0 or moved <= 0:
                raise ValueError("original throughput moved no selected bytes")
            self.add(lane + ".megabits_per_sec", "megabits_per_second", value)
        else:
            for value in self._values(metrics, "round_trip_ms", "milliseconds", scalar=False):
                self.add("tcp.round_trip_ms", "milliseconds", value)
        return facts

    def udp(self, report, *, count):
        if report.get("udp", {}).get("count") != count:
            raise ValueError("original UDP invocation count differs")
        metrics = report["metrics"]
        counts = {key: self._values(metrics, key, "count", integral=True)[0]
                  for key in ("sent", "received", "lost")}
        if counts["sent"] != count or counts["received"] + counts["lost"] != count:
            raise ValueError("original UDP accounting differs")
        latencies = self._values(metrics, "round_trip_ms", "milliseconds", scalar=False)
        if len(latencies) != counts["received"]:
            raise ValueError("original UDP latency sample count differs")
        self.add("udp.lost", "count", counts["lost"])
        elapsed = self._values(metrics, "elapsed_seconds", "seconds")[0]
        if elapsed <= 0:
            raise ValueError("original UDP elapsed time is empty")
        self.add("udp.elapsed_seconds", "seconds", elapsed)
        for value in latencies:
            self.add("udp.round_trip_ms", "milliseconds", value)
        return counts

    def sidecar(self):
        return json.dumps({"resource_windows": self.windows, "counter_scopes": {
            "tcp_retransmits": "Each sender/receiver guest kernel SNMP counter around the bulk exec, not per-flow packets; each delta checked independently.",
            "switch_drops": "Original owned-network close rows across setup, trials and unlink, not per-trial drop attribution.",
            "bytes": "Original Rust client completed-byte counters; no payload or kernel device-service-time claim.",
            "work_counts": "Byte/stream counts and UDP sent/received are quality facts, not lower-is-better cost metrics; UDP sent counts attempts, not delivered bytes.",
        }, "unavailable": {
            "guest_pump_cpu": "No existing fixture-owned stable pump PID/start/clock contract; pgrep/head and assumed HZ are not evidence.",
            "out_of_tree_apple_vz": "No unambiguous out-of-tree helper owner stamp; no global-name attribution.",
        }}, allow_nan=False)

    def complete(self, lanes, *, trials):
        expected = {(lane, trial) for lane in lanes for trial in range(trials)}
        actual = {(row["lane"], row["trial"]) for row in self.windows}
        if actual != expected or len(self.windows) != len(expected):
            raise ValueError("network resource window coverage is incomplete or duplicated")
        for key, metric in self.metrics.items():
            if ".resources." in key and len(metric["samples"]) != trials:
                raise ValueError("network resource series has partial trial availability: " + key)

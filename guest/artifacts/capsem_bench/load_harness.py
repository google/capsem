"""Shared load-test config, summaries, and rendering.

The load-style benches all need the same accounting contract: explicit
concurrency, enough samples, percentile latency rows, error counts, and stable
JSON. Keep that machinery here so DNS, MCP, MITM, and local mock-server
benchmarks cannot drift into incompatible result shapes.
"""

from dataclasses import dataclass
import os
import resource


GLOBAL_CONCURRENCY_ENV = "CAPSEM_BENCH_CONCURRENCY"
GLOBAL_DURATION_ENV = "CAPSEM_BENCH_DURATION_S"
GLOBAL_TOTAL_REQUESTS_ENV = "CAPSEM_BENCH_TOTAL_REQUESTS"
GLOBAL_TIMEOUT_ENV = "CAPSEM_BENCH_TIMEOUT_S"
GLOBAL_SCENARIOS_ENV = "CAPSEM_BENCH_SCENARIOS"


def _env_prefix(mode):
    return "CAPSEM_BENCH_" + mode.upper().replace("-", "_")


def _mode_env(mode, suffix):
    return f"{_env_prefix(mode)}_{suffix}"


def _env_value(mode, suffix, global_name=None):
    mode_value = os.environ.get(_mode_env(mode, suffix))
    if mode_value is not None:
        return mode_value
    if global_name:
        return os.environ.get(global_name)
    return None


def parse_positive_int(value, name):
    try:
        parsed = int(str(value).strip())
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{name} must be a positive integer") from exc
    if parsed <= 0:
        raise ValueError(f"{name} must be a positive integer")
    return parsed


def parse_positive_float(value, name):
    try:
        parsed = float(str(value).strip())
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{name} must be a positive number") from exc
    if parsed <= 0:
        raise ValueError(f"{name} must be a positive number")
    return parsed


def parse_concurrency_levels(value, name=GLOBAL_CONCURRENCY_ENV):
    levels = []
    for part in str(value).split(","):
        item = part.strip()
        if item:
            levels.append(parse_positive_int(item, name))
    if not levels:
        raise ValueError(f"{name} must include at least one positive integer")
    return tuple(levels)


def parse_name_list(value, name=GLOBAL_SCENARIOS_ENV):
    names = tuple(part.strip() for part in str(value).split(",") if part.strip())
    if not names:
        raise ValueError(f"{name} must include at least one name")
    return names


@dataclass(frozen=True)
class DurationLoadConfig:
    mode: str
    concurrency_levels: tuple[int, ...]
    duration_s: float

    @classmethod
    def from_inputs(
        cls,
        mode,
        *,
        default_concurrency,
        default_duration_s,
        concurrency_levels=None,
        duration_s=None,
    ):
        if concurrency_levels is None:
            raw = _env_value(mode, "CONCURRENCY", GLOBAL_CONCURRENCY_ENV)
            concurrency_levels = (
                parse_concurrency_levels(raw) if raw else tuple(default_concurrency)
            )
        else:
            concurrency_levels = tuple(
                parse_positive_int(value, "concurrency") for value in concurrency_levels
            )

        if duration_s is None:
            raw = _env_value(mode, "DURATION_S", GLOBAL_DURATION_ENV)
            duration_s = (
                parse_positive_float(raw, "duration_s")
                if raw else float(default_duration_s)
            )
        else:
            duration_s = parse_positive_float(duration_s, "duration_s")

        return cls(
            mode=mode,
            concurrency_levels=concurrency_levels,
            duration_s=duration_s,
        )


@dataclass(frozen=True)
class CountLoadConfig:
    mode: str
    total_requests: int
    concurrency: int
    timeout_s: float
    scenarios: tuple[str, ...] | None = None

    @classmethod
    def from_inputs(
        cls,
        mode,
        *,
        default_total_requests,
        default_concurrency,
        default_timeout_s,
        total_requests=None,
        concurrency=None,
        timeout_s=None,
        scenarios=None,
    ):
        if total_requests is None:
            raw = _env_value(mode, "TOTAL_REQUESTS", GLOBAL_TOTAL_REQUESTS_ENV)
            total_requests = (
                parse_positive_int(raw, "total_requests")
                if raw else int(default_total_requests)
            )
        else:
            total_requests = parse_positive_int(total_requests, "total_requests")

        if concurrency is None:
            raw = _env_value(mode, "CONCURRENCY", GLOBAL_CONCURRENCY_ENV)
            concurrency = (
                parse_positive_int(raw, "concurrency")
                if raw else int(default_concurrency)
            )
        else:
            concurrency = parse_positive_int(concurrency, "concurrency")

        if timeout_s is None:
            raw = _env_value(mode, "TIMEOUT_S", GLOBAL_TIMEOUT_ENV)
            timeout_s = (
                parse_positive_float(raw, "timeout_s")
                if raw else float(default_timeout_s)
            )
        else:
            timeout_s = parse_positive_float(timeout_s, "timeout_s")

        if scenarios is None:
            raw = _env_value(mode, "SCENARIOS", GLOBAL_SCENARIOS_ENV)
            scenarios = parse_name_list(raw) if raw else None
        elif isinstance(scenarios, str):
            scenarios = parse_name_list(scenarios, "scenarios")
        else:
            scenarios = tuple(scenarios)
            if not scenarios:
                raise ValueError("scenarios must include at least one name")

        return cls(
            mode=mode,
            total_requests=total_requests,
            concurrency=concurrency,
            timeout_s=timeout_s,
            scenarios=scenarios,
        )


def peak_rss_mb():
    ru = resource.getrusage(resource.RUSAGE_SELF)
    return ru.ru_maxrss / 1024.0


def summarize_load_level(latencies_ms, errors, concurrency, duration_s, *, extra=None):
    from .helpers import percentile

    if not latencies_ms:
        row = {
            "concurrency": concurrency,
            "duration_s": duration_s,
            "total_requests": 0,
            "errors": errors,
            "rps": 0.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "p99_ms": 0.0,
            "p999_ms": 0.0,
        }
    else:
        sorted_latencies = sorted(latencies_ms)
        row = {
            "concurrency": concurrency,
            "duration_s": duration_s,
            "total_requests": len(latencies_ms),
            "errors": errors,
            "rps": len(latencies_ms) / duration_s,
            "p50_ms": percentile(sorted_latencies, 50),
            "p95_ms": percentile(sorted_latencies, 95),
            "p99_ms": percentile(sorted_latencies, 99),
            "p999_ms": percentile(sorted_latencies, 99.9),
        }
    row["rss_peak_mb"] = peak_rss_mb()
    if extra:
        row.update(extra)
    return row


def render_load_table(title, rows, *, extra_columns=None):
    from rich.table import Table
    from .helpers import console

    extra_columns = extra_columns or []
    table = Table(title=title)
    table.add_column("concurrency", justify="right")
    table.add_column("requests", justify="right")
    table.add_column("rps", justify="right")
    table.add_column("p50_ms", justify="right")
    table.add_column("p95_ms", justify="right")
    table.add_column("p99_ms", justify="right")
    table.add_column("p999_ms", justify="right")
    table.add_column("errors", justify="right")
    for column, _formatter in extra_columns:
        table.add_column(column, justify="left")

    for row in rows:
        values = [
            str(row["concurrency"]),
            str(row["total_requests"]),
            f"{row['rps']:.1f}",
            f"{row['p50_ms']:.1f}",
            f"{row['p95_ms']:.1f}",
            f"{row['p99_ms']:.1f}",
            f"{row['p999_ms']:.1f}",
            str(row["errors"]),
        ]
        for _column, formatter in extra_columns:
            values.append(formatter(row))
        table.add_row(*values)
    console.print(table)

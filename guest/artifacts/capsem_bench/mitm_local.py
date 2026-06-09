"""Deterministic MITM benchmark against capsem-debug-upstream.

This mode is intentionally explicit. A host-side harness starts
capsem-debug-upstream and passes its routable base URL into the guest through
CAPSEM_BENCH_MITM_LOCAL_BASE_URL or the first CLI argument. That keeps this
benchmark local, repeatable, and free of public-network variance.
"""

import os
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from urllib.parse import urlsplit, urlunsplit

from rich.table import Table

from .helpers import console, percentile
from .load_harness import CountLoadConfig

BASE_URL_ENV = "CAPSEM_BENCH_MITM_LOCAL_BASE_URL"
DEFAULT_TOTAL_REQUESTS = 20
DEFAULT_CONCURRENCY = 1
DEFAULT_TIMEOUT_S = 30.0
SECRET_SHAPED_MARKER = "capsem_test_"

HTTP_SCENARIOS = (
    {
        "name": "tiny_http",
        "path": "/tiny",
        "expected_status": 200,
        "expected_bytes": len(b"capsem-debug-upstream:tiny\n"),
        "body_kind": "tiny",
    },
    {
        "name": "http_1mb",
        "path": "/bytes/1mb",
        "expected_status": 200,
        "expected_bytes": 1024 * 1024,
        "body_kind": "1mb",
    },
    {
        "name": "gzip_1mb",
        "path": "/gzip/1mb",
        "expected_status": 200,
        "expected_bytes": 1024 * 1024,
        "body_kind": "gzip",
    },
    {
        "name": "sse_model",
        "path": "/sse/model",
        "expected_status": 200,
        "body_kind": "sse",
        "required_text": "model.tool_call",
    },
    {
        "name": "model_json_response",
        "path": "/model/response",
        "expected_status": 200,
        "body_kind": "model_json",
        "required_text": "tool_calls",
    },
    {
        "name": "denied_target",
        "path": "/deny-target",
        "expected_status": 200,
        "body_kind": "tiny",
    },
    {
        "name": "credential_response",
        "path": "/credential/response",
        "expected_status": 200,
        "body_kind": "credential",
        "secret_shaped_fixture": True,
    },
)

WEBSOCKET_SCENARIOS = (
    {"name": "websocket_echo", "path": "/ws/echo", "frames": 10},
    {"name": "websocket_close", "path": "/ws/close", "frames": 1},
)


def _selected_http_scenarios(selected=None):
    if not selected:
        return list(HTTP_SCENARIOS)

    if isinstance(selected, str):
        wanted = [name.strip() for name in selected.split(",") if name.strip()]
    else:
        wanted = list(selected)
    by_name = {scenario["name"]: scenario for scenario in HTTP_SCENARIOS}
    unknown = [name for name in wanted if name not in by_name]
    if unknown:
        valid = ", ".join(sorted(by_name))
        raise ValueError(
            f"unknown mitm-local scenario(s): {', '.join(unknown)}; valid: {valid}"
        )
    return [by_name[name] for name in wanted]


def _strip_trailing_slash(url):
    return url.rstrip("/")


def _base_url(base_url):
    url = base_url or os.environ.get(BASE_URL_ENV)
    if not url:
        raise ValueError(
            f"mitm-local requires BASE_URL or {BASE_URL_ENV}; "
            "start capsem-debug-upstream and pass its base_url"
        )
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https") or not parts.netloc:
        raise ValueError(f"invalid mitm-local base URL: {url!r}")
    return _strip_trailing_slash(url)


def _ws_url(base_url, path):
    parts = urlsplit(base_url)
    scheme = "wss" if parts.scheme == "https" else "ws"
    return urlunsplit((scheme, parts.netloc, path, "", ""))


def _timed_http_get(session, url, timeout_s, scenario):
    start = time.monotonic()
    try:
        response = session.get(url, timeout=timeout_s)
        body = response.content
        elapsed_ms = (time.monotonic() - start) * 1000
        return {
            "status": response.status_code,
            "size": len(body),
            "latency_ms": elapsed_ms,
            "error": None,
            "required_text_present": _required_text_present(body, scenario),
            "secret_shaped_fixture_seen": _secret_fixture_seen(body, scenario),
        }
    except Exception as exc:
        elapsed_ms = (time.monotonic() - start) * 1000
        return {
            "status": 0,
            "size": 0,
            "latency_ms": elapsed_ms,
            "error": str(exc),
            "required_text_present": False,
            "secret_shaped_fixture_seen": False,
        }


def _required_text_present(body, scenario):
    required = scenario.get("required_text")
    if not required:
        return True
    return required.encode("utf-8") in body


def _secret_fixture_seen(body, scenario):
    if not scenario.get("secret_shaped_fixture"):
        return False
    return SECRET_SHAPED_MARKER.encode("utf-8") in body


def _result_ok(result, scenario):
    if result["error"] is not None:
        return False
    if result["status"] != scenario["expected_status"]:
        return False
    expected_bytes = scenario.get("expected_bytes")
    if expected_bytes is not None and result["size"] != expected_bytes:
        return False
    if not result["required_text_present"]:
        return False
    return True


def _latency_summary(latencies):
    latencies = sorted(latencies)
    return {
        "min": round(latencies[0], 1) if latencies else 0.0,
        "max": round(latencies[-1], 1) if latencies else 0.0,
        "mean": round(sum(latencies) / len(latencies), 1) if latencies else 0.0,
        "p50": round(percentile(latencies, 50), 1),
        "p95": round(percentile(latencies, 95), 1),
        "p99": round(percentile(latencies, 99), 1),
    }


def _summarize_http_results(scenario, results, wall_time_s, total_requests, concurrency):
    latencies = [r["latency_ms"] for r in results]
    successful = sum(1 for r in results if _result_ok(r, scenario))
    failed = total_requests - successful
    total_bytes = sum(r["size"] for r in results)
    errors = {}
    for result in results:
        if result["error"]:
            errors[result["error"]] = errors.get(result["error"], 0) + 1

    out = {
        "name": scenario["name"],
        "path": scenario["path"],
        "body_kind": scenario["body_kind"],
        "total_requests": total_requests,
        "concurrency": concurrency,
        "successful": successful,
        "failed": failed,
        "total_duration_ms": round(wall_time_s * 1000, 1),
        "requests_per_sec": round(total_requests / wall_time_s, 1)
        if wall_time_s > 0
        else 0.0,
        "transfer_bytes": total_bytes,
        "bytes_per_sec": round(total_bytes / wall_time_s, 1)
        if wall_time_s > 0
        else 0.0,
        "latency_ms": _latency_summary(latencies),
        "errors": errors,
    }
    if scenario.get("secret_shaped_fixture"):
        out["secret_shaped_fixture_seen"] = any(
            r["secret_shaped_fixture_seen"] for r in results
        )
        out["raw_secret_stored_in_result"] = False
    return out


def _run_http_scenario(base_url, scenario, total_requests, concurrency, timeout_s):
    import requests as req

    url = f"{base_url}{scenario['path']}"

    def worker(n_requests):
        session = req.Session()
        worker_results = []
        try:
            for _ in range(n_requests):
                worker_results.append(
                    _timed_http_get(session, url, timeout_s, scenario)
                )
        finally:
            session.close()
        return worker_results

    per_worker = total_requests // concurrency
    remainder = total_requests % concurrency
    all_results = []
    wall_start = time.monotonic()
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = []
        for idx in range(concurrency):
            n_requests = per_worker + (1 if idx < remainder else 0)
            if n_requests > 0:
                futures.append(pool.submit(worker, n_requests))
        for future in as_completed(futures):
            all_results.extend(future.result())
    wall_time_s = time.monotonic() - wall_start
    return _summarize_http_results(
        scenario, all_results, wall_time_s, total_requests, concurrency
    )


def _run_websocket_scenario(base_url, scenario, timeout_s):
    try:
        from websockets.sync.client import connect
    except Exception as exc:
        return {
            "name": scenario["name"],
            "path": scenario["path"],
            "skipped": True,
            "reason": f"websockets sync client unavailable: {exc}",
            "frames": 0,
            "frames_per_sec": 0.0,
            "latency_ms": _latency_summary([]),
        }

    url = _ws_url(base_url, scenario["path"])
    latencies = []
    frames = scenario["frames"]
    start = time.monotonic()
    try:
        with connect(
            url,
            proxy=None,
            open_timeout=timeout_s,
            close_timeout=timeout_s,
        ) as ws:
            if scenario["name"] == "websocket_echo":
                for idx in range(frames):
                    payload = f"capsem-bench-{idx}"
                    frame_start = time.monotonic()
                    ws.send(payload)
                    reply = ws.recv(timeout=timeout_s)
                    elapsed_ms = (time.monotonic() - frame_start) * 1000
                    if reply != payload:
                        raise RuntimeError(
                            f"unexpected echo reply: {reply!r} != {payload!r}"
                        )
                    latencies.append(elapsed_ms)
            else:
                # The endpoint closes immediately; connecting successfully is
                # the deterministic control frame exercise.
                latencies.append((time.monotonic() - start) * 1000)
    except Exception as exc:
        return {
            "name": scenario["name"],
            "path": scenario["path"],
            "skipped": False,
            "frames": len(latencies),
            "failed": True,
            "error": str(exc),
            "frames_per_sec": 0.0,
            "latency_ms": _latency_summary(latencies),
        }

    duration_s = time.monotonic() - start
    return {
        "name": scenario["name"],
        "path": scenario["path"],
        "skipped": False,
        "frames": frames,
        "failed": False,
        "duration_ms": round(duration_s * 1000, 1),
        "frames_per_sec": round(frames / duration_s, 1) if duration_s > 0 else 0.0,
        "latency_ms": _latency_summary(latencies),
    }


def mitm_local_bench(
    base_url=None, total_requests=None, concurrency=None, timeout_s=None,
    scenarios=None,
):
    """Run deterministic local MITM benchmark scenarios."""
    base_url = _base_url(base_url)
    config = CountLoadConfig.from_inputs(
        "mitm-local",
        default_total_requests=DEFAULT_TOTAL_REQUESTS,
        default_concurrency=DEFAULT_CONCURRENCY,
        default_timeout_s=DEFAULT_TIMEOUT_S,
        total_requests=total_requests,
        concurrency=concurrency,
        timeout_s=timeout_s,
        scenarios=scenarios,
    )
    selected_scenarios = _selected_http_scenarios(config.scenarios)

    console.print(
        "[bold]mitm-local[/bold] "
        f"base_url={base_url} requests={config.total_requests} "
        f"concurrency={config.concurrency}"
    )

    scenario_results = []
    for scenario in selected_scenarios:
        row = _run_http_scenario(
            base_url,
            scenario,
            config.total_requests,
            config.concurrency,
            config.timeout_s,
        )
        scenario_results.append(row)

    websocket = [
        _run_websocket_scenario(base_url, scenario, config.timeout_s)
        for scenario in WEBSOCKET_SCENARIOS
    ]

    out = {
        "version": "1.0",
        "base_url": base_url,
        "total_requests": config.total_requests,
        "concurrency": config.concurrency,
        "timeout_s": config.timeout_s,
        "selected_scenarios": [scenario["name"] for scenario in selected_scenarios],
        "scenarios": scenario_results,
        "websocket": websocket,
    }

    _print_table(out)
    return out


def _print_table(result):
    table = Table(title=f"mitm-local ({result['base_url']})")
    table.add_column("scenario")
    table.add_column("ok", justify="right")
    table.add_column("rps", justify="right")
    table.add_column("p50", justify="right")
    table.add_column("p95", justify="right")
    table.add_column("p99", justify="right")
    table.add_column("bytes/sec", justify="right")
    for row in result["scenarios"]:
        table.add_row(
            row["name"],
            f"{row['successful']}/{row['total_requests']}",
            f"{row['requests_per_sec']:.1f}",
            f"{row['latency_ms']['p50']:.1f} ms",
            f"{row['latency_ms']['p95']:.1f} ms",
            f"{row['latency_ms']['p99']:.1f} ms",
            f"{row['bytes_per_sec']:.1f}",
        )
    for row in result["websocket"]:
        if row.get("skipped"):
            table.add_row(row["name"], "skip", "0.0", "0.0 ms", "0.0 ms", "0.0 ms", "0.0")
            continue
        ok = "fail" if row.get("failed") else str(row["frames"])
        table.add_row(
            row["name"],
            ok,
            f"{row['frames_per_sec']:.1f}",
            f"{row['latency_ms']['p50']:.1f} ms",
            f"{row['latency_ms']['p95']:.1f} ms",
            f"{row['latency_ms']['p99']:.1f} ms",
            "0.0",
        )
    console.print(table)

"""What `config/gate.toml` says about measuring performance.

Its own module because `buildschema` is within a handful of lines of the
300-line ceiling this package enforces on itself, and because benchmarking is
a distinct subject from building and releasing.
"""

from __future__ import annotations

import math
from itertools import pairwise
from typing import Annotated, Literal

from pydantic import Field, PositiveFloat, PositiveInt, field_validator

from .benchmarkimageschema import ImageRouteConfig
from .benchmarkpolicyschema import PolicyRouteConfig
from .benchmarkstreamschema import StreamConfig, StreamEndpointEvidence
from .configschema import Strict


class RouteCaseConfig(Strict):
    """One successful HTTP method, template and distinct fixture case."""

    method: Literal["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"]
    path: str
    case: str
    parameters: dict[str, str] = Field(default_factory=dict)
    query: dict[str, str | int] = Field(default_factory=dict)
    response: Literal["json", "text", "binary", "warc", "health", "openapi", "html", "javascript"] = "json"
    target_scope: Literal["both", "gateway"] = "both"
    samples: PositiveInt | None = None
    load_requests: PositiveInt | None = None
    concurrency: PositiveInt | None = None


class RouteDebtConfig(Strict):
    """An uncovered operation names its existing fixture owner and gap."""

    owner: str = Field(min_length=1)
    reason: str = Field(min_length=20)


class RouteCorpusConfig(Strict):
    """Exact bounded read corpus, retained in the benchmark config cohort."""

    file_sizes: tuple[PositiveInt, ...]
    archive_facts_max_bytes: PositiveInt
    archive_inflated_max_bytes: PositiveInt
    archive_max_records: PositiveInt
    warc_info_max_bytes: PositiveInt
    workspace_files: PositiveInt
    log_bytes: PositiveInt
    log_line_bytes: PositiveInt
    log_tail: PositiveInt
    log_read_bytes: PositiveInt
    file_depth: PositiveInt
    mcp_tools: tuple[str, ...]
    network_name: str


class RouteCoverageConfig(Strict):
    """Exhaustive method-aware coverage; no path-only coverage claims."""

    measured: tuple[RouteCaseConfig, ...]
    streaming: dict[str, StreamEndpointEvidence]
    images: ImageRouteConfig
    policy: PolicyRouteConfig
    unmeasured: dict[str, RouteDebtConfig]
    internal: dict[str, RouteDebtConfig]
    unmeasured_cases: dict[str, RouteDebtConfig]
    corpus: RouteCorpusConfig


class RouteLoadConfig(Strict):
    """Concurrent request window applied to every measured read-only route."""

    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False)
    max_response_bytes: PositiveInt
    concurrency: PositiveInt
    requests: PositiveInt
    quick_requests: PositiveInt


class RouteBudgetConfig(Strict):
    """Actual ceilings for one route class and one transport."""

    p95_ms: PositiveFloat
    p99_ms: PositiveFloat
    cpu_s: PositiveFloat

    @field_validator("p95_ms", "p99_ms")
    @classmethod
    def latency_uses_human_increment(cls, value: float) -> float:
        """Keep ceilings honest at the precision their duration supports."""
        if value <= 10:
            increment = 1
        elif value <= 100:
            increment = 5
        elif value <= 1_000:
            increment = 10
        elif value <= 10_000:
            increment = 1_000
        elif value <= 100_000:
            increment = 5_000
        else:
            increment = 10_000
        if not math.isclose(value % increment, 0.0, abs_tol=1e-9):
            raise ValueError(f"latency must use a {increment}ms increment at this scale")
        return value


class RouteBudgetPairConfig(Strict):
    """The service and gateway have separate observable costs."""

    service: RouteBudgetConfig
    gateway: RouteBudgetConfig


class RouteBudgetsConfig(Strict):
    """Exhaustive semantic classes resolved by the route budget library."""

    status: RouteBudgetPairConfig
    vm_scalar: RouteBudgetPairConfig
    vms_list: RouteBudgetPairConfig
    stats_detail: RouteBudgetPairConfig
    aggregate_ledger: RouteBudgetPairConfig
    ledger: RouteBudgetPairConfig
    assets_status: RouteBudgetPairConfig
    mcp_default: RouteBudgetPairConfig
    mcp_servers: RouteBudgetPairConfig
    latest: RouteBudgetPairConfig
    plugin_info: RouteBudgetPairConfig
    stats: RouteBudgetPairConfig
    default: RouteBudgetPairConfig


class RouteHealthConfig(Strict):
    """How black-box route measurements retain operating margin."""

    minimum_headroom_factor: float = Field(gt=1, allow_inf_nan=False)
    cpu_accounting_slack_s: PositiveFloat
    reference_samples: PositiveInt
    window_samples: PositiveInt
    windows: PositiveInt
    concurrent_stats: RouteBudgetConfig
    budgets: RouteBudgetsConfig


class BenchmarkRunConfig(Strict):
    """What `just bench` invokes, and with what bounds.

    Every value here was a literal somewhere before: the store path in the
    binary's own `default_value`, the collector directory in a second, the
    timeout in a third. A default in the CLI is fine for running the binary by
    hand and is not config -- the gate passes these in, so changing where
    measurements land is one edit rather than a search.
    """

    crate: str
    max_collector_bytes: PositiveInt
    max_samples_per_metric: PositiveInt
    max_samples_per_run: PositiveInt
    bin_name: str
    binary: str
    collectors: str
    store: str
    interpreter: str
    timeout_secs: int
    quick_timeout_secs: int


class BenchmarkProtocolConfig(Strict):
    """Stable workload for the in-guest model and MCP protocol collector."""

    requests: PositiveInt
    concurrency: PositiveInt
    timeout_ms: PositiveInt
    scenarios: list[str]


class ContentFilterProfile(Strict):
    """One independent plugin mode compared to the disabled baseline."""

    name: str
    plugin: Literal["pii_filter", "secret_exfiltration", "utf8_guard"]
    mode: Literal["disable", "rewrite", "block"]


class BenchmarkContentFilterConfig(Strict):
    """Bounded text/SSE cohorts; concurrency respects the two-body inspection budget."""

    requests: PositiveInt
    concurrency: tuple[Annotated[int, Field(ge=1, le=2)], ...] = Field(min_length=1)
    body_bytes: tuple[Annotated[int, Field(ge=1, le=1048576)], ...] = Field(min_length=1)
    stream_delay_ms: Annotated[int, Field(ge=1, le=1000)]
    timeout_ms: PositiveInt
    profiles: tuple[ContentFilterProfile, ...] = Field(min_length=1)


class BenchmarkMitmScaleConfig(Strict):
    """Independent-VM fanout used to measure aggregate HTTPS interception."""

    vm_counts: tuple[PositiveInt, ...]
    per_vm_concurrency: PositiveInt
    duration_seconds: PositiveInt

    @field_validator("vm_counts")
    @classmethod
    def fanout_starts_at_one_and_increases(cls, values: tuple[int, ...]) -> tuple[int, ...]:
        if not values or values[0] != 1:
            raise ValueError("MITM scale fanout must start at one VM")
        if any(current <= previous for previous, current in pairwise(values)):
            raise ValueError("MITM scale VM counts must be strictly increasing")
        return values


class DiskIopsConfig(Strict):
    """A floor that is not the same number on every platform.

    Linux measures on a shared CI disk where the same hardware reads very
    differently, so one value would either be unsatisfiable there or vacuous
    everywhere else.
    """

    default: int
    linux: int


class StartupConfig(Strict):
    """Per runtime, because "slow to start" is not one duration.

    The minimal VM runtime ships only `python3`; agent CLIs and Node come from
    OCI images, which carry their own startup expectations.
    """

    python3: int


class BenchmarkGatesConfig(Strict):
    """Gross-regression floors and ceilings for the in-guest benchmark.

    Not evidence ratchets. These are authored and deliberately loose: they
    catch a collapse -- a disk doing 3 MB/s, a runtime taking thirty seconds
    to start -- rather than a drift, which is what recorded evidence is for.
    """

    disk_seq_mbps: int
    disk_rand_iops: DiskIopsConfig
    rootfs_seq_mbps: int
    rootfs_rand_iops: int
    startup_mean_ms: StartupConfig
    http_min_rps: int
    http_p99_ms: int
    throughput_min_bytes: int
    throughput_min_mbps: int
    #: Liveness rather than speed: a read reporting zero means the measurement
    #: did not happen.
    storage_min_mbps: int
    storage_min_iops: int


class BenchmarkConfig(Strict):
    stream: StreamConfig
    routes: RouteCoverageConfig
    route_load: RouteLoadConfig
    route_health: RouteHealthConfig
    run: BenchmarkRunConfig
    protocol: BenchmarkProtocolConfig
    mitm_scale: BenchmarkMitmScaleConfig
    content_filter: BenchmarkContentFilterConfig
    gates: BenchmarkGatesConfig

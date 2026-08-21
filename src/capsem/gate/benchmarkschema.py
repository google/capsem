"""What `config/gate.toml` says about measuring performance.

Its own module because `buildschema` is within a handful of lines of the
300-line ceiling this package enforces on itself, and because benchmarking is
a distinct subject from building and releasing.
"""

from __future__ import annotations

from .configschema import Strict


class RouteCoverageConfig(Strict):
    """Which HTTP routes the benchmark system measures.

    Two lists that must together account for every route the service
    registers. `unmeasured` is a debt inventory, not an exemption list: it
    starts holding every route, and entries leave it as the routes collector
    covers them. A route in neither list fails
    `tests/test_bench_route_coverage.py`, which is what makes a new route
    measured by default rather than invisible.
    """

    measured: tuple[str, ...]
    unmeasured: tuple[str, ...]


class BenchmarkConfig(Strict):
    routes: RouteCoverageConfig

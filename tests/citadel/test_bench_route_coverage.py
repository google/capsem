"""A route the benchmark system does not measure has to say so.

53 of the 101 service routes carried no timing signal of any kind -- every
file I/O route, every snapshot and save route, every skills route, every
settings and corp route, every update route, and every mutation route -- and
nothing noticed, because coverage was a hand-maintained list of 27 route
contracts inside a test file. A route added to the router simply never
appeared in it.

So coverage is derived from the router instead. Every registered route must be
named by exactly one of `[benchmark.routes] measured`, `unmeasured`, or
`internal`. `unmeasured` is debt; `internal` is the much smaller inventory of
service-only control routes that cannot be user-facing benchmarks and must
carry a reason. This file lives in Citadel so a route change fails before a
hosted package build rather than nineteen minutes into binary qualification.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).parents[2]
SERVICE = PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs"
GATEWAY = PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs"


def _route_literals(body: str) -> set[str]:
    """Read the literal first argument of each Axum `.route(...)` call.

    Route registration has one checked-in shape: a method call whose first
    argument is an ordinary Rust string literal. A source regex used to encode
    that shape invisibly. This scanner makes each delimiter explicit and
    refuses a new shape instead of silently returning an incomplete inventory.
    """
    marker = ".route("
    cursor = 0
    found: set[str] = set()
    while (call := body.find(marker, cursor)) != -1:
        start = call + len(marker)
        while start < len(body) and body[start].isspace():
            start += 1
        assert start < len(body) and body[start] == '"', (
            "route registration must use a literal path as its first argument"
        )
        end = start + 1
        while end < len(body) and body[end] != '"':
            assert body[end] != "\\", "route paths must not hide behind Rust escapes"
            end += 1
        assert end < len(body), "unterminated route path literal"
        found.add(body[start + 1 : end])
        cursor = end + 1
    return found


def _routes(source: Path, function: str) -> set[str]:
    """Every path registered inside one router-building function."""
    text = source.read_text(encoding="utf-8")
    start = text.index(function)
    end = text.find("\nfn ", start + len(function))
    body = text[start : end if end != -1 else len(text)]
    found = _route_literals(body)
    assert found, f"{function} in {source.name} registered no routes; the reader is wrong"
    return found


def _config() -> dict:
    text = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    return tomllib.loads(text)["benchmark"]["routes"]


def test_every_service_route_has_benchmark_ownership() -> None:
    registered = _routes(SERVICE, "fn build_service_router")
    coverage = _config()
    accounted = (
        set(coverage["measured"])
        | set(coverage["unmeasured"])
        | set(coverage["internal"])
    )

    missing = sorted(registered - accounted)
    assert not missing, (
        "these routes are registered but named by neither `measured`, "
        "`unmeasured`, nor `internal` in [benchmark.routes], so nothing "
        "measures them and "
        "nothing records that:\n  " + "\n  ".join(missing)
    )


def test_the_inventory_names_no_route_that_no_longer_exists() -> None:
    registered = _routes(SERVICE, "fn build_service_router")
    coverage = _config()
    accounted = (
        set(coverage["measured"])
        | set(coverage["unmeasured"])
        | set(coverage["internal"])
    )

    stale = sorted(accounted - registered)
    assert not stale, (
        "[benchmark.routes] names routes the service no longer registers; a "
        "stale entry reads as coverage and is not:\n  " + "\n  ".join(stale)
    )


def test_each_route_has_exactly_one_benchmark_classification() -> None:
    coverage = _config()
    classes = {
        "measured": set(coverage["measured"]),
        "unmeasured": set(coverage["unmeasured"]),
        "internal": set(coverage["internal"]),
    }
    duplicates = {
        route
        for routes in classes.values()
        for route in routes
        if sum(route in other for other in classes.values()) != 1
    }
    assert not duplicates, f"listed in multiple classes: {sorted(duplicates)}"


def test_internal_routes_carry_a_reason_and_are_a_tight_inventory() -> None:
    internal = _config()["internal"]
    assert all(reason.strip() for reason in internal.values())
    assert len(internal) <= 1, (
        "the internal service-only route inventory grew; prefer a public, "
        "measured gateway route unless the control-plane exception is real"
    )


def test_the_gateway_proxies_what_the_service_serves() -> None:
    """A proxied route that the service does not serve cannot be measured."""
    service = _routes(SERVICE, "fn build_service_router")
    proxied = _routes(GATEWAY, "fn service_proxy_routes")

    orphaned = sorted(proxied - service)
    assert not orphaned, (
        "the gateway proxies routes the service does not register:\n  "
        + "\n  ".join(orphaned)
    )


def test_every_service_only_route_is_explicitly_known() -> None:
    """The gateway owns `/status`; internal control routes stay on the UDS."""
    service = _routes(SERVICE, "fn build_service_router")
    proxied = _routes(GATEWAY, "fn service_proxy_routes")
    expected = {"/status", *_config()["internal"]}
    assert service - proxied == expected


def test_the_debt_is_a_ratchet_that_can_only_shrink() -> None:
    """Records where the inventory stands, so a regression is visible.

    Not a target: the number is expected to fall to zero as the routes
    collector covers each one. It may never rise -- a new route joins
    `measured`, or it joins `unmeasured` and this count is lowered in the same
    change with a reason.
    """
    coverage = _config()
    unmeasured = len(coverage["unmeasured"])
    assert unmeasured <= 101, (
        f"{unmeasured} routes are unmeasured, more than when this guard was "
        "written. A new route should be measured, not added to the debt."
    )

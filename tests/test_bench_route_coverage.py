"""A route the benchmark system does not measure has to say so.

53 of the 101 service routes carried no timing signal of any kind -- every
file I/O route, every snapshot and save route, every skills route, every
settings and corp route, every update route, and every mutation route -- and
nothing noticed, because coverage was a hand-maintained list of 27 route
contracts inside a test file. A route added to the router simply never
appeared in it.

So coverage is derived from the router instead. Every registered route must be
named by exactly one of `[benchmark.routes] measured` or `unmeasured`, and a
route in neither fails here. `unmeasured` is a debt inventory in the same shape
as the size ratchets: entries leave it as the routes collector covers them, and
it may not name a route that no longer exists.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
SERVICE = PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs"
GATEWAY = PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs"

_ROUTE = re.compile(r'\.route\(\s*"([^"]+)"')


def _routes(source: Path, function: str) -> set[str]:
    """Every path registered inside one router-building function."""
    text = source.read_text(encoding="utf-8")
    start = text.index(function)
    end = text.find("\nfn ", start + len(function))
    body = text[start : end if end != -1 else len(text)]
    found = set(_ROUTE.findall(body))
    assert found, f"{function} in {source.name} registered no routes; the reader is wrong"
    return found


def _config() -> dict:
    text = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    return tomllib.loads(text)["benchmark"]["routes"]


def test_every_service_route_is_measured_or_declared_unmeasured() -> None:
    registered = _routes(SERVICE, "fn build_service_router")
    coverage = _config()
    accounted = set(coverage["measured"]) | set(coverage["unmeasured"])

    missing = sorted(registered - accounted)
    assert not missing, (
        "these routes are registered but named by neither `measured` nor "
        "`unmeasured` in [benchmark.routes], so nothing measures them and "
        "nothing records that:\n  " + "\n  ".join(missing)
    )


def test_the_inventory_names_no_route_that_no_longer_exists() -> None:
    registered = _routes(SERVICE, "fn build_service_router")
    coverage = _config()
    accounted = set(coverage["measured"]) | set(coverage["unmeasured"])

    stale = sorted(accounted - registered)
    assert not stale, (
        "[benchmark.routes] names routes the service no longer registers; a "
        "stale entry reads as coverage and is not:\n  " + "\n  ".join(stale)
    )


def test_a_route_is_not_both_measured_and_unmeasured() -> None:
    coverage = _config()
    both = sorted(set(coverage["measured"]) & set(coverage["unmeasured"]))
    assert not both, f"listed twice, so its coverage is ambiguous: {both}"


def test_the_gateway_proxies_what_the_service_serves() -> None:
    """A proxied route that the service does not serve cannot be measured."""
    service = _routes(SERVICE, "fn build_service_router")
    proxied = _routes(GATEWAY, "fn service_proxy_routes")

    orphaned = sorted(proxied - service)
    assert not orphaned, (
        "the gateway proxies routes the service does not register:\n  "
        + "\n  ".join(orphaned)
    )


def test_the_one_route_the_gateway_serves_itself_is_known() -> None:
    """`/status` is the gateway's own composite, not a proxy hop.

    It is the single documented difference between the two routers, so it is
    asserted rather than left to be rediscovered.
    """
    service = _routes(SERVICE, "fn build_service_router")
    proxied = _routes(GATEWAY, "fn service_proxy_routes")
    assert sorted(service - proxied) == ["/status"]


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

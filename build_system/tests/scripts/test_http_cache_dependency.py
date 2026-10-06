"""Actual installed Astro dependencies must not reuse security-zeroed entries."""
from __future__ import annotations

import json
import subprocess
import tomllib
from pathlib import Path

import pytest
from helpers.bounded import bounded

ROOT = Path(__file__).resolve().parents[3]
WORKSPACES = ("web/app", "web/docs", "web/marketing", "build_system/release_site")
ADVISORY = "GHSA-ch52-4w7c-c8xp"
PROBE = r"""
const path = require('node:path');
const astro = path.dirname(require.resolve('astro/package.json', {paths:[process.argv[1]]}));
const location = require.resolve('http-cache-semantics', {paths:[astro]});
const Policy = require(location);
const request = {method:'GET',url:'https://fixture.invalid/resource',headers:{host:'fixture.invalid',accept:'text/plain'}};
const mixed = new Policy(request, {status:200,headers:{'cache-control':'public, max-age=600',vary:'Accept, *'}});
const ordinary = new Policy(request, {status:200,headers:{'cache-control':'public, max-age=600',vary:'Accept'}});
const cookie = new Policy(request, {status:200,headers:{'set-cookie':'synthetic-session=alice; HttpOnly'}});
const stale = {...request,headers:{...request.headers,'cache-control':'max-stale=31536000'}};
const errorCookie = new Policy(request, {status:200,headers:{'set-cookie':'synthetic-session=alice',
 'cache-control':'stale-if-error=31536000, stale-while-revalidate=31536000'}});
const errorResult = errorCookie.revalidatedPolicy(stale, {status:503,headers:{}});
const positive = new Policy(request, {status:200,headers:{'cache-control':'public, max-age=600, stale-if-error=31536000, stale-while-revalidate=31536000'}});
const now = positive.now(); positive.now = () => now + 700000;
process.stdout.write(JSON.stringify({location,mixed_hit:mixed.satisfiesWithoutRevalidation(request),
ordinary_hit:ordinary.satisfiesWithoutRevalidation(request),cookie_lifetime:cookie.maxAge(),
cookie_stale_hit:cookie.satisfiesWithoutRevalidation(stale),cookie_swr:errorCookie.useStaleWhileRevalidate(),
cookie_error_reused:errorResult.policy===errorCookie,positive_stale_hit:positive.satisfiesWithoutRevalidation(stale),
positive_swr:positive.useStaleWhileRevalidate(),positive_error_reused:positive.revalidatedPolicy(stale,{status:503,headers:{}}).policy===positive}));
"""


@pytest.mark.parametrize("workspace", WORKSPACES)
def test_actual_astro_cache_refuses_zero_lifetime_cookie_reuse(workspace: str) -> None:
    result = subprocess.run(
        bounded(["node", "-e", PROBE, str(ROOT / workspace)], 30),
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    facts = json.loads(result.stdout)
    assert facts["mixed_hit"] is False, facts
    assert facts["ordinary_hit"] is True, facts
    assert facts["cookie_lifetime"] == 0, facts
    assert facts["cookie_stale_hit"] is False, facts
    assert facts["cookie_swr"] is False, facts
    assert facts["cookie_error_reused"] is False, facts
    assert facts["positive_stale_hit"] is True, facts
    assert facts["positive_swr"] is True, facts
    assert facts["positive_error_reused"] is True, facts
    policy = tomllib.loads((ROOT / ".config/osv-scanner.toml").read_text())
    exceptions = [entry for entry in policy["IgnoredVulns"] if entry["id"] == ADVISORY]
    assert not exceptions, "the local max-stale fix is real: retire the obsolete exception"

#!/usr/bin/env python3
"""Purge only the release.capsem.org cache bound to the release Pages project."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import cast
from urllib.parse import quote

API_ROOT = "https://api.cloudflare.com/client/v4"
RELEASE_HOSTNAME = "release.capsem.org"
JsonObject = dict[str, object]
Requester = Callable[[str, str, JsonObject | None], JsonObject]


class CachePurgeError(RuntimeError):
    """Cloudflare did not prove an exact release-hostname cache purge."""


def _error_detail(payload: JsonObject) -> str:
    errors = payload.get("errors")
    if isinstance(errors, list):
        messages: list[str] = []
        for error in errors:
            if not isinstance(error, dict):
                continue
            message = cast(JsonObject, error).get("message")
            if message:
                messages.append(str(message))
        if messages:
            return "; ".join(messages)
    return str(payload)


def _successful_object(payload: JsonObject, operation: str) -> JsonObject:
    result = payload.get("result")
    if payload.get("success") is not True or not isinstance(result, dict):
        raise CachePurgeError(f"Cloudflare {operation} failed: {_error_detail(payload)}")
    return cast(JsonObject, result)


def _is_zone_tag(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdefABCDEF" for character in value)
    )


def purge_release_hostname(request: Requester, account_id: str, project: str) -> str:
    """Resolve the Pages-owned zone and purge only the fixed release hostname."""
    if not account_id or not project:
        raise CachePurgeError("Cloudflare account id and Pages project are required")
    domain_path = (
        f"/accounts/{quote(account_id, safe='')}/pages/projects/"
        f"{quote(project, safe='')}/domains/{RELEASE_HOSTNAME}"
    )
    domain = _successful_object(request("GET", domain_path, None), "domain lookup")
    if domain.get("name") != RELEASE_HOSTNAME:
        raise CachePurgeError(
            f"Cloudflare Pages domain is {domain.get('name')!r}, not {RELEASE_HOSTNAME!r}"
        )
    if domain.get("status") != "active":
        raise CachePurgeError(f"Cloudflare Pages domain is not active: {domain.get('status')!r}")
    zone_tag = domain.get("zone_tag")
    if not _is_zone_tag(zone_tag):
        raise CachePurgeError("Cloudflare Pages domain returned an invalid zone tag")
    assert isinstance(zone_tag, str)
    purge = _successful_object(
        request(
            "POST",
            f"/zones/{zone_tag}/purge_cache",
            {"hosts": [RELEASE_HOSTNAME]},
        ),
        "hostname cache purge",
    )
    if purge.get("id") != zone_tag:
        raise CachePurgeError("Cloudflare purge response did not identify the Pages-owned zone")
    return zone_tag


def cloudflare_requester(api_token: str) -> Requester:
    if not api_token:
        raise CachePurgeError("Cloudflare API token is required")

    def request(method: str, path: str, body: JsonObject | None) -> JsonObject:
        encoded = json.dumps(body).encode("utf-8") if body is not None else None
        headers = {"Authorization": f"Bearer {api_token}"}
        if encoded is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"{API_ROOT}{path}", method=method, headers=headers, data=encoded
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            error.close()
            raise CachePurgeError(
                f"Cloudflare {method} {path} failed: HTTP {error.code}: {detail}"
            ) from None
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise CachePurgeError(f"Cloudflare {method} {path} failed: {error}") from error
        if not isinstance(payload, dict):
            raise CachePurgeError(f"Cloudflare {method} {path} returned a non-object")
        return cast(JsonObject, payload)

    return request


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", default="release")
    parser.add_argument("--account-id", default=os.environ.get("CLOUDFLARE_ACCOUNT_ID"))
    parser.add_argument("--api-token", default=os.environ.get("CLOUDFLARE_API_TOKEN"))
    args = parser.parse_args()
    try:
        zone_tag = purge_release_hostname(
            cloudflare_requester(args.api_token or ""), args.account_id or "", args.project
        )
    except CachePurgeError as error:
        print(f"Cloudflare release hostname cache purge failed: {error}", file=sys.stderr)
        return 1
    print(f"purged {RELEASE_HOSTNAME} cache in zone {zone_tag}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

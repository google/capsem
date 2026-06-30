#!/usr/bin/env python3
"""Verify that Cloudflare credentials can see a Pages project."""

from __future__ import annotations

import argparse
import json
import os
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any


CLOUDFLARE_API_ROOT = "https://api.cloudflare.com/client/v4"


@dataclass
class CloudflareResponse:
    status: int
    payload: dict[str, Any] | None
    error: str | None = None


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fail unless the configured Cloudflare account owns a Pages project."
    )
    parser.add_argument(
        "--account-id",
        default=os.environ.get("CLOUDFLARE_ACCOUNT_ID"),
        help="Cloudflare account id. Defaults to CLOUDFLARE_ACCOUNT_ID.",
    )
    parser.add_argument(
        "--api-token",
        default=os.environ.get("CLOUDFLARE_API_TOKEN"),
        help="Cloudflare API token. Defaults to CLOUDFLARE_API_TOKEN.",
    )
    parser.add_argument(
        "--project",
        default=os.environ.get("RELEASE_CHANNEL_PROJECT", "release"),
        help="Cloudflare Pages project name.",
    )
    args = parser.parse_args()

    if not args.account_id:
        print("::error::CLOUDFLARE_ACCOUNT_ID secret is required to deploy release.capsem.org")
        return 1
    if not args.api_token:
        print("::error::CLOUDFLARE_API_TOKEN secret is required to deploy release.capsem.org")
        return 1

    response = fetch_cloudflare_project(args.account_id, args.api_token, args.project)
    ok, detail = validate_project_response(response, args.project)
    if ok:
        print(detail)
        return 0
    print(f"::error::{detail}")
    return 1


def fetch_cloudflare_project(
    account_id: str,
    api_token: str,
    project: str,
) -> CloudflareResponse:
    url = f"{CLOUDFLARE_API_ROOT}/accounts/{account_id}/pages/projects/{project}"
    request = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {api_token}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return CloudflareResponse(
                status=response.status,
                payload=json.loads(response.read().decode("utf-8")),
            )
    except urllib.error.HTTPError as error:
        try:
            payload = json.loads(error.read().decode("utf-8"))
        except (json.JSONDecodeError, UnicodeDecodeError):
            payload = None
        error.close()
        return CloudflareResponse(error.code, payload, str(error))
    except (OSError, urllib.error.URLError) as error:
        return CloudflareResponse(0, None, str(error))
    except json.JSONDecodeError as error:
        return CloudflareResponse(0, None, f"invalid JSON: {error}")


def validate_project_response(
    response: CloudflareResponse,
    expected_project: str,
) -> tuple[bool, str]:
    payload = response.payload
    if (
        response.status == 200
        and isinstance(payload, dict)
        and payload.get("success") is True
        and isinstance(payload.get("result"), dict)
        and payload["result"].get("name") == expected_project
    ):
        return (
            True,
            f"Cloudflare Pages project {expected_project} is visible to the configured account.",
        )

    details = "unknown error"
    if isinstance(payload, dict):
        errors = payload.get("errors")
        if isinstance(errors, list) and errors:
            details = "; ".join(
                f"{error.get('code', 'unknown')}: {error.get('message', 'unknown error')}"
                for error in errors
                if isinstance(error, dict)
            )
        else:
            result = payload.get("result")
            project_name = result.get("name") if isinstance(result, dict) else None
            details = (
                f"HTTP {response.status}, success={payload.get('success')}, "
                f"result.name={project_name!r}"
            )
    elif response.error:
        details = response.error

    return (
        False,
        f"Cloudflare Pages project {expected_project} is not visible to "
        f"the configured CLOUDFLARE_ACCOUNT_ID/API_TOKEN: {details}",
    )


if __name__ == "__main__":
    raise SystemExit(main())

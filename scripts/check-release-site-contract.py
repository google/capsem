#!/usr/bin/env python3
"""Validate the public release-channel site after a Cloudflare deploy."""

from __future__ import annotations

import argparse
import importlib.util
import sys
import time
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate release.capsem.org release-channel content and cache headers."
    )
    parser.add_argument(
        "--release-site",
        "--base-url",
        dest="release_site",
        default="https://release.capsem.org",
        help="Public release-channel site root.",
    )
    parser.add_argument(
        "--channel",
        action="append",
        dest="channels",
        help="Asset channel to validate. Repeat to validate multiple channels.",
    )
    parser.add_argument(
        "--attempts",
        type=int,
        default=6,
        help="Number of validation attempts while Cloudflare propagates.",
    )
    parser.add_argument(
        "--delay-seconds",
        type=float,
        default=10.0,
        help="Delay between failed validation attempts.",
    )
    args = parser.parse_args()
    exit_code = 0
    for channel in args.channels or ["stable"]:
        channel_exit_code = validate_release_site(
            release_site=args.release_site,
            channel=channel,
            attempts=args.attempts,
            delay_seconds=args.delay_seconds,
        )
        if channel_exit_code != 0:
            exit_code = channel_exit_code
    return exit_code


def validate_release_site(
    *,
    release_site: str,
    channel: str,
    attempts: int,
    delay_seconds: float,
    checker: Any | None = None,
) -> int:
    checker = checker or load_readiness_checker()
    release_site = normalize_release_site(release_site)
    if getattr(checker, "BLAKE3_IMPORT_ERROR", None) is not None:
        print(
            "missing Python dependency: blake3. Run `uv sync` before validation.",
            file=sys.stderr,
        )
        return 2

    attempts = max(attempts, 1)
    last_failures: list[Any] = []
    for attempt in range(1, attempts + 1):
        checks = []
        if urlparse(release_site).scheme != "file":
            checks.append(checker.check_release_site_dns(release_site))
        checks.append(checker.check_release_site_contract(release_site, channel))
        failures = [check for check in checks if not check.ok]
        if not failures:
            print(
                f"{release_site.rstrip('/')} {channel} release-channel contract passed."
            )
            return 0
        last_failures = failures
        for failure in failures:
            print(
                f"attempt {attempt}/{attempts}: {failure.name}: {failure.detail}",
                file=sys.stderr,
            )
        if attempt != attempts:
            time.sleep(delay_seconds)

    print(
        f"{release_site.rstrip('/')} {channel} release-channel contract failed.",
        file=sys.stderr,
    )
    for failure in last_failures:
        print(f"FAIL: {failure.name}: {failure.detail}", file=sys.stderr)
    return 1


def normalize_release_site(release_site: str) -> str:
    parsed = urlparse(release_site)
    if parsed.scheme:
        return release_site
    return Path(release_site).resolve().as_uri()


def load_readiness_checker() -> Any:
    module_path = Path(__file__).resolve().with_name("check-remote-release-readiness.py")
    spec = importlib.util.spec_from_file_location("check_remote_release_readiness", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {module_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


if __name__ == "__main__":
    raise SystemExit(main())

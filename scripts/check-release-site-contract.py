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

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from release_site_snapshot import retain_successful_external_fetches, snapshot_distribution_bytes

from capsem import runtime_preflight_manifest as channel_resolver


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
        "--catalog-members",
        action="store_true",
        help="Validate each published or retired first-party catalog member.",
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
    snapshots = parser.add_mutually_exclusive_group()
    snapshots.add_argument(
        "--snapshot-out",
        type=Path,
        help="Write exact SHA-256/size identities for every validated response.",
    )
    snapshots.add_argument(
        "--expect-snapshot",
        type=Path,
        help="Require every validated response to match a prior snapshot.",
    )
    parser.add_argument(
        "--snapshot-only",
        action="store_true",
        help=(
            "Capture or compare same-origin distribution bytes without requiring the "
            "prior graph's external references to remain healthy."
        ),
    )
    parser.add_argument(
        "--dist",
        type=Path,
        help="Fetch every public deploy-root file and require its exact local bytes.",
    )
    args = parser.parse_args()
    if args.snapshot_only and args.snapshot_out is None and args.expect_snapshot is None:
        parser.error("--snapshot-only requires --snapshot-out or --expect-snapshot")
    if args.snapshot_only and args.dist is not None:
        parser.error("--snapshot-only cannot be combined with --dist")
    if args.catalog_members and args.channels:
        parser.error("--catalog-members cannot be combined with --channel")
    checker = load_readiness_checker()

    def populate_snapshot() -> int:
        if args.catalog_members:
            return validate_catalog_members(
                release_site=args.release_site,
                attempts=1,
                delay_seconds=0,
                checker=checker,
            )
        return validate_release_channels(
            release_site=args.release_site,
            channels=args.channels or ["stable"],
            attempts=1,
            delay_seconds=0,
            checker=checker,
        )

    needs_snapshot = args.snapshot_out is not None or args.expect_snapshot is not None
    if args.snapshot_only or needs_snapshot or args.dist is not None:
        try:
            snapshot_distribution_bytes(
                checker,
                args.release_site,
                populate=populate_snapshot,
                attempts=args.attempts,
                delay_seconds=args.delay_seconds,
                snapshot_out=args.snapshot_out,
                expect_snapshot=args.expect_snapshot,
                require_valid=not args.snapshot_only,
                same_origin_only=args.snapshot_only,
                dist=args.dist,
            )
        except (OSError, RuntimeError, ValueError) as error:
            print(error, file=sys.stderr)
            return 1
        print(f"{args.release_site.rstrip('/')} release-channel byte snapshot passed.")
        return 0
    if args.catalog_members:
        return validate_catalog_members(
            release_site=args.release_site,
            attempts=args.attempts,
            delay_seconds=args.delay_seconds,
            checker=checker,
        )
    return validate_release_channels(
        release_site=args.release_site,
        channels=args.channels or ["stable"],
        attempts=args.attempts,
        delay_seconds=args.delay_seconds,
        checker=checker,
    )


def validate_catalog_members(
    *,
    release_site: str,
    attempts: int,
    delay_seconds: float,
    checker: Any | None = None,
    resolver: Any = channel_resolver.resolve_remote_channel,
) -> int:
    """Resolve typed public state, then deeply validate only catalog members."""
    retired = channel_resolver.retirement.load_retired_public_graphs()
    channels: list[str] = []
    for channel in channel_resolver.retirement.FirstPartyChannel:
        resolution = resolver(
            release_site=release_site,
            channel=channel,
            retired_public_graphs=retired,
        )
        if resolution.state is channel_resolver.ChannelState.ABSENT:
            print(f"{channel.value}: absent from the public catalog; skipped.")
            continue
        if resolution.state in {
            channel_resolver.ChannelState.PUBLISHED,
            channel_resolver.ChannelState.RETIRED,
        }:
            print(f"{channel.value}: {resolution.state.value}; validating references.")
            channels.append(channel.value)
            continue
        print(
            f"FAIL: {channel.value}: {resolution.state.value}: {resolution.detail}",
            file=sys.stderr,
        )
        return 1
    if not channels:
        print(f"{release_site.rstrip('/')} has no first-party catalog members.")
        return 0
    return validate_release_channels(
        release_site=release_site,
        channels=channels,
        attempts=attempts,
        delay_seconds=delay_seconds,
        checker=checker,
    )


def validate_release_site(
    *,
    release_site: str,
    channel: str,
    attempts: int,
    delay_seconds: float,
    checker: Any | None = None,
) -> int:
    return validate_release_channels(
        release_site=release_site,
        channels=[channel],
        attempts=attempts,
        delay_seconds=delay_seconds,
        checker=checker,
    )


def validate_release_channels(
    *,
    release_site: str,
    channels: list[str],
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
    channels = channels or ["stable"]
    last_failures: list[tuple[str, Any]] = []
    for attempt in range(1, attempts + 1):
        clear_checker_fetch_cache(checker, release_site)
        failures: list[tuple[str, Any]] = []
        if urlparse(release_site).scheme != "file":
            dns = checker.check_release_site_dns(release_site)
            if not dns.ok:
                failures.append(("dns", dns))
        if not failures:
            for channel in channels:
                contract = checker.check_release_site_contract(release_site, channel)
                if not contract.ok:
                    failures.append((channel, contract))
        if not failures:
            for channel in channels:
                print(f"{release_site.rstrip('/')} {channel} release-channel contract passed.")
            return 0
        last_failures = failures
        for channel, failure in failures:
            print(
                f"attempt {attempt}/{attempts}: {channel}: {failure.name}: {failure.detail}",
                file=sys.stderr,
            )
        if attempt != attempts:
            time.sleep(delay_seconds)

    print(
        f"{release_site.rstrip('/')} release-channel contract failed.",
        file=sys.stderr,
    )
    for channel, failure in last_failures:
        print(f"FAIL: {channel}: {failure.name}: {failure.detail}", file=sys.stderr)
    return 1


def clear_checker_fetch_cache(checker: Any, release_site: str) -> None:
    retain_successful_external_fetches(checker, release_site)


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

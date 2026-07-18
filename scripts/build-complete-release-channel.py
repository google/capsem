#!/usr/bin/env python3
"""Build one deployable release-site dist containing every public channel."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import url2pathname, urlopen


REQUIRED_CHANNELS = ("stable", "nightly")


def read_json_source(source: str) -> dict[str, Any]:
    parsed = urlparse(source)
    if parsed.scheme in {"http", "https"}:
        with urlopen(source, timeout=60) as response:
            payload = response.read()
    elif parsed.scheme == "file":
        payload = Path(url2pathname(parsed.path)).read_bytes()
    elif parsed.scheme:
        raise ValueError(f"unsupported manifest URL scheme: {parsed.scheme}")
    else:
        payload = Path(source).read_bytes()
    value = json.loads(payload)
    if not isinstance(value, dict):
        raise ValueError(f"manifest source must contain a JSON object: {source}")
    return value


def is_release_graph(value: dict[str, Any]) -> bool:
    return (
        isinstance(value.get("channel"), str)
        and isinstance(value.get("profiles"), dict)
        and isinstance(value.get("packages"), list)
    )


def parse_channel_sources(values: list[str]) -> dict[str, str]:
    sources: dict[str, str] = {}
    for value in values:
        channel, separator, source = value.partition("=")
        if separator != "=" or channel not in REQUIRED_CHANNELS or not source:
            expected = " or ".join(f"{name}=<manifest>" for name in REQUIRED_CHANNELS)
            raise ValueError(f"--channel-source must be {expected}: {value!r}")
        if channel in sources:
            raise ValueError(f"duplicate --channel-source for {channel}")
        sources[channel] = source
    if not sources:
        raise ValueError("at least one --channel-source is required")
    return sources


def resolve_channel_sources(
    *,
    explicit: dict[str, str],
    primary_channel: str,
    release_site: str,
    allow_mirror_missing: bool,
) -> tuple[dict[str, str], dict[str, dict[str, Any]]]:
    if primary_channel not in explicit:
        raise ValueError(f"primary channel {primary_channel} needs an explicit source")

    sources = dict(explicit)
    documents = {channel: read_json_source(source) for channel, source in sources.items()}
    for channel, document in documents.items():
        if is_release_graph(document) and document.get("channel") != channel:
            raise ValueError(
                f"graph source for {channel} declares channel {document.get('channel')!r}"
            )

    for channel in REQUIRED_CHANNELS:
        if channel in sources:
            continue
        public_source = f"{release_site.rstrip('/')}/assets/{channel}/manifest.json"
        try:
            public_document = read_json_source(public_source)
            if not is_release_graph(public_document):
                raise ValueError("public preserved manifest is not a release graph")
            if public_document.get("channel") != channel:
                raise ValueError(
                    f"public preserved graph declares {public_document.get('channel')!r}"
                )
        except (HTTPError, URLError, OSError, TimeoutError, ValueError, json.JSONDecodeError) as error:
            primary_source = sources[primary_channel]
            primary_document = documents[primary_channel]
            if not allow_mirror_missing:
                raise RuntimeError(
                    f"cannot preserve required {channel} channel from {public_source}: {error}"
                ) from error
            if is_release_graph(primary_document):
                raise RuntimeError(
                    f"cannot bootstrap missing {channel} from the {primary_channel} graph; "
                    "provide an explicit source"
                ) from error
            print(
                f"required {channel} channel is not published; bootstrapping it from "
                f"the {primary_channel} asset manifest",
                file=sys.stderr,
            )
            sources[channel] = primary_source
            documents[channel] = primary_document
        else:
            sources[channel] = public_source
            documents[channel] = public_document
    return sources, documents


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    print("+", " ".join(command))
    subprocess.run(command, check=True, env=env)


def build_complete_dist(args: argparse.Namespace) -> None:
    explicit = parse_channel_sources(args.channel_source)
    sources, documents = resolve_channel_sources(
        explicit=explicit,
        primary_channel=args.primary_channel,
        release_site=args.release_site,
        allow_mirror_missing=args.allow_mirror_missing,
    )
    out_dir = args.out_dir.resolve()
    if out_dir.exists():
        import shutil

        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)
    generated_at = args.generated_at or datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")

    build_order = [channel for channel in REQUIRED_CHANNELS if channel != args.primary_channel]
    build_order.append(args.primary_channel)
    graph_channels: list[str] = []
    for channel in build_order:
        command = [
            "cargo",
            "run",
            "-p",
            "capsem-admin",
            "--",
            "assets",
            "channel",
            "build",
            "--manifest",
            sources[channel],
            "--assets-dir",
            str(args.assets_dir),
            "--profiles-dir",
            str(args.profiles_dir),
            "--channel",
            channel,
            "--manifest-version",
            args.manifest_version,
            "--generated-at",
            generated_at,
            "--out-dir",
            str(out_dir),
        ]
        if args.asset_source_base:
            command.extend(["--asset-source-base", args.asset_source_base])
        run(command)
        if is_release_graph(documents[channel]):
            graph_channels.append(channel)

    env = dict(os.environ)
    env["CAPSEM_RELEASE_CHANNEL_DIST"] = str(out_dir)
    run(["bash", "scripts/check-web-surface.sh", "release-site-build"], env=env)
    for channel in graph_channels:
        command = [
            "uv",
            "run",
            "python3",
            "scripts/materialize-graph-profile-artifacts.py",
            "--dist",
            str(out_dir),
            "--channel",
            channel,
        ]
        if args.profile_source_ref:
            command.extend(["--source-ref", args.profile_source_ref])
        elif args.profile_source_root:
            command.extend(["--source-root", str(args.profile_source_root)])
        run(command)
    for channel in REQUIRED_CHANNELS:
        run(
            [
                "cargo",
                "run",
                "-p",
                "capsem-admin",
                "--",
                "assets",
                "channel",
                "check",
                "--channel",
                channel,
                "--dist",
                str(out_dir),
            ]
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel-source", action="append", default=[])
    parser.add_argument("--primary-channel", choices=REQUIRED_CHANNELS, required=True)
    parser.add_argument("--assets-dir", type=Path, default=Path("assets"))
    parser.add_argument("--profiles-dir", type=Path, default=Path("config/profiles"))
    parser.add_argument("--asset-source-base")
    parser.add_argument("--manifest-version", required=True)
    parser.add_argument("--generated-at")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--release-site", default="https://release.capsem.org")
    parser.add_argument("--allow-mirror-missing", action="store_true")
    profile_source = parser.add_mutually_exclusive_group()
    profile_source.add_argument(
        "--profile-source-ref",
        help="Override graph profile config source ref.",
    )
    profile_source.add_argument(
        "--profile-source-root",
        type=Path,
        help="Read graph profile config from this local candidate worktree.",
    )
    return parser.parse_args()


def main() -> int:
    try:
        build_complete_dist(parse_args())
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        print(f"complete release-channel build failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

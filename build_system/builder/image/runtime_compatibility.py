"""The minimum host version supported by the guest runtime being built."""

import re
import tomllib
from pathlib import Path

IMAGE_MANIFEST = Path(__file__).resolve().parents[3] / "config/docker/image/manifest.toml"
RELEASE_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def _release_version(value: str, source: str) -> tuple[int, int, int]:
    match = RELEASE_VERSION.fullmatch(value)
    if match is None:
        raise ValueError(f"{source} must be MAJOR.MINOR.PATCH, got {value!r}")
    major, minor, patch = map(int, match.groups())
    return major, minor, patch


def runtime_min_binary(binary_version: str) -> str:
    """Apply the guest ABI floor and retain the normal release-line minimum."""
    with IMAGE_MANIFEST.open("rb") as manifest:
        minimum = tomllib.load(manifest)["image"]["minimum_host_binary"]
    host = _release_version(binary_version, "host binary version")
    floor = _release_version(minimum, "minimum_host_binary")
    if host < floor:
        raise ValueError(f"host binary {binary_version} is below the guest runtime minimum {minimum}")
    return ".".join(map(str, max(floor, (host[0], host[1], 0))))

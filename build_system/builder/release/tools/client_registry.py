"""Anonymous registry reads and immutable-byte verification for client packages."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.parse import quote, urlsplit
from urllib.request import Request, urlopen

from packaging.utils import canonicalize_name

from ... import crypto
from .client_package_archives import Package
from .verify_immutable_publication import file_identity

USER_AGENT = "capsem-release (https://github.com/google/capsem)"


def _public_url(registry: str, url: str) -> str:
    allowed = {"pypi": "files.pythonhosted.org", "npm": "registry.npmjs.org", "crates": "crates.io"}
    parsed = urlsplit(url)
    if parsed.scheme != "https" or parsed.hostname != allowed[registry] or parsed.username or parsed.password:
        raise ValueError(f"unexpected {registry} public artifact origin")
    return url


def published_files(package: Package, payload: dict[str, Any]) -> dict[Path, str]:
    """Identify a valid already-published subset; mismatched versions never resume."""
    expected = {path.name: path for path in package.archives}
    if package.registry == "pypi":
        info = payload["info"]
        if canonicalize_name(info["name"]) != canonicalize_name(package.name) or info["version"] != package.version:
            raise ValueError("PyPI returned a different package identity")
        found: dict[Path, str] = {}
        for row in payload["urls"]:
            path = expected.get(row["filename"])
            if path is None or path in found:
                raise ValueError("immutable Python version has unexpected or duplicate files")
            if row["digests"]["sha256"] != file_identity(path)[1]:
                raise ValueError(f"immutable PyPI archive differs: {path.name}")
            found[path] = _public_url(package.registry, row["url"])
        return found
    if package.registry == "npm":
        if payload["name"] != package.name or payload["version"] != package.version:
            raise ValueError("npm returned a different package identity")
        return {package.archives[0]: _public_url(package.registry, payload["dist"]["tarball"])}
    version = payload["version"]
    if version["crate"] != package.name or version["num"] != package.version:
        raise ValueError("crates.io returned a different package identity")
    if version["checksum"] != file_identity(package.archives[0])[1]:
        raise ValueError("immutable crates.io archive differs")
    url = f"https://crates.io/api/v1/crates/{quote(package.name, safe='')}/{quote(package.version, safe='')}/download"
    return {package.archives[0]: _public_url(package.registry, url)}


def _metadata_url(package: Package) -> str:
    name, version = quote(package.name, safe=""), quote(package.version, safe="")
    if package.registry == "pypi":
        return f"https://pypi.org/pypi/{name}/{version}/json"
    if package.registry == "npm":
        return f"https://registry.npmjs.org/{name}/{version}"
    return f"https://crates.io/api/v1/crates/{name}/{version}"


def inspect(package: Package) -> dict[Path, str]:
    """Verify real public bytes without registry credentials or a local-cache fallback."""
    request = Request(_metadata_url(package), headers={"User-Agent": USER_AGENT, "Cache-Control": "no-cache"})
    try:
        with urlopen(request, timeout=120) as response:
            raw = response.read((1 << 20) + 1)
            if len(raw) > 1 << 20:
                raise ValueError("registry package metadata exceeds its bound")
            payload = json.loads(raw)
    except HTTPError as error:
        if error.code == 404:
            return {}
        raise
    found = published_files(package, payload)
    for archive, url in found.items():
        size, digest = file_identity(archive)
        with urlopen(Request(url, headers={"User-Agent": USER_AGENT}), timeout=120) as response:
            crypto.verify_reader(response, f"sha256:{digest}", size)
    return found

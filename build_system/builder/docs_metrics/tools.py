"""Install the pinned prebuilt source scanner into an owner-local cache."""

from __future__ import annotations

import hashlib
import io
import platform
import tarfile
import urllib.request
from pathlib import Path

VERSION = "4.1.0"
ARCHIVES = {
    ("Linux", "x86_64"): (
        "scc_Linux_x86_64.tar.gz",
        "c7328436d3027f4357d3d7853f7dc3ac2bbcb4ca08f1adad91a27c593884079b",
    ),
    ("Linux", "aarch64"): (
        "scc_Linux_arm64.tar.gz",
        "6e0d2a1f8d3540ba7df185477dec40bb7340f1b214bfd303147de5cad2bd7b8b",
    ),
    ("Darwin", "arm64"): (
        "scc_Darwin_arm64.tar.gz",
        "7201c7aa4aace058d43308462cba72adb18f6094c08c0741727455976d0a0747",
    ),
    ("Darwin", "x86_64"): (
        "scc_Darwin_x86_64.tar.gz",
        "7f705031228add7e55edded409179a60de6b538d41f153ba2922dee95adda50d",
    ),
}


def source_scanner(cache: Path) -> Path:
    archive, expected = ARCHIVES[(platform.system(), platform.machine())]
    destination = cache / f"scc-{VERSION}-{platform.system()}-{platform.machine()}"
    marker = destination.with_name(destination.name + ".sha256")
    if (
        destination.exists()
        and marker.exists()
        and hashlib.sha256(destination.read_bytes()).hexdigest() == marker.read_text()
    ):
        return destination
    request = urllib.request.Request(
        f"https://github.com/boyter/scc/releases/download/v{VERSION}/{archive}",
        headers={"User-Agent": "capsem-docs-metrics"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read(32 * 1024 * 1024)
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError("Source scanner archive failed its pinned SHA-256 check")
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as package:
        member = package.getmember("scc")
        if not member.isfile():
            raise ValueError("Source scanner archive did not contain a regular binary")
        stream = package.extractfile(member)
        if stream is None:
            raise ValueError("Source scanner binary is unreadable")
        binary = stream.read()
    cache.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    temporary.write_bytes(binary)
    temporary.chmod(0o555)
    temporary.replace(destination)
    marker.write_text(hashlib.sha256(binary).hexdigest())
    return destination

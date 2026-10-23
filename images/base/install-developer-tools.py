"""Install the shared guest toolkit from pinned snapshots and release bytes."""
from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath


def binary_bytes(data: bytes, archive: str, name: str) -> bytes:
    """Read exactly one regular binary; never extract archive paths or links."""
    if archive == "binary":
        return data
    if archive == "tar":
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as package:
            members = [member for member in package.getmembers()
                       if member.isfile() and PurePosixPath(member.name).name == name]
            if len(members) != 1:
                raise ValueError(f"{name}: expected one regular archive binary")
            stream = package.extractfile(members[0])
            assert stream is not None
            return stream.read()
    if archive == "zip":
        with zipfile.ZipFile(io.BytesIO(data)) as package:
            members = [member for member in package.infolist()
                       if not member.is_dir() and PurePosixPath(member.filename).name == name]
            if len(members) != 1:
                raise ValueError(f"{name}: expected one archive binary")
            return package.read(members[0])
    raise ValueError(f"unsupported archive type {archive}")


def install(arch: str, path: Path) -> None:
    if arch not in {"arm64", "amd64"}:
        raise ValueError(f"unsupported architecture {arch}")
    manifest = json.loads(path.read_text())
    environment = {**os.environ, "DEBIAN_FRONTEND": "noninteractive"}
    subprocess.run(["apt-get", "-o", "Acquire::Check-Valid-Until=false", "update"],
                   env=environment, check=True)
    subprocess.run(["apt-get", "install", "-y", "--no-install-recommends", *manifest["packages"]],
                   env=environment, check=True)
    for name, target in [("fd", "/usr/bin/fdfind"), ("bat", "/usr/bin/batcat")]:
        Path("/usr/local/bin", name).symlink_to(target)
    for tool in manifest["downloads"]:
        artifact = tool["artifacts"][arch]
        request = urllib.request.Request(artifact["url"], headers={"User-Agent": "Capsem-toolkit-build"})
        with urllib.request.urlopen(request, timeout=120) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != artifact["sha256"]:
            raise ValueError(f"{tool['name']}: release checksum mismatch")
        binary = binary_bytes(data, artifact["archive"], tool["name"])
        destination = Path("/usr/local/bin", tool["name"])
        destination.write_bytes(binary)
        destination.chmod(0o555)
    versions = []
    pins = {tool["name"]: tool for tool in manifest["downloads"]}
    for tool in manifest["tools"]:
        result = subprocess.run(tool["command"], capture_output=True, text=True, check=True)
        observed = (result.stdout or result.stderr).splitlines()[0]
        pin = pins.get(tool["name"])
        if pin and pin["version"] not in observed:
            raise ValueError(f"{tool['name']}: wrong observed version {observed}")
        executable = shutil.which(tool["command"][0])
        if executable is None:
            raise ValueError(f"{tool['name']}: executable not found")
        binary_path = Path(executable).resolve(strict=True)
        versions.append({"name": tool["name"], "version": observed,
                         "binary_path": str(binary_path),
                         "binary_sha256": hashlib.sha256(binary_path.read_bytes()).hexdigest(),
                         **({"artifact": pin["artifacts"][arch]} if pin else {})})
    subprocess.run(["ast-grep", "outline", "--help"], stdout=subprocess.DEVNULL, check=True)
    packages = subprocess.check_output(["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\n"], text=True)
    inventory = {"schema": "capsem.toolkit_inventory.v1", "architecture": arch,
                 "scope": "shared_base", "tools": versions,
                 "packages": [{"name": row.split("\t")[0], "version": row.split("\t")[1]}
                              for row in packages.splitlines()]}
    output = Path("/usr/local/share/capsem/toolkit-inventory.json")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(inventory, indent=2) + "\n")
    output.chmod(0o444)
    shutil.rmtree("/var/lib/apt/lists")
    Path("/var/lib/apt/lists").mkdir()


if __name__ == "__main__":
    install(sys.argv[1], Path(sys.argv[2]))

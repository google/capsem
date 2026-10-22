#!/usr/bin/env python3
"""Vendor the pinned xpra-html5 client the gateway serves for Xpra surfaces.

The pin -- package URL and SHA-256 -- lives in the vendor directory's
SOURCE.toml, beside the files it produced. This script downloads that exact
.deb, refuses it unless the digest matches, extracts the unmodified client,
then applies only the declared owned patches. SHA256SUMS covers the package
pin, patches, license and generated client. The gateway verifies all these
inputs and outputs, so an edited or missing file fails the build.

Run from the repository root:

    uv run --project build_system --frozen python \
        build_system/scripts/build/vendor_xpra_html5.py
"""

from __future__ import annotations

import argparse
import io
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.request
from pathlib import Path

from capsem_builder import crypto as hashlib

DEFAULT_VENDOR = Path("crates/capsem-gateway/vendor/xpra-html5")
#: Where the package installs the client.
WWW = "usr/share/xpra/www/"
#: The client page and everything it loads. Xpra's other pages (connect,
#: crypto, digest, mitm, clipboard), its service worker and default settings
#: are not served: Capsem serves its own settings and disconnect page.
PAGES = frozenset({"index.html", "favicon.png", "favicon.ico"})
TREES = ("css/", "js/", "icons/")
#: Precompressed copies of the same bytes.
SKIPPED_SUFFIXES = (".gz", ".br")
DOWNLOAD_TIMEOUT_SECONDS = 120


def ar_members(archive: bytes) -> dict[str, bytes]:
    """The members of a Debian package's `ar` archive, by name."""
    if not archive.startswith(b"!<arch>\n"):
        raise ValueError("not an ar archive")
    members: dict[str, bytes] = {}
    offset = 8
    while offset < len(archive):
        header = archive[offset : offset + 60]
        if len(header) != 60 or header[58:60] != b"`\n":
            raise ValueError(f"malformed ar header at {offset}")
        name = header[:16].decode("ascii").strip().rstrip("/")
        size = int(header[48:58].decode("ascii").strip())
        start = offset + 60
        members[name] = archive[start : start + size]
        offset = start + size + (size % 2)
    return members


def selected(relative: str) -> bool:
    if relative.endswith(SKIPPED_SUFFIXES):
        return False
    return relative in PAGES or relative.startswith(TREES)


def client_files(package: bytes) -> dict[str, bytes]:
    """The selected regular files under the client root, by relative path."""
    data = [content for name, content in ar_members(package).items() if name.startswith("data.tar")]
    if len(data) != 1:
        raise ValueError("expected exactly one data.tar member")
    files: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(data[0])) as tar:
        for member in tar.getmembers():
            path = member.name.removeprefix("./")
            if not path.startswith(WWW) or not member.isfile():
                continue
            relative = path.removeprefix(WWW)
            if ".." in Path(relative).parts or not selected(relative):
                continue
            extracted = tar.extractfile(member)
            if extracted is None:
                raise ValueError(f"unreadable member {member.name}")
            files[relative] = extracted.read()
    if "index.html" not in files:
        raise ValueError("the package carries no client index.html")
    return files


def checksums(files: dict[str, bytes]) -> str:
    return "".join(f"{hashlib.sha256(files[path]).hexdigest()}  {path}\n" for path in sorted(files))


def patch_client(vendor: Path, files: dict[str, bytes], patches: list[str]) -> dict[str, bytes]:
    """Apply declared patches in a private tree; never mutate the upstream input."""
    owned = []
    for name in patches:
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or path.parts[:1] != ("patches",):
            raise ValueError(f"unsafe vendor patch: {name}")
        patch = vendor / path
        if patch.is_symlink() or not patch.is_file() or not patch.resolve().is_relative_to(vendor.resolve()):
            raise ValueError(f"vendor patch is not an owned regular file: {name}")
        owned.append(patch.resolve())
    with tempfile.TemporaryDirectory(prefix="xpra-patch-") as temporary:
        tree = Path(temporary)
        for name, content in files.items():
            target = tree / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        for patch in owned:
            result = subprocess.run(
                ["git", "apply", "--whitespace=error", "--", str(patch)],
                cwd=tree, capture_output=True, text=True, check=False,
            )
            if result.returncode:
                raise ValueError(f"vendor patch {patch.name} failed: {result.stderr}")
        result = {}
        for path in tree.rglob("*"):
            if path.is_symlink():
                raise ValueError(f"vendor patch created a symlink: {path.relative_to(tree)}")
            if path.is_file():
                result[path.relative_to(tree).as_posix()] = path.read_bytes()
        if set(result) != set(files):
            raise ValueError("vendor patch changed the selected file inventory")
        return result


def write_vendor(vendor: Path, files: dict[str, bytes]) -> None:
    www = vendor / "www"
    if www.exists():
        shutil.rmtree(www)
    for relative, content in files.items():
        target = www / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    inputs_and_outputs = {
        path.relative_to(vendor).as_posix(): path.read_bytes()
        for path in vendor.rglob("*") if path.is_file() and path != vendor / "SHA256SUMS"
    }
    (vendor / "SHA256SUMS").write_text(checksums(inputs_and_outputs), encoding="utf-8")


def download(url: str, sha256: str) -> bytes:
    with urllib.request.urlopen(url, timeout=DOWNLOAD_TIMEOUT_SECONDS) as response:
        package = response.read()
    actual = hashlib.sha256(package).hexdigest()
    if actual != sha256:
        raise SystemExit(f"{url}: sha256 {actual} does not match the pin {sha256}")
    return package


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--vendor", type=Path, default=DEFAULT_VENDOR)
    parser.add_argument(
        "--package", type=Path, help="use an already downloaded .deb instead of fetching it"
    )
    args = parser.parse_args()
    source = tomllib.loads((args.vendor / "SOURCE.toml").read_text(encoding="utf-8"))
    if not source["url"].startswith("https://"):
        raise SystemExit("the pinned package URL must be https")
    if args.package:
        package = args.package.read_bytes()
        if hashlib.sha256(package).hexdigest() != source["sha256"]:
            raise SystemExit(f"{args.package}: does not match the pin {source['sha256']}")
    else:
        package = download(source["url"], source["sha256"])
    files = client_files(package)
    files = patch_client(args.vendor, files, source.get("patches", []))
    write_vendor(args.vendor, files)
    print(
        f"vendored {len(files)} files of {source['package']} {source['version']} into {args.vendor}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

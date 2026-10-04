"""capsem-debug, the test-tooling image: verified, pulled by digest, or built.

`config/gate.toml [functional.debug_image]` pins one OCI image manifest per
platform. Its layout lives in the shared cache stage that section names, one
directory per digest, so a local build and a registry pull of the same digest
are the same entry, and every checkout and gate prefix reuses it instead of
pulling it again.

    debug_image.py prepare [--platform P]   verify the pinned layout; pull it by digest if absent
    debug_image.py build [--platform P]     build images/capsem-debug into the stage; print its digest

Both are host-only: `prepare` reaches the registry and `build` drives Docker,
so the gate runs them outside its network sandbox, before any suite. The
suites call `ready()`, which only verifies.
"""

from __future__ import annotations

import argparse
import functools
import hashlib
import json
import os
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from capsem_builder.cache.config import load_paths
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]
MANIFEST_TYPES = (
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
)
CHUNK = 1 << 20
TIMEOUT = 120


@functools.cache
def _config():
    return gate_config.load(ROOT)


def settings():
    return _config().functional.debug_image


def platform_name(selected: str | None = None) -> str:
    return selected or _config().host_arch().docker_platform


def pinned(selected: str | None = None) -> str:
    """The manifest digest config/gate.toml pins for `selected` (default: this host)."""
    platform = platform_name(selected)
    digests = settings().digests
    if platform not in digests:
        raise ValueError(
            f"capsem-debug pins no image for {platform}; pinned: {sorted(digests)}"
        )
    return digests[platform]


def stage() -> Path:
    return load_paths(ROOT).stage(settings().cache_stage)


def layout_path(digest: str) -> Path:
    return stage() / f"{settings().name}-{digest.removeprefix('sha256:')}"


def _blob(layout: Path, digest: str) -> Path:
    return layout / "blobs" / "sha256" / digest.removeprefix("sha256:")


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(CHUNK), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def verify(layout: Path, digest: str, platform: str) -> None:
    """Raise unless `layout` holds exactly the `platform` image `digest` names, intact."""
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]
    if entry["digest"] != digest:
        raise ValueError(f"{layout} indexes {entry['digest']}, not {digest}")
    manifest_path = _blob(layout, digest)
    if _sha256(manifest_path) != digest:
        raise ValueError(f"{layout}: manifest bytes do not hash to {digest}")
    manifest = json.loads(manifest_path.read_bytes())
    if manifest.get("mediaType") not in MANIFEST_TYPES:
        raise ValueError(
            f"{digest} is a {manifest.get('mediaType')}, not one platform's manifest"
        )
    for descriptor in (manifest["config"], *manifest["layers"]):
        path = _blob(layout, descriptor["digest"])
        if (
            not path.is_file()
            or path.stat().st_size != descriptor["size"]
            or _sha256(path) != descriptor["digest"]
        ):
            raise ValueError(
                f"{layout}: blob {descriptor['digest']} is missing or corrupt"
            )
    image = json.loads(_blob(layout, manifest["config"]["digest"]).read_bytes())
    if f"{image['os']}/{image['architecture']}" != platform:
        raise ValueError(
            f"{digest} is {image['os']}/{image['architecture']}, not {platform}"
        )


@functools.cache
def ready(selected: str | None = None) -> Path:
    """The verified layout of the pinned image, for the harness to serve.

    Never fetches: the suites run with no network, so a missing layout is the
    prefetch step's failure, and reported as one.
    """
    platform = platform_name(selected)
    digest = pinned(platform)
    layout = layout_path(digest)
    if not layout.is_dir():
        raise FileNotFoundError(
            f"{layout} is missing: run the kingslanding prefetch "
            f"({settings().script} prepare) or `capsem-gate debug-image`"
        )
    verify(layout, digest, platform)
    return layout


def _publish(staged: Path, layout: Path) -> None:
    """Move a verified layout into place; an equal one already there wins."""
    try:
        staged.rename(layout)
    except OSError:
        if not layout.is_dir():
            raise


class _NoCrossHostAuth(urllib.request.HTTPRedirectHandler):
    """A registry redirects blobs to a CDN; its bearer token must not follow."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        new = super().redirect_request(req, fp, code, msg, headers, newurl)
        if new is not None and urllib.parse.urlsplit(newurl).netloc != req.host:
            new.remove_header("Authorization")
        return new


class _Registry:
    """One registry's HTTPS client: public roots unless a test hands it a CA."""

    def __init__(self, context: ssl.SSLContext | None = None) -> None:
        self._opener = urllib.request.build_opener(
            _NoCrossHostAuth, urllib.request.HTTPSHandler(context=context)
        )

    def open(self, url: str, token: str | None = None, accept: str | None = None):
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        if accept:
            headers["Accept"] = accept
        return self._opener.open(
            urllib.request.Request(url, headers=headers), timeout=TIMEOUT
        )


def _token(client: _Registry, registry: str, name: str) -> str | None:
    """An anonymous pull token, if the registry's challenge asks for one."""
    try:
        client.open(f"https://{registry}/v2/").close()
        return None
    except urllib.error.HTTPError as error:
        if error.code != 401:
            raise
        challenge = error.headers.get("WWW-Authenticate", "")
    fields = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
    query = urllib.parse.urlencode(
        {"service": fields.get("service", registry), "scope": f"repository:{name}:pull"}
    )
    with client.open(f"{fields['realm']}?{query}") as response:
        body = json.load(response)
    return body.get("token") or body["access_token"]


def _fetch(
    client: _Registry,
    url: str,
    token: str | None,
    accept: str | None,
    target: Path,
    digest: str,
) -> None:
    partial = target.with_suffix(".part")
    with client.open(url, token, accept) as response, partial.open("wb") as out:
        shutil.copyfileobj(response, out, CHUNK)
    if _sha256(partial) != digest:
        raise ValueError(f"{url} did not serve {digest}")
    partial.rename(target)


def pull(
    digest: str,
    platform: str,
    layout: Path,
    *,
    repository: str | None = None,
    context: ssl.SSLContext | None = None,
) -> None:
    """Fetch the pinned manifest and its blobs verbatim, so the digest survives."""
    registry, name = (repository or settings().repository).split("/", 1)
    client = _Registry(context)
    token = _token(client, registry, name)
    base = f"https://{registry}/v2/{name}"
    with tempfile.TemporaryDirectory(dir=layout.parent, prefix=".pull-") as tmp:
        staged = Path(tmp) / "layout"
        _blob(staged, digest).parent.mkdir(parents=True)
        _fetch(
            client,
            f"{base}/manifests/{digest}",
            token,
            ",".join(MANIFEST_TYPES),
            _blob(staged, digest),
            digest,
        )
        manifest_bytes = _blob(staged, digest).read_bytes()
        manifest = json.loads(manifest_bytes)
        for descriptor in (manifest["config"], *manifest["layers"]):
            blob = descriptor["digest"]
            _fetch(
                client, f"{base}/blobs/{blob}", token, None, _blob(staged, blob), blob
            )
        os_name, architecture = platform.split("/")
        (staged / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        entry = {
            "mediaType": manifest["mediaType"],
            "digest": digest,
            "size": len(manifest_bytes),
            "platform": {"architecture": architecture, "os": os_name},
        }
        (staged / "index.json").write_text(
            json.dumps({"schemaVersion": 2, "manifests": [entry]}, sort_keys=True)
        )
        verify(staged, digest, platform)
        _publish(staged, layout)


def prepare(selected: str | None = None) -> Path:
    platform = platform_name(selected)
    digest = pinned(platform)
    layout = layout_path(digest)
    layout.parent.mkdir(parents=True, exist_ok=True)
    if layout.is_dir():
        verify(layout, digest, platform)
        print(f"Verified cached capsem-debug {platform}: {digest}")
    else:
        pull(digest, platform, layout)
        print(
            f"Pulled capsem-debug {platform} by digest: {settings().repository}@{digest}"
        )
    # The stage is retained least-recently-used; using an entry is what keeps it.
    os.utime(layout)
    return layout


def build(selected: str | None = None) -> str:
    """Build the image into the stage by digest and load it as `<name>:<arch>` too."""
    platform = platform_name(selected)
    context = str(ROOT / settings().context)
    common = [
        "docker",
        "buildx",
        "build",
        "--platform",
        platform,
        "--provenance=false",
        "--sbom=false",
    ]
    stage().mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=stage(), prefix=".build-") as tmp:
        staged = Path(tmp) / "layout"
        output = f"type=oci,tar=false,dest={staged}"
        subprocess.run([*common, "--output", output, context], check=True, timeout=3600)
        (entry,) = json.loads((staged / "index.json").read_text())["manifests"]
        digest = entry["digest"]
        verify(staged, digest, platform)
        layout = layout_path(digest)
        if layout.is_dir():
            verify(layout, digest, platform)
        else:
            _publish(staged, layout)
    # The same build again from BuildKit's cache, loaded into Docker under a
    # local tag, for images/ci/rootfs.py (the OBOM) and hands-on use.
    tag = f"{settings().name}:{platform.split('/')[1]}"
    subprocess.run([*common, "--load", "--tag", tag, context], check=True, timeout=3600)
    print(f"Built {settings().name} {platform}: {digest}")
    print(f"  layout: {layout}")
    print(f"  docker: {tag}")
    print("Pin it in config/gate.toml [functional.debug_image] digests:")
    print(f'  "{platform}" = "{digest}"')
    return digest


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("action", choices=("prepare", "build"))
    parser.add_argument("--platform", help="linux/<arch>; this host's by default")
    args = parser.parse_args(argv)
    (prepare if args.action == "prepare" else build)(args.platform)


if __name__ == "__main__":
    sys.exit(main())

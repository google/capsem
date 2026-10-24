"""Images the suites serve by pinned digest: verified, pulled, or built.
`config/gate.toml [functional.<section>]` pins one OCI image manifest per
platform for each image the harness serves:

- `debug_image`: capsem-debug, the test-tooling image;
- `reference_image`: the official `dev` image, the product stand-in every
  runtime test boots.

Layouts share the configured cache stage. Both actions are host-only; suites
call ready(), which verifies without fetching; --receipt is CI evidence only.
"""

from __future__ import annotations

import argparse
import base64
import functools
import hashlib
import json
import os
import re
import ssl
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from capsem_builder.cache.config import load_paths
from capsem_builder.gate import config as gate_config
from capsem_builder.release.tools.fetch_channel_source_manifest import github_token

ROOT = Path(__file__).resolve().parents[3]
if __package__ in (None, ""):
    sys.path.insert(0, str(ROOT))

MANIFEST_TYPES = (
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
)
CHUNK = 1 << 20
TIMEOUT = 120


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
    if (
        not isinstance(manifest, dict)
        or manifest.get("mediaType") not in MANIFEST_TYPES
        or not isinstance(manifest.get("layers"), list)
    ):
        raise ValueError(f"{digest} is a {manifest.get('mediaType')}, not one platform's manifest")
    for descriptor in (manifest["config"], *manifest["layers"]):
        path = _blob(layout, descriptor["digest"])
        if (
            not path.is_file()
            or path.stat().st_size != descriptor["size"]
            or _sha256(path) != descriptor["digest"]
        ):
            raise ValueError(f"{layout}: blob {descriptor['digest']} is missing or corrupt")
    image = json.loads(_blob(layout, manifest["config"]["digest"]).read_bytes())
    if f"{image['os']}/{image['architecture']}" != platform:
        raise ValueError(f"{digest} is {image['os']}/{image['architecture']}, not {platform}")


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

    def open(
        self,
        url: str,
        token: str | None = None,
        accept: str | None = None,
        basic: tuple[str, str] | None = None,
    ):
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        if basic is not None:
            encoded = base64.b64encode(f"{basic[0]}:{basic[1]}".encode()).decode()
            headers["Authorization"] = f"Basic {encoded}"
        if accept:
            headers["Accept"] = accept
        return self._opener.open(urllib.request.Request(url, headers=headers), timeout=TIMEOUT)


def _token(client: _Registry, registry: str, name: str) -> str | None:
    """An anonymous pull token, if the registry's challenge asks for one."""
    try:
        client.open(f"https://{registry}/v2/").close()
        return None
    except urllib.error.HTTPError as error:
        with error:
            if error.code != 401:
                raise
            challenge = error.headers.get("WWW-Authenticate", "")
    fields = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
    query = urllib.parse.urlencode(
        {"service": fields.get("service", registry), "scope": f"repository:{name}:pull"}
    )
    token_url = f"{fields['realm']}?{query}"
    realm = urllib.parse.urlsplit(fields.get("realm", ""))
    # GHCR can issue an anonymous token for a private package, then 404 its manifest.
    credential = (
        github_token()
        if registry == "ghcr.io" and realm == urllib.parse.urlsplit("https://ghcr.io/token")
        else None
    )
    try:
        response = client.open(
            token_url, basic=("x-access-token", credential) if credential else None
        )
    except urllib.error.HTTPError as error:
        with error:
            if error.code not in (401, 403) or credential is None:
                raise
            response = client.open(token_url)
    with response:
        body = json.load(response)
    return body.get("token") or body["access_token"]


def _fetch(
    client: _Registry,
    url: str,
    token: str | None,
    accept: str | None,
    target: Path,
    digest: str,
    size: int,
) -> None:
    partial = target.with_suffix(".part")
    with client.open(url, token, accept) as response, partial.open("wb") as out:
        count = 0
        while chunk := response.read(min(CHUNK, size - count + 1)):
            count += len(chunk)
            if count > size:
                raise ValueError(
                    f"{url} did not serve {digest}: blob exceeds its original descriptor"
                )
            out.write(chunk)
    if count != size or _sha256(partial) != digest:
        raise ValueError(f"{url} did not serve {digest}")
    partial.rename(target)


def manifest_intent(
    raw: bytes, digest: str, platform: str, maximum: int
) -> tuple[dict, bytes, int]:
    """Verified original descriptor closure, before any cache or blob write."""
    if len(raw) > maximum or "sha256:" + hashlib.sha256(raw).hexdigest() != digest:
        raise ValueError("original manifest exceeds its bound or digest differs")
    manifest = json.loads(raw)
    if manifest.get("mediaType") not in MANIFEST_TYPES:
        raise ValueError("original subject is not a platform manifest")
    total = len(raw)
    for item in (manifest["config"], *manifest["layers"]):
        if (
            not isinstance(item, dict)
            or type(item.get("size")) is not int
            or item["size"] < 0
            or not isinstance(item.get("digest"), str)
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", item["digest"])
        ):
            raise ValueError("original blob descriptor size/digest is invalid")
        # Repeated descriptors count again: their old blob and a replacement
        # .part can coexist. Each new blob otherwise renames, not copies.
        total += item["size"]
    os_name, architecture = platform.split("/")
    entry = {
        "mediaType": manifest["mediaType"],
        "digest": digest,
        "size": len(raw),
        "platform": {"architecture": architecture, "os": os_name},
    }
    index = json.dumps({"schemaVersion": 2, "manifests": [entry]}, sort_keys=True).encode()
    return manifest, index, total + len(index) + len(b'{"imageLayoutVersion":"1.0.0"}')


def _manifest(client, base, token, digest, maximum):
    with client.open(f"{base}/manifests/{digest}", token, ",".join(MANIFEST_TYPES)) as response:
        raw = response.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError("original manifest exceeds its metadata bound")
    return raw


def download_intent(digest, *, repository, maximum, context=None):
    registry, name = repository.split("/", 1)
    client = _Registry(context)
    return _manifest(
        client, f"https://{registry}/v2/{name}", _token(client, registry, name), digest, maximum
    )


def pull(
    digest: str,
    platform: str,
    layout: Path,
    *,
    repository: str,
    context: ssl.SSLContext | None = None,
    original_manifest: bytes | None = None,
) -> None:
    """Fetch the pinned manifest and its blobs verbatim, so the digest survives."""
    registry, name = repository.split("/", 1)
    client = _Registry(context)
    token = _token(client, registry, name)
    base = f"https://{registry}/v2/{name}"
    maximum = _config().functional.reference_image.evidence_max_bytes
    manifest_bytes = (
        original_manifest
        if original_manifest is not None
        else _manifest(client, base, token, digest, maximum)
    )
    manifest, index, _ = manifest_intent(manifest_bytes, digest, platform, maximum)
    with tempfile.TemporaryDirectory(dir=layout.parent, prefix=".pull-") as tmp:
        staged = Path(tmp) / "layout"
        _blob(staged, digest).parent.mkdir(parents=True)
        _blob(staged, digest).write_bytes(manifest_bytes)
        for descriptor in (manifest["config"], *manifest["layers"]):
            blob = descriptor["digest"]
            _fetch(
                client,
                f"{base}/blobs/{blob}",
                token,
                None,
                _blob(staged, blob),
                blob,
                descriptor["size"],
            )
        (staged / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        (staged / "index.json").write_bytes(index)
        verify(staged, digest, platform)
        _publish(staged, layout)


@functools.cache
def _config():
    return gate_config.load(ROOT)


class PinnedImage:
    """One `[functional.<section>]` pin and its layout in the cache stage."""

    def __init__(self, section: str) -> None:
        self.section = section

    def settings(self):
        return getattr(_config().functional, self.section)

    @staticmethod
    def platform_name(selected: str | None = None) -> str:
        return selected or _config().host_arch().docker_platform

    def pinned(self, selected: str | None = None) -> str:
        """The manifest digest the config pins for `selected` (default: this host)."""
        platform = self.platform_name(selected)
        digests = self.settings().digests
        if platform not in digests:
            raise ValueError(
                f"{self.settings().name} pins no image for {platform}; pinned: {sorted(digests)}"
            )
        return digests[platform]

    def stage(self) -> Path:
        return load_paths(ROOT).stage(self.settings().cache_stage)

    def layout_path(self, digest: str) -> Path:
        return self.stage() / f"{self.settings().name}-{digest.removeprefix('sha256:')}"

    verify = staticmethod(verify)

    def ready(self, selected: str | None = None) -> Path:
        """The verified layout of the pinned image, for the harness to serve.

        Never fetches: the suites run with no network, so a missing layout is
        the prefetch step's failure, and reported as one.
        """
        return _ready(self.section, self.platform_name(selected))

    def pull(
        self,
        digest: str,
        platform: str,
        layout: Path,
        *,
        repository: str | None = None,
        context: ssl.SSLContext | None = None,
    ) -> None:
        """Fetch the pinned manifest and its blobs verbatim, so the digest survives."""
        pull(
            digest,
            platform,
            layout,
            repository=repository or self.settings().repository,
            context=context,
        )

    def prepare(self, selected: str | None = None) -> Path:
        name = self.settings().name
        platform = self.platform_name(selected)
        digest = self.pinned(platform)
        layout = self.layout_path(digest)
        layout.parent.mkdir(parents=True, exist_ok=True)
        if layout.is_dir():
            verify(layout, digest, platform)
            print(f"Verified cached {name} {platform}: {digest}")
        else:
            self.pull(digest, platform, layout)
            print(f"Pulled {name} {platform} by digest: {self.settings().repository}@{digest}")
        # The stage is retained least-recently-used; using an entry is what keeps it.
        os.utime(layout)
        from tests.fixtures.oci.published_root import prepare

        prepare(layout, self.settings().repository)
        return layout

    def build(self, selected: str | None = None) -> str:
        from tests.fixtures.oci.image_build import build

        return build(self, selected)


@functools.cache
def _ready(section: str, platform: str) -> Path:
    image = SECTIONS[section]
    digest = image.pinned(platform)
    layout = image.layout_path(digest)
    if not layout.is_dir():
        raise FileNotFoundError(
            f"{layout} is missing: run the kingslanding prefetch "
            f"({image.settings().script} {section} prepare)"
        )
    verify(layout, digest, platform)
    return layout


debug_image = PinnedImage("debug_image")
reference_image = PinnedImage("reference_image")
SECTIONS = {image.section: image for image in (debug_image, reference_image)}


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("section", choices=sorted(SECTIONS))
    parser.add_argument("action", choices=("prepare", "build"))
    parser.add_argument("--platform", help="linux/<arch>; this host's by default")
    parser.add_argument("--receipt", action="store_true")
    parser.add_argument(
        "--candidate-image",
        choices=("dev", "codex-cli", "claude-code", "agy", "antigravity-desktop"),
    )
    args = parser.parse_args(argv)
    if args.candidate_image:
        if (args.section, args.action, args.receipt) != ("reference_image", "build", True):
            parser.error("--candidate-image requires reference_image build --receipt")
        from tests.fixtures.oci import native_build, native_candidates

        original = native_candidates.source(ROOT)
        config = _config()
        selected = args.platform or config.host_arch().docker_platform
        with native_build.produce(config, args.candidate_image, selected) as (
            image,
            digest,
            binding,
        ):
            native_candidates.record(config, image, digest, selected, original, build=binding)
        return
    image = SECTIONS[args.section]
    if args.receipt:
        if args.action != "build":
            parser.error("--receipt is only valid for build")
        from tests.fixtures.oci import native_candidates

        original = native_candidates.source(ROOT)
        digest = image.build(args.platform)
        native_candidates.record(
            _config(), image, digest, image.platform_name(args.platform), original
        )
    else:
        (image.prepare if args.action == "prepare" else image.build)(args.platform)


if __name__ == "__main__":
    sys.exit(main())

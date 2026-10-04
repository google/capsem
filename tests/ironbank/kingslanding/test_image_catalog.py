"""An image runs by catalog name, and the catalog alone admits it.

The service reads the catalog `[images] catalog` names -- here one a
hermetic registry serves, trusted through `[images] catalog_ca` -- and
resolves a name to the digest it pins for this host. No `[images]` grant
names the registry: the catalog makes its repository a source and its
digest admitted. A digest the catalog does not list is refused after the
pull, and a name it does not list names nothing; neither reaches Docker Hub.
"""

import re
import subprocess

import pytest
from helpers.constants import CODE_PROFILE_ID

from tests.fixtures.oci.prepare_redis import native_pin
from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_run import cli, environment, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

UNLISTED = "sha256:" + "0" * 64


def use_catalog(service, reference, certificate):
    """Point the service's `[images] catalog` at `reference`, with no grants."""
    path = service.home_dir / "settings.toml"
    text = path.read_text() if path.exists() else ""
    text = re.sub(r"\[images\]\n(?:[^\[\n][^\n]*\n)*", "", text)
    block = f'[images]\ncatalog = "{reference}"\ncatalog_ca = "{certificate}"\n'
    path.write_text(text + ("\n" if text and not text.endswith("\n") else "") + block)


def run(service, certificate, image, *args):
    return subprocess.run(
        [
            *cli(service, "run", "--profile", CODE_PROFILE_ID, "--registry-ca", str(certificate)),
            "--image",
            image,
            *args,
        ],
        env=environment(service),
        capture_output=True,
        timeout=180,
        check=False,
    )


def entry(image):
    return {
        "redis": {
            "description": "Hermetic Redis fixture",
            "versions": [{"image": image, "platforms": [native_pin()["platform"]], "contract": 1}],
        }
    }


def test_an_image_runs_by_catalog_name_and_only_listed_digests_run(service, tmp_path):
    settings = service.home_dir / "settings.toml"
    before = settings.read_text() if settings.exists() else None
    artifacts = {}
    try:
        with registry(tmp_path, artifacts=artifacts) as (reference, certificate, requests):
            authority, pinned = reference.split("/", 1)[0], reference.rsplit("@", 1)[1]
            repository = reference.rsplit("@", 1)[0]
            catalog = publish_catalog(artifacts, "library/catalog", "stable", entry(reference))
            # The same entry, revoked: the catalog now lists another digest.
            publish_catalog(artifacts, "library/catalog", "revoked", entry(f"{repository}@{UNLISTED}"))
            use_catalog(service, f"{authority}/library/catalog:stable", certificate)

            listing = service.client().get("/images")
            assert listing["catalog"]["reference"] == f"{authority}/library/catalog:stable"
            assert listing["catalog"]["digest"] == catalog
            (image,) = listing["images"]
            assert image["name"] == "redis"
            assert image["image"] == reference
            assert image["cached"] == "unknown"

            # `redis` is also a Docker Hub short name; the catalog's wins.
            ran = run(service, certificate, "redis", "redis-server", "--version")
            output = (ran.stdout + ran.stderr).decode(errors="replace")
            assert ran.returncode == 0, output
            assert "Redis server v=" in output, output
            assert any(path.endswith(f"/manifests/{pinned}") for path in requests), (
                f"the name was not pulled by its pin: {requests}"
            )
            assert service.client().get("/vms/list")["sandboxes"] == []

            pulled = subprocess.run(
                [*cli(service, "images", "pull", "--registry-ca", str(certificate), "redis")],
                env=environment(service),
                capture_output=True,
                timeout=180,
                check=False,
            )
            assert pulled.returncode == 0, pulled.stderr
            assert pulled.stdout.decode().strip() == reference

            # A name the catalog does not list names nothing.
            missing = run(service, certificate, "postgres", "true")
            assert missing.returncode != 0
            assert b"not in the image catalog" in missing.stdout + missing.stderr
            assert service.client().get("/vms/list")["sandboxes"] == []

            # Revoked: the very digest that just ran is refused after the
            # pull, since the catalog no longer lists it and nothing grants it.
            use_catalog(service, f"{authority}/library/catalog:revoked", certificate)
            refused = run(service, certificate, reference, "true")
            refusal = (refused.stdout + refused.stderr).decode(errors="replace")
            assert refused.returncode != 0, refusal
            assert "not admitted" in refusal, refusal
            assert service.client().get("/vms/list")["sandboxes"] == []
            status, body = service.client().call_json(
                "POST", "/images/pull", {"image": reference}, timeout=180
            )
            assert status == 403, body
            assert "not admitted" in body["error"], body
    finally:
        if before is None:
            settings.unlink(missing_ok=True)
        else:
            settings.write_text(before)

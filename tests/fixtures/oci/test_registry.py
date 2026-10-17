"""The hermetic registry serves a catalog the way the image workflow pushes it."""

import hashlib
import json
import ssl
import urllib.request

from tests.fixtures.oci.registry import CATALOG_MEDIA, publish_catalog, serve


def test_a_published_catalog_is_one_verified_layer_behind_a_moving_tag(tmp_path):
    artifacts = {}
    manifest = b'{"schemaVersion":2}'
    with serve(tmp_path, "redis", manifest, "application/json", lambda _: None, artifacts) as (
        reference,
        certificate,
        requests,
    ):
        authority = reference.split("/", 1)[0]
        image = f"{authority}/library/redis@sha256:{'a' * 64}"
        entries = {
            "redis": {
                "description": "Hermetic Redis",
                "versions": [{"image": image, "platforms": ["linux/arm64"], "contract": 1}],
            }
        }
        digest = publish_catalog(artifacts, "library/catalog", "stable", entries)
        context = ssl.create_default_context(cafile=str(certificate))

        def get(path):
            with urllib.request.urlopen(f"https://{authority}{path}", context=context) as response:
                return response.read(), response.headers["Docker-Content-Digest"]

        body, served_digest = get("/v2/library/catalog/manifests/stable")
        assert served_digest == digest == "sha256:" + hashlib.sha256(body).hexdigest()
        artifact = json.loads(body)
        (layer,) = artifact["layers"]
        assert artifact["artifactType"] == layer["mediaType"] == CATALOG_MEDIA
        document, _ = get(f"/v2/library/catalog/blobs/{layer['digest']}")
        assert "sha256:" + hashlib.sha256(document).hexdigest() == layer["digest"]
        assert len(document) == layer["size"]
        assert json.loads(document) == {"schema_version": 1, "channel": "stable", "entries": entries}
        assert requests == [
            "/v2/library/catalog/manifests/stable",
            f"/v2/library/catalog/blobs/{layer['digest']}",
        ]

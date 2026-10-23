"""Canonical catalog metadata and subject-bound evidence for local acceptance."""

import json
from typing import Any

from images.ci import catalog
from tests.fixtures.oci.published_root import EMPTY, MANIFEST, descriptor
from tests.fixtures.oci.registry import publish_catalog

INVENTORY = "application/vnd.capsem.package-inventory.v1+json"
OBOM = "application/vnd.cyclonedx+json"


def _attach(artifacts, repository, subject, body, media):
    document = {
        "schemaVersion": 2, "mediaType": MANIFEST, "artifactType": media,
        "subject": {key: subject[key] for key in ("mediaType", "digest", "size")},
        "config": descriptor(b"{}", EMPTY), "layers": [descriptor(body, media)],
    }
    data = json.dumps(document, sort_keys=True).encode()
    artifact = descriptor(data, MANIFEST)
    for payload, kind, route in ((body, media, "blobs"), (b"{}", EMPTY, "blobs"), (data, MANIFEST, "manifests")):
        artifacts[f"/v2/{repository}/{route}/{descriptor(payload, kind)['digest']}"] = (payload, kind)
    referrers = f"/v2/{repository}/referrers/{subject['digest']}"
    index: dict[str, Any] = json.loads(artifacts[referrers][0]) if referrers in artifacts else {"schemaVersion": 2, "manifests": []}
    index["manifests"].append({**artifact, "artifactType": media})
    artifacts[referrers] = (json.dumps(index).encode(), "application/vnd.oci.image.index.v1+json")
    return artifact["digest"]


def publish(artifacts, reference, subject, config, evidence, root):
    """Use the publication producer, never a second set of app names or metadata.

    `evidence(reference)` runs the canonical rootfs producer for this local
    repository and returns its output directory. A foreign subject is an error;
    callers cannot turn a GHCR inventory into local evidence by relabeling it.
    """
    repository = reference.split("@", 1)[0]
    image = repository.rsplit("/", 1)[1]
    route = repository.split("/", 1)[1]
    arch = config["architecture"]
    surface = (config.get("config", {}).get("Labels") or {}).get("org.capsem.surface", "terminal")
    if catalog.surfaces().get(image) != surface:
        raise ValueError("local image surface differs from canonical catalog metadata")
    version = {"image": reference, "platforms": [f"{config['os']}/{arch}"], "contract": catalog.CONTRACT}
    if root:
        version["erofs"] = {arch: root["digest"]}
    if evidence:
        output = evidence(reference)
        inventory = (output / "packages.json").read_bytes()
        document = json.loads(inventory)
        if document.get("subject") != reference or document.get("architecture") != arch:
            raise ValueError("local inventory subject or platform differs")
        obom = (output / "obom.cdx.json").read_bytes()
        component = json.loads(obom)["metadata"]["component"]
        if component["name"] != repository or component["version"] != subject["digest"]:
            raise ValueError("local OBOM subject differs")
        pin = _attach(artifacts, route, subject, inventory, INVENTORY)
        version["evidence"] = {arch: [{"kind": "package_inventory", "reference": f"{repository}@{pin}", "media_type": INVENTORY}]}
        version["obom"] = {arch: _attach(artifacts, route, subject, obom, OBOM)}
    document, _ = catalog.merge(
        None, channel="stable", generated_at="local-prepared-acceptance",
        described=catalog.descriptions(), new={image: version},
        surfaces=catalog.surfaces(), metadata=catalog.metadata(),
    )
    publish_catalog(artifacts, "library/catalog", "stable", document["entries"])

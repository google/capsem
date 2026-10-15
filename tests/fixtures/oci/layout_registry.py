"""A pinned OCI layout and its prepared EROFS served through a real registry."""

import contextlib
import json
from pathlib import Path


@contextlib.contextmanager
def layout_registry(directory, layout, image, *, referrers_api=True):
    """Serve the single-platform image in an OCI layout (`docker buildx build
    --output type=oci,tar=false`) under `library/<image>`."""
    from tests.fixtures.oci.registry import publish_catalog, serve

    layout = Path(layout)
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]

    def fetch(digest):
        path = layout / "blobs" / "sha256" / digest.removeprefix("sha256:")
        return path.read_bytes() if digest.startswith("sha256:") and path.is_file() else None

    manifest = fetch(entry["digest"])
    assert manifest is not None, entry
    artifacts = {}
    prepared = (layout / "rootfs-v2").is_dir()
    if prepared:
        from tests.fixtures.oci.published_root import ready

        root_manifest, _ = ready(layout)
        (artifact,) = json.loads((layout / "rootfs-v2/index.json").read_text())["manifests"]
        for blob in (artifact, root_manifest["config"], *root_manifest["layers"]):
            path = layout / "rootfs-v2/blobs/sha256" / blob["digest"].removeprefix("sha256:")
            route = "manifests" if blob == artifact else "blobs"
            artifacts[f"/v2/library/{image}/{route}/{blob['digest']}"] = (path, blob["mediaType"])
        referrers_path = (
            f"referrers/{entry['digest']}"
            if referrers_api
            else f"manifests/{entry['digest'].replace(':', '-', 1)}"
        )
        artifacts[f"/v2/library/{image}/{referrers_path}"] = (
            json.dumps(
                {
                    "schemaVersion": 2,
                    "manifests": [{**artifact, "artifactType": root_manifest["artifactType"]}],
                }
            ).encode(),
            "application/vnd.oci.image.index.v1+json",
        )
    with serve(directory, image, manifest, entry["mediaType"], fetch, artifacts) as served:
        if prepared:
            reference, _, _ = served
            config = json.loads(fetch(json.loads(manifest)["config"]["digest"]))
            publish_catalog(
                artifacts,
                "library/catalog",
                "stable",
                {
                    image: {
                        "description": "Prepared native fixture",
                        "versions": [
                            {
                                "image": reference,
                                "platforms": [f"{config['os']}/{config['architecture']}"],
                                "contract": 1,
                                "erofs": {config["architecture"]: artifact["digest"]},
                            }
                        ],
                    }
                },
            )
        yield served

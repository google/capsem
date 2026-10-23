"""Local catalog acceptance uses canonical metadata and real producer output."""

import json
import ssl
import urllib.request

import pytest
from capsem_builder.release.package_inventory import from_rootfs

from tests.fixtures.oci.registry import layout_registry
from tests.fixtures.oci.test_pinned_image import _layout
from tests.images.test_package_inventory import image_tar


def test_official_catalogue_serves_exact_local_inventory_and_metadata(tmp_path):
    layout, digest, _ = _layout(tmp_path)
    source = tmp_path / "rootfs.tar"
    image_tar(source, {
        "var/lib/dpkg/status": b"Package: git\nStatus: install ok installed\nVersion: 2.39.5\nArchitecture: arm64\n",
        "usr/local/share/capsem/toolkit-inventory.json": b'{"architecture":"arm64","tools":[]}',
    })

    def produce(reference):
        output = tmp_path / "evidence"
        output.mkdir()
        (output / "packages.json").write_text(json.dumps(from_rootfs(source, reference, "arm64")))
        (output / "obom.cdx.json").write_text(json.dumps({
            "metadata": {"component": {"name": reference.split("@")[0], "version": digest}},
        }))
        return output

    with layout_registry(tmp_path, layout, "dev", official=True, evidence=produce) as (reference, ca, _):
        authority = reference.split("/")[0]
        context = ssl.create_default_context(cafile=str(ca))

        def get(path):
            with urllib.request.urlopen(f"https://{authority}{path}", context=context) as response:
                return json.load(response)

        catalog_manifest = get("/v2/library/catalog/manifests/stable")
        catalog = get(f"/v2/library/catalog/blobs/{catalog_manifest['layers'][0]['digest']}")
        entry = catalog["entries"]["dev"]
        assert entry["display_name"] == "Development"
        assert entry["description"] == "An agentless environment with development tools."
        assert entry["surface"] == "terminal"
        (version,) = entry["versions"]
        assert version["image"] == reference
        (evidence,) = version["evidence"]["arm64"]
        artifact = get(f"/v2/library/dev/manifests/{evidence['reference'].split('@')[1]}")
        assert artifact["subject"]["digest"] == digest
        inventory = get(f"/v2/library/dev/blobs/{artifact['layers'][0]['digest']}")
        assert inventory["subject"] == reference
        assert inventory["packages"] == [{"name": "git", "version": "2.39.5", "architecture": "arm64"}]
        referrers = get(f"/v2/library/dev/referrers/{digest}")
        assert {row["artifactType"] for row in referrers["manifests"]} == {
            "application/vnd.capsem.package-inventory.v1+json", "application/vnd.cyclonedx+json",
        }
        assert "erofs" not in version
        assert "abom" not in version


def test_local_catalogue_refuses_foreign_inventory_subject(tmp_path):
    layout, _, _ = _layout(tmp_path)
    output = tmp_path / "evidence"
    output.mkdir()
    (output / "packages.json").write_text(json.dumps({"subject": "elsewhere@sha256:" + "0" * 64}))
    with (
        pytest.raises(ValueError, match="subject"),
        layout_registry(tmp_path, layout, "dev", official=True, evidence=lambda _: output),
    ):
        pass


def test_official_metadata_cannot_misrepresent_the_image_surface(tmp_path):
    layout, _, _ = _layout(tmp_path, config={"Labels": {"org.capsem.surface": "xpra"}})
    with (
        pytest.raises(ValueError, match="surface"),
        layout_registry(tmp_path, layout, "dev", official=True),
    ):
        pass

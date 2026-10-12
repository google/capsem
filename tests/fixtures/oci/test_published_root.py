"""Prepared fixture roots retain their image subject and verified bytes."""

import json

import pytest

from tests.fixtures.oci.test_pinned_image import _layout


def test_a_prepared_root_is_bound_to_the_exact_platform_manifest(tmp_path):
    from tests.fixtures.oci.published_root import ready, stage

    layout, subject, _ = _layout(tmp_path)
    output = tmp_path / "output"
    output.mkdir()
    (output / "rootfs.erofs").write_bytes(b"prepared filesystem")
    (output / "obom.cdx.json").write_text(
        json.dumps({"metadata": {"component": {"name": "fixture", "version": subject}}})
    )
    stage(layout, output)
    manifest, blob = ready(layout)
    assert manifest["subject"]["digest"] == subject
    assert blob.read_bytes() == b"prepared filesystem"
    assert manifest["layers"][0]["size"] == blob.stat().st_size


@pytest.mark.parametrize("defect", ["payload", "subject", "missing"])
def test_a_missing_corrupt_or_foreign_prepared_root_is_refused(tmp_path, defect):
    from tests.fixtures.oci.published_root import ready, stage

    layout, subject, _ = _layout(tmp_path)
    output = tmp_path / "output"
    output.mkdir()
    (output / "rootfs.erofs").write_bytes(b"prepared filesystem")
    (output / "obom.cdx.json").write_text(
        json.dumps({"metadata": {"component": {"name": "fixture", "version": subject}}})
    )
    stage(layout, output)
    _, blob = ready(layout)
    if defect == "payload":
        blob.write_bytes(b"corrupt filesystem!")
    elif defect == "missing":
        blob.unlink()
    else:
        index = json.loads((layout / "index.json").read_text())
        index["manifests"][0]["digest"] = "sha256:" + "0" * 64
        (layout / "index.json").write_text(json.dumps(index))
    with pytest.raises((ValueError, FileNotFoundError)):
        ready(layout)


def test_a_producer_output_for_another_image_is_not_staged(tmp_path):
    from tests.fixtures.oci.published_root import stage

    layout, _, _ = _layout(tmp_path)
    output = tmp_path / "output"
    output.mkdir()
    (output / "rootfs.erofs").write_bytes(b"prepared filesystem")
    (output / "obom.cdx.json").write_text(
        json.dumps(
            {"metadata": {"component": {"name": "fixture", "version": "sha256:" + "0" * 64}}}
        )
    )
    with pytest.raises(ValueError, match="subject"):
        stage(layout, output)
    assert not (layout / "rootfs").exists()


def test_the_registry_serves_the_catalog_and_subject_bound_root(tmp_path):
    import ssl
    import urllib.request

    from tests.fixtures.oci.published_root import ready, stage
    from tests.fixtures.oci.registry import grant_exact, layout_registry

    layout, subject, _ = _layout(tmp_path)
    output = tmp_path / "output"
    output.mkdir()
    payload = b"prepared filesystem"
    (output / "rootfs.erofs").write_bytes(payload)
    (output / "obom.cdx.json").write_text(
        json.dumps({"metadata": {"component": {"name": "fixture", "version": subject}}})
    )
    stage(layout, output)
    with layout_registry(tmp_path, layout, "fixture") as (image, certificate, requests):
        authority = image.split("/", 1)[0]
        context = ssl.create_default_context(cafile=str(certificate))

        def get(path):
            with urllib.request.urlopen(f"https://{authority}{path}", context=context) as response:
                return response.read()

        artifact = json.loads(get("/v2/library/catalog/manifests/stable"))
        catalog = json.loads(get(f"/v2/library/catalog/blobs/{artifact['layers'][0]['digest']}"))
        (version,) = catalog["entries"]["fixture"]["versions"]
        assert version["image"] == image
        root = json.loads(get(f"/v2/library/fixture/manifests/{version['erofs']['arm64']}"))
        assert root == ready(layout)[0]
        assert root["subject"]["digest"] == subject
        assert get(f"/v2/library/fixture/blobs/{root['layers'][0]['digest']}") == payload
        home = tmp_path / "home"
        home.mkdir()
        grant_exact(
            home, image, catalog=f"{authority}/library/catalog:stable", certificate=certificate
        )
        import tomllib

        settings = tomllib.loads((home / "settings.toml").read_text())["images"]
        assert settings["catalog"] == f"{authority}/library/catalog:stable"
        assert settings["admit"] == [image]
        assert settings["sources"] == [image.split("@")[0]]
        assert not any("/referrers/" in path for path in requests)


@pytest.mark.parametrize("corrupt", [False, True])
def test_prefetch_retains_a_verified_published_root_before_offline_tests(tmp_path, corrupt):
    import ssl

    from tests.fixtures.oci.published_root import prepare, ready, stage
    from tests.fixtures.oci.registry import layout_registry

    source, subject, _ = _layout(tmp_path / "source")
    target, target_subject, _ = _layout(tmp_path / "target")
    assert target_subject == subject
    output = tmp_path / "output"
    output.mkdir()
    (output / "rootfs.erofs").write_bytes(b"prepared filesystem")
    (output / "obom.cdx.json").write_text(
        json.dumps({"metadata": {"component": {"name": "fixture", "version": subject}}})
    )
    stage(source, output)
    with layout_registry(tmp_path, source, "fixture") as (image, certificate, _):
        if corrupt:
            ready(source)[1].write_bytes(b"corrupt filesystem!")
            with pytest.raises(ValueError, match="bytes"):
                prepare(
                    target,
                    image.split("@")[0],
                    context=ssl.create_default_context(cafile=str(certificate)),
                )
            assert not (target / "rootfs").exists()
        else:
            prepare(
                target,
                image.split("@")[0],
                context=ssl.create_default_context(cafile=str(certificate)),
            )
            assert ready(target)[1].read_bytes() == b"prepared filesystem"

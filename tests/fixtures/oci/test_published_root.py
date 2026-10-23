"""Prepared fixture roots retain their image subject and verified bytes."""

import io
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
    assert not (layout / "rootfs-v2").exists()


@pytest.mark.parametrize("surface", ["terminal", "xpra"])
def test_the_registry_serves_the_catalog_and_subject_bound_root(tmp_path, surface):
    import ssl
    import urllib.request

    from tests.fixtures.oci.published_root import ready, stage
    from tests.fixtures.oci.registry import grant_exact, layout_registry

    labels = {"org.capsem.surface": "xpra"} if surface == "xpra" else {}
    layout, subject, _ = _layout(tmp_path, config={"Labels": labels})
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
        assert catalog["entries"]["fixture"]["surface"] == surface
        (version,) = catalog["entries"]["fixture"]["versions"]
        assert version["image"] == image
        root = json.loads(get(f"/v2/library/fixture/manifests/{version['erofs']['arm64']}"))
        assert root == ready(layout)[0]
        assert root["subject"]["digest"] == subject
        assert get(f"/v2/library/fixture/blobs/{root['layers'][0]['digest']}") == payload
        home = tmp_path / "home"
        home.mkdir()
        grant_exact(
            home,
            image,
            catalog=f"{authority}/library/catalog:stable",
            certificate=certificate,
        )
        import tomllib

        settings = tomllib.loads((home / "settings.toml").read_text())["images"]
        assert settings["catalog"] == f"{authority}/library/catalog:stable"
        assert settings["admit"] == [image]
        assert settings["sources"] == [image.split("@")[0]]
        assert not any("/referrers/" in path for path in requests)


@pytest.mark.parametrize("corrupt", [False, True])
@pytest.mark.parametrize("referrers_api", [True, False])
def test_prefetch_retains_a_verified_published_root_before_offline_tests(
    tmp_path, corrupt, referrers_api
):
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
    with layout_registry(tmp_path, source, "fixture", referrers_api=referrers_api) as (
        image,
        certificate,
        requests,
    ):
        if corrupt:
            ready(source)[1].write_bytes(b"corrupt filesystem!")
            with pytest.raises(ValueError, match="bytes"):
                prepare(
                    target,
                    image.split("@")[0],
                    context=ssl.create_default_context(cafile=str(certificate)),
                )
            assert not (target / "rootfs-v2").exists()
        else:
            prepare(
                target,
                image.split("@")[0],
                context=ssl.create_default_context(cafile=str(certificate)),
            )
            assert ready(target)[1].read_bytes() == b"prepared filesystem"
        fallback = f"/v2/library/fixture/manifests/{subject.replace(':', '-', 1)}"
        assert (fallback in requests) is not referrers_api


@pytest.mark.parametrize("status", [401, 403, 500])
def test_prefetch_does_not_fall_back_after_authentication_or_server_failure(
    tmp_path, monkeypatch, status
):
    import urllib.error
    from email.message import Message

    from tests.fixtures.oci import pinned_image, published_root

    layout, subject, _ = _layout(tmp_path)
    calls = []
    response = urllib.error.HTTPError(
        "https://registry.example/referrers", status, "refused", Message(), io.BytesIO()
    )

    class Client:
        def __init__(self, context):
            pass

        def open(self, url, token):
            calls.append(url)
            raise response

    monkeypatch.setattr(pinned_image, "_Registry", Client)
    monkeypatch.setattr(pinned_image, "_token", lambda *args: None)
    with pytest.raises(urllib.error.HTTPError) as error:
        published_root.prepare(layout, "registry.example/fixture")
    assert error.value.code == status
    assert error.value.closed, "a refused response must not retain its socket"
    assert calls == [f"https://registry.example/v2/fixture/referrers/{subject}"]
    assert not (layout / "rootfs-v2").exists()

"""Original producer evidence survives cleanup; no Docker/image build is run."""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.crypto import digest_file

from tests.fixtures.oci import image_build, published_root
from tests.fixtures.oci.test_pinned_image import _layout

REPOSITORY = "ghcr.io/google/capsem/capsem-debug"


def test_projected_original_bytes_equal_actual_atomic_output(tmp_path):
    from tests.fixtures.oci import producer_evidence

    layout, digest, _ = _layout(tmp_path / "subject")
    original = tmp_path / "original"
    output(original, digest)
    before = {path.name: path.read_bytes() for path in original.iterdir()}
    required = producer_evidence.required_bytes(
        published_root.subject(layout), original, REPOSITORY, "arm64", maximum=1048576
    )
    assert not (layout / "rootfs-v2").exists()
    assert not (layout / producer_evidence.DIRECTORY).exists()
    producer_evidence.stage(layout, original, REPOSITORY, "arm64", maximum=1048576)
    written = sum(
        path.stat().st_size
        for directory in ("rootfs-v2", producer_evidence.DIRECTORY)
        for path in producer_evidence.regular_files(layout / directory)
    )
    assert required == written
    assert {path.name: path.read_bytes() for path in original.iterdir()} == before


def output(
    path, digest, *, repository=REPOSITORY, architecture="arm64", root=b"ORIGINAL root bytes"
):
    path.mkdir()
    (path / "rootfs.erofs").write_bytes(root)
    (path / "rootfs.blake3").write_text("blake3:" + digest_file(path / "rootfs.erofs") + "\n")
    (path / "packages.json").write_text(
        json.dumps(
            {
                "subject": repository + "@" + digest,
                "architecture": architecture,
                "packages": [{"name": "filelock", "version": "4.2.1"}],
            }
        )
        + "\n"
    )
    (path / "obom.cdx.json").write_text(
        json.dumps(
            {
                "metadata": {"component": {"name": repository, "version": digest}},
                "components": [{"name": "filelock", "version": "4.2.1"}],
            }
        )
        + "\n"
    )
    return {
        name: (path / name).read_bytes()
        for name in ("packages.json", "obom.cdx.json", "rootfs.blake3")
    }


def setup(monkeypatch, tmp_path):
    from images.ci import rootfs

    source, digest, _ = _layout(tmp_path / "source")
    cache = tmp_path / "stage"
    cache.mkdir()
    settings = SimpleNamespace(
        name="capsem-debug",
        local_name="capsem-debug",
        repository=REPOSITORY,
        context="images/capsem-debug",
        base_context=None,
        evidence_max_bytes=1048576,
    )
    image = SimpleNamespace(
        section="debug_image",
        settings=lambda: settings,
        stage=lambda: cache,
        platform_name=lambda value: value or "linux/arm64",
        layout_path=lambda selected: cache / selected.split(":")[1],
    )
    calls, produced = [], {}

    def run(args, **kwargs):
        calls.append(args)
        if "--output" in args:
            destination = Path(args[args.index("--output") + 1].split("dest=")[1])
            shutil.copytree(source, destination)
        return SimpleNamespace(returncode=0)

    def produce(args):
        destination = Path(args[args.index("--out") + 1])
        # The canonical producer permits an existing fresh output directory.
        destination.rmdir()
        produced.update(output(destination, digest))
        calls.append(("producer", str(destination)))

    monkeypatch.setattr(image_build.subprocess, "run", run)
    monkeypatch.setattr(rootfs, "main", produce)
    return image, digest, calls, produced


def test_first_original_output_survives_temporary_cleanup_without_regeneration(
    monkeypatch, tmp_path
):
    image, digest, calls, produced = setup(monkeypatch, tmp_path)
    assert image_build.build(image) == digest
    layout = image.layout_path(digest)
    retained = layout / "producer-evidence"
    assert retained.is_dir(), "original canonical metadata was discarded by TemporaryDirectory"
    for name, content in produced.items():
        assert (retained / name).read_bytes() == content
    assert len([call for call in calls if call[0] == "producer"]) == 1
    assert published_root.ready(layout)[1].read_bytes() == b"ORIGINAL root bytes"
    assert not list(retained.glob("*.erofs")), "do not duplicate the multi-GB retained root"
    assert not list(image.stage().glob(".build-*")) and not list(
        image.stage().glob(".filesystem-*")
    )
    # An identical OCI generation reuses its original evidence/root, without
    # another canonical rootfs scanner/repack (the public build still loads Docker).
    calls.clear()
    assert image_build.build(image) == digest
    assert len(calls) == 2 and "--output" in calls[0] and "--load" in calls[1]
    for name, content in produced.items():
        assert (retained / name).read_bytes() == content


def retained(tmp_path):
    from tests.fixtures.oci import producer_evidence

    layout, digest, _ = _layout(tmp_path)
    source = tmp_path / "producer"
    expected = output(source, digest)
    producer_evidence.stage(layout, source, REPOSITORY, "arm64", maximum=1048576)
    return layout, digest, source, expected


@pytest.mark.parametrize(
    "defect",
    [
        "missing",
        "package",
        "obom",
        "package-version",
        "obom-component",
        "integrity",
        "root",
        "foreign",
        "platform",
        "traversal",
        "symlink",
        "layout-symlink",
        "blob-symlink",
    ],
)
def test_missing_corrupt_foreign_or_unsafe_reuse_never_succeeds(tmp_path, defect):
    from tests.fixtures.oci import producer_evidence

    layout, digest, source, _ = retained(tmp_path)
    metadata = layout / "producer-evidence"
    if defect == "missing":
        (metadata / "packages.json").unlink()
    elif defect == "package":
        (metadata / "packages.json").write_bytes(b'{"packages":[]}')
    elif defect == "obom":
        (metadata / "obom.cdx.json").write_bytes(b"{}")
    elif defect in {"package-version", "obom-component"}:
        file = metadata / ("packages.json" if defect == "package-version" else "obom.cdx.json")
        value = json.loads(file.read_text())
        value["packages" if defect == "package-version" else "components"][0]["version"] = (
            "tampered"
        )
        file.write_text(json.dumps(value))
    elif defect == "integrity":
        (metadata / "rootfs.blake3").write_text("blake3:" + "0" * 64 + "\n")
    elif defect == "root":
        published_root.ready(layout)[1].write_bytes(b"OTHER root bytes")
    elif defect in {"foreign", "platform", "traversal"}:
        file = metadata / "receipt.json"
        receipt = json.loads(file.read_text())
        if defect == "foreign":
            receipt["repository"] = "other.example/image"
        elif defect == "platform":
            receipt["platform"] = "linux/amd64"
        else:
            receipt["files"]["../outside"] = receipt["files"].pop("packages.json")
        file.write_text(json.dumps(receipt))
    elif defect == "symlink":
        (metadata / "packages.json").unlink()
        (metadata / "packages.json").symlink_to(source / "packages.json")
    elif defect == "layout-symlink":
        alias = tmp_path / "alias"
        alias.symlink_to(layout, target_is_directory=True)
        layout = alias
    else:
        file = layout / "blobs/sha256" / digest.split(":")[1]
        original = file.read_bytes()
        file.unlink()
        outside = tmp_path / "outside"
        outside.write_bytes(original)
        file.symlink_to(outside)
    with pytest.raises((AssertionError, ValueError, FileNotFoundError)):
        producer_evidence.ready(layout, REPOSITORY, "arm64", maximum=1048576)


@pytest.mark.parametrize(
    "defect", ["subject", "repository", "platform", "integrity", "symlink", "overflow"]
)
def test_bad_original_output_is_not_partially_published(tmp_path, defect):
    from tests.fixtures.oci import producer_evidence

    layout, digest, _ = _layout(tmp_path)
    source = tmp_path / "producer"
    output(
        source,
        digest,
        repository="other.example/image" if defect == "repository" else REPOSITORY,
        architecture="amd64" if defect == "platform" else "arm64",
    )
    if defect == "subject":
        (source / "obom.cdx.json").write_text(
            json.dumps(
                {"metadata": {"component": {"name": REPOSITORY, "version": "sha256:" + "0" * 64}}}
            )
        )
    elif defect == "integrity":
        (source / "rootfs.blake3").write_text("blake3:" + "0" * 64)
    elif defect == "symlink":
        (source / "packages.json").unlink()
        (source / "packages.json").symlink_to(tmp_path / "missing")
    with pytest.raises((ValueError, FileNotFoundError)):
        producer_evidence.stage(
            layout, source, REPOSITORY, "arm64", maximum=1 if defect == "overflow" else 1048576
        )
    assert not (layout / "producer-evidence").exists()
    assert not list(layout.glob(".evidence-*"))


def test_existing_original_root_cannot_be_replaced_by_regenerated_output(tmp_path):
    from tests.fixtures.oci import producer_evidence

    layout, _, source, expected = retained(tmp_path)
    old = published_root.ready(layout)[1].read_bytes()
    # A new producer writes a new inode; writing through the original hardlink
    # would instead deliberately corrupt the original cache blob.
    (source / "rootfs.erofs").unlink()
    (source / "rootfs.erofs").write_bytes(b"REGENERATED root bytes")
    (source / "rootfs.blake3").write_text("blake3:" + digest_file(source / "rootfs.erofs") + "\n")
    with pytest.raises(ValueError, match=r"root|original|differ"):
        producer_evidence.stage(layout, source, REPOSITORY, "arm64", maximum=1048576)
    assert published_root.ready(layout)[1].read_bytes() == old
    assert (layout / "producer-evidence/obom.cdx.json").read_bytes() == expected["obom.cdx.json"]


def test_failure_before_completion_leaves_no_managed_generation(monkeypatch, tmp_path):
    from images.ci import rootfs

    image, digest, _, _ = setup(monkeypatch, tmp_path)

    def failed(args):
        raise RuntimeError("synthetic producer failure")

    monkeypatch.setattr(rootfs, "main", failed)
    with pytest.raises(RuntimeError, match="producer failure"):
        image_build.build(image)
    assert not image.layout_path(digest).exists()
    assert list(image.stage().iterdir()) == []


def test_cached_layout_without_original_evidence_fails_without_rescan(monkeypatch, tmp_path):
    image, digest, calls, _ = setup(monkeypatch, tmp_path)
    source = tmp_path / "source" / "layout"
    shutil.copytree(source, image.layout_path(digest))
    with pytest.raises((ValueError, FileNotFoundError)):
        image_build.build(image)
    assert not any(call[0] == "producer" or "--load" in call for call in calls)


def test_only_allowlisted_original_metadata_is_retained_without_root_copy(tmp_path):
    from tests.fixtures.oci import producer_evidence

    layout, digest, _ = _layout(tmp_path)
    source = tmp_path / "producer"
    expected = output(source, digest)
    (source / "not-evidence.txt").write_bytes(b"never retained")
    producer_evidence.stage(layout, source, REPOSITORY, "arm64", maximum=1048576)
    metadata = producer_evidence.ready(layout, REPOSITORY, "arm64", maximum=1048576)
    assert {path.name for path in metadata.iterdir()} == {*expected, "receipt.json"}
    assert published_root.ready(layout)[1].samefile(source / "rootfs.erofs")
    shutil.rmtree(source)  # original producer TemporaryDirectory cleanup
    assert producer_evidence.ready(layout, REPOSITORY, "arm64", maximum=1048576) == metadata
    assert published_root.ready(layout)[1].read_bytes() == b"ORIGINAL root bytes"


def test_partial_metadata_write_failure_never_publishes_candidate(monkeypatch, tmp_path):
    image, digest, _, _ = setup(monkeypatch, tmp_path)
    original = Path.write_bytes

    def fail(path, data):
        if path.name == "obom.cdx.json" and path.parent.name == "producer-evidence":
            raise OSError("synthetic atomic metadata write failure")
        return original(path, data)

    monkeypatch.setattr(Path, "write_bytes", fail)
    with pytest.raises(OSError, match="atomic metadata"):
        image_build.build(image)
    assert not image.layout_path(digest).exists()
    assert list(image.stage().iterdir()) == []


def test_traversal_and_special_file_evidence_refused_before_read(tmp_path):
    import os

    from tests.fixtures.oci import producer_evidence

    layout, _, _, _ = retained(tmp_path)
    (tmp_path / "unused").mkdir()
    with pytest.raises(ValueError, match="traversal"):
        producer_evidence.ready(
            tmp_path / "unused" / ".." / "layout", REPOSITORY, "arm64", maximum=1048576
        )
    pipe = layout / "producer-evidence" / "pipe"
    os.mkfifo(pipe)
    with pytest.raises(ValueError, match="special"):
        producer_evidence.ready(layout, REPOSITORY, "arm64", maximum=1048576)


def test_root_stage_itself_refuses_a_different_original_generation(tmp_path):
    layout, digest, _ = _layout(tmp_path)
    first, second = tmp_path / "first", tmp_path / "second"
    output(first, digest)
    original, blob = published_root.stage(layout, first)
    output(second, digest, root=b"NEW generated root")
    with pytest.raises(ValueError, match="original root differs"):
        published_root.stage(layout, second)
    assert published_root.ready(layout)[0] == original
    assert blob.read_bytes() == b"ORIGINAL root bytes"


def test_evidence_budget_is_required_and_config_owned():
    from capsem_builder.gate.functionalschema import PinnedImageConfig

    from tests.fixtures.oci.pinned_image import debug_image, reference_image

    for image in (debug_image, reference_image):
        settings = image.settings()
        assert settings.evidence_max_bytes == 67108864
        data = settings.model_dump()
        data.pop("evidence_max_bytes")
        with pytest.raises(ValueError):
            PinnedImageConfig.model_validate(data)
        with pytest.raises(ValueError):
            PinnedImageConfig.model_validate({**data, "evidence_max_bytes": 0})


def test_tree_entry_budget_stops_actual_enumeration_before_exhaustion(monkeypatch, tmp_path):
    import os
    from contextlib import contextmanager

    from tests.fixtures.oci import producer_evidence

    tree = tmp_path / "many"
    tree.mkdir()
    for number in range(100):
        (tree / str(number)).write_bytes(b"fixture")
    original = os.scandir
    visits = []

    @contextmanager
    def counted(descriptor):
        with original(descriptor) as entries:

            def iterate():
                for entry in entries:
                    visits.append(entry.name)
                    yield entry

            yield iterate()

    monkeypatch.setattr(producer_evidence.os, "scandir", counted)
    with pytest.raises(ValueError, match="entry budget"):
        list(producer_evidence.regular_files(tree, maximum=5))
    assert len(visits) == 6


def test_tree_owner_refuses_nested_directory_alias_without_reading_private_sibling(tmp_path):
    from tests.fixtures.oci import producer_evidence

    tree = tmp_path / "tree"
    tree.mkdir()
    private = tmp_path / "private"
    private.mkdir()
    (private / "canary").write_bytes(b"private preserved")
    (tree / "alias").symlink_to(private, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        list(producer_evidence.regular_files(tree, maximum=10))
    assert (private / "canary").read_bytes() == b"private preserved"

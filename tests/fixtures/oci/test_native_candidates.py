"""Native candidate receipts bind actual producer bytes, not mutable tags."""

from __future__ import annotations

import json
from types import SimpleNamespace

import pytest

from tests.fixtures.oci import native_candidates as native
from tests.fixtures.oci import producer_evidence
from tests.fixtures.oci.test_pinned_image import _layout
from tests.fixtures.oci.test_producer_evidence import output


def test_new_app_receipts_require_current_original_build_binding(monkeypatch, tmp_path):
    from tests.fixtures.oci import native_inputs
    from tests.fixtures.oci.test_native_inputs import oci

    config, image, _, _ = fixture(monkeypatch, tmp_path)
    layout = tmp_path / "images/official"
    digest, _, _ = oci(layout)
    image.layout_path = lambda value: layout
    image.stage = lambda: layout.parent
    image.config = config
    settings = image.settings()
    settings.name = "codex-cli"
    settings.repository = "ghcr.io/google/capsem/codex-cli"
    produced = tmp_path / "official-output"
    output(produced, digest, repository=settings.repository, architecture="amd64")
    producer_evidence.stage(layout, produced, settings.repository, "amd64", maximum=1048576)
    image.section = "codex-cli"
    image.settings = lambda: settings
    base = native_inputs.BaseBinding.capture(
        layout, digest, "linux/amd64", "a" * 64, maximum=1048576
    )

    def key(root, name, base=None):
        return ("a" if name == "base" else "b") * 64

    monkeypatch.setattr(native_inputs, "workflow_key", key)
    original = native.source(config.root)
    with pytest.raises(ValueError, match="binding"):
        native.record(config, image, digest, "linux/amd64", original)
    build = native_inputs.BuildBinding(input_key="b" * 64, base=base)
    native.record(config, image, digest, "linux/amd64", original, build=build)
    receipt, found = native.selected(config, image)
    assert found == layout and receipt.build == build
    data = json.loads(native.receipt_path(config, image.section).read_text())
    data["build"]["input_key"] = "e" * 64
    native.receipt_path(config, image.section).write_text(json.dumps(data))
    with pytest.raises(ValueError, match="inputs"):
        native.selected(config, image)


@pytest.mark.parametrize(
    "args",
    [
        ["debug_image", "build", "--receipt", "--candidate-image", "codex-cli"],
        ["reference_image", "prepare", "--receipt", "--candidate-image", "codex-cli"],
        ["reference_image", "build", "--candidate-image", "codex-cli"],
        ["reference_image", "build", "--receipt", "--candidate-image", "claude-desktop"],
    ],
)
def test_candidate_cli_cannot_substitute_default_prepare_or_debug_pin(args):
    from tests.fixtures.oci import pinned_image

    with pytest.raises(SystemExit):
        pinned_image.main(args)


def test_candidate_cli_records_actual_producer_inside_its_existing_lease(monkeypatch, tmp_path):
    from contextlib import contextmanager

    from tests.fixtures.oci import native_build, pinned_image

    config = SimpleNamespace(host_arch=lambda: SimpleNamespace(docker_platform="linux/amd64"))
    original = native.Source(commit="a" * 40, tree="b" * 40)
    image, digest, binding = object(), "sha256:" + "c" * 64, object()
    held = False

    @contextmanager
    def produce(actual, name, platform):
        nonlocal held
        assert actual is config and name == "agy" and platform == "linux/amd64"
        held = True
        yield image, digest, binding
        held = False

    calls = []

    def record(actual, selected, subject, platform, source, *, build):
        assert held and (actual, selected, subject, platform, source, build) == (
            config,
            image,
            digest,
            "linux/amd64",
            original,
            binding,
        )
        calls.append(subject)

    monkeypatch.setattr(pinned_image, "_config", lambda: config)
    monkeypatch.setattr(native, "source", lambda _: original)
    monkeypatch.setattr(native_build, "produce", produce)
    monkeypatch.setattr(native, "record", record)
    pinned_image.main(["reference_image", "build", "--receipt", "--candidate-image", "agy"])
    assert calls == [digest] and not held


def fixture(monkeypatch, tmp_path):
    layout, digest, _ = _layout(tmp_path / "images" / "candidate", architecture="amd64")
    repository = "ghcr.io/google/capsem/capsem-debug"
    produced = tmp_path / "original"
    output(produced, digest, architecture="amd64")
    producer_evidence.stage(layout, produced, repository, "amd64", maximum=1048576)
    image = SimpleNamespace(
        section="debug_image",
        stage=lambda: layout.parent,
        layout_path=lambda value: layout if value == digest else layout.parent / "foreign",
        settings=lambda: SimpleNamespace(
            name="capsem-debug", repository=repository, evidence_max_bytes=1048576
        ),
    )
    monkeypatch.setattr(
        native, "source", lambda root: native.Source(commit="a" * 40, tree="b" * 40)
    )
    monkeypatch.setattr(native, "host_platform", lambda: "linux/amd64")
    config = SimpleNamespace(
        root=tmp_path,
        path=lambda value: tmp_path / value,
        functional=SimpleNamespace(
            native_images=SimpleNamespace(
                receipt_dir="receipts", receipt_max_bytes=1048576, platform="linux/amd64"
            )
        ),
    )
    return config, image, layout, digest


def test_actual_original_receipt_is_validated_and_selected(monkeypatch, tmp_path):
    config, image, layout, digest = fixture(monkeypatch, tmp_path)
    original = native.source(config.root)
    native.record(config, image, digest, "linux/amd64", original)
    receipt, found = native.selected(config, image)
    assert found == layout and receipt.subject == digest
    assert receipt.source == original and receipt.section == "debug_image"
    assert receipt.platform == "linux/amd64"
    assert receipt.root_integrity.startswith("blake3:")


@pytest.mark.parametrize(
    "field,value",
    [
        ("source", {"commit": "c" * 40, "tree": "b" * 40}),
        ("platform", "linux/arm64"),
        ("subject", "sha256:" + "e" * 64),
        ("layout_key", "../escape"),
        ("section", "reference_image"),
        ("image", "dev"),
        ("root_integrity", "blake3:" + "f" * 64),
    ],
)
def test_foreign_or_traversal_receipt_is_refused(monkeypatch, tmp_path, field, value):
    config, image, _, digest = fixture(monkeypatch, tmp_path)
    native.record(config, image, digest, "linux/amd64", native.source(config.root))
    path = native.receipt_path(config, image.section)
    data = json.loads(path.read_text())
    data[field] = value
    path.write_text(json.dumps(data))
    with pytest.raises(ValueError):
        native.selected(config, image)


def test_source_changes_during_production_cannot_record(monkeypatch, tmp_path):
    config, image, _, digest = fixture(monkeypatch, tmp_path)
    with pytest.raises(ValueError, match="source"):
        native.record(
            config, image, digest, "linux/amd64", native.Source(commit="c" * 40, tree="b" * 40)
        )
    assert not native.receipt_path(config, image.section).exists()


@pytest.mark.parametrize("kind", ["link", "fifo", "oversize"])
def test_receipt_is_bounded_regular_nofollow(monkeypatch, tmp_path, kind):
    import os

    config, image, _, digest = fixture(monkeypatch, tmp_path)
    native.record(config, image, digest, "linux/amd64", native.source(config.root))
    path = native.receipt_path(config, image.section)
    original = path.read_bytes()
    path.unlink()
    if kind == "link":
        target = tmp_path / "foreign.json"
        target.write_bytes(original)
        path.symlink_to(target)
    elif kind == "fifo":
        os.mkfifo(path)
    else:
        path.write_bytes(b"x" * 1048577)
    with pytest.raises((ValueError, OSError)):
        native.selected(config, image)


def test_existing_build_cli_records_actual_result_not_printed_output(monkeypatch, tmp_path, capsys):
    from tests.fixtures.oci import pinned_image

    config, image, layout, digest = fixture(monkeypatch, tmp_path)
    calls = []

    def build(selected):
        calls.append(selected)
        print("layout: /not-the-candidate sha256:" + "f" * 64)
        return digest

    image.build = build
    image.platform_name = lambda selected: "linux/amd64"
    monkeypatch.setattr(pinned_image, "ROOT", tmp_path)
    monkeypatch.setattr(pinned_image, "_config", lambda: config)
    monkeypatch.setitem(pinned_image.SECTIONS, "debug_image", image)
    pinned_image.main(["debug_image", "build", "--receipt"])
    receipt, selected = native.selected(config, image)
    assert calls == [None] and selected == layout and receipt.subject == digest
    assert "/not-the-candidate" in capsys.readouterr().out


@pytest.mark.parametrize(
    "defect", ["layout-link", "receipt-parent-link", "foreign-evidence", "non-native-host"]
)
def test_owned_paths_and_original_evidence_remain_required(monkeypatch, tmp_path, defect):
    config, image, layout, digest = fixture(monkeypatch, tmp_path)
    native.record(config, image, digest, "linux/amd64", native.source(config.root))
    if defect == "layout-link":
        original = layout.with_name("original-layout")
        layout.rename(original)
        layout.symlink_to(original)
    if defect == "receipt-parent-link":
        path = native.receipt_path(config, image.section).parent
        original = path.with_name("original-receipts")
        path.rename(original)
        path.symlink_to(original)
    if defect == "foreign-evidence":
        path = layout / "producer-evidence" / "receipt.json"
        value = json.loads(path.read_text())
        value["repository"] = "ghcr.io/foreign/image"
        path.write_text(json.dumps(value))
    if defect == "non-native-host":
        monkeypatch.setattr(native, "host_platform", lambda: "darwin/arm64")
    with pytest.raises(ValueError):
        native.selected(config, image)

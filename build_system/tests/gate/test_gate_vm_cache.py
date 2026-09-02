"""VM/install-image reuse is mandatory, receipted, protected, and bounded."""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest
from capsem_builder.cache.api import CacheOperation, CacheRequest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.registry import CacheRegistry
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from helpers.gate import RECORDED_IMAGE_ID, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def _config(tmp_path: Path):
    policy = PROJECT_ROOT / "config" / "cache.toml"
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    (tmp_path / "config" / "cache.toml").write_bytes(policy.read_bytes())
    prefix = CONFIG.prefix.model_copy(
        update={
            "parent": str(tmp_path / "prefixes"),
            "build_cache": str(tmp_path / "cache" / "target" / "prefix-products"),
            "cargo_target": str(tmp_path / "cache" / "target" / "cargo"),
        }
    )
    return CONFIG.model_copy(update={"root": tmp_path, "prefix": prefix})


def _source(tmp_path: Path):
    from capsem_builder.gate import sourcecapture

    root = tmp_path / "frozen-source"
    root.mkdir()
    return sourcecapture.SourceSnapshot(root, sourcecapture.SourceDigest("a" * 64))


def _helper():
    from capsem_builder.gate.installbuilder import InstallBuilderIdentity

    return InstallBuilderIdentity(
        "capsem-install-builder:helper",
        RECORDED_IMAGE_ID,
        "capsem-install-builder:helper",
    )


def test_a_valid_warm_install_image_must_not_run_construction(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import installbuilder, installimage, sourcecapture

    config = _config(tmp_path)
    source = _source(tmp_path)
    helper = _helper()
    runner = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(sourcecapture, "require_snapshot", lambda *_args: None)
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )

    first = installimage.build_source_image(runner, config, identity=helper, source=source)
    builds = len(runner.matching(r"docker build .*Dockerfile.install-test"))
    second = installimage.build_source_image(runner, config, identity=helper, source=source)

    assert builds == 1
    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == builds
    assert second.image_id == first.image_id
    assert any("cache hit is mandatory" in note for note in runner.notes)


def test_a_corrupt_install_receipt_rebuilds_instead_of_claiming_a_hit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import installbuilder, installimage, sourcecapture

    config = _config(tmp_path)
    source = _source(tmp_path)
    helper = _helper()
    runner = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(sourcecapture, "require_snapshot", lambda *_args: None)
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    installimage.build_source_image(runner, config, identity=helper, source=source)
    receipt = config.path(config.install.builder.source_identity_file)
    document = json.loads(receipt.read_text(encoding="utf-8"))
    receipt.write_text(json.dumps({**document, "image_size_bytes": 7}), encoding="utf-8")

    installimage.build_source_image(runner, config, identity=helper, source=source)

    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == 2
    assert any("byte size no longer matches" in note for note in runner.notes)


def test_an_expired_install_image_is_rebuilt(tmp_path: Path, monkeypatch) -> None:
    from capsem_builder.gate import installbuilder, installimage, sourcecapture

    config = _config(tmp_path)
    source = _source(tmp_path)
    helper = _helper()
    runner = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(sourcecapture, "require_snapshot", lambda *_args: None)
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    installimage.build_source_image(runner, config, identity=helper, source=source)
    receipt = config.path(config.install.builder.source_identity_file)
    document = json.loads(receipt.read_text(encoding="utf-8"))
    receipt.write_text(json.dumps({**document, "created_at": 0}), encoding="utf-8")

    installimage.build_source_image(runner, config, identity=helper, source=source)

    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == 2
    assert any("configured cache age" in note for note in runner.notes)


def test_a_non_finite_install_receipt_is_rebuilt(tmp_path: Path, monkeypatch) -> None:
    from capsem_builder.gate import installbuilder, installimage, sourcecapture

    config = _config(tmp_path)
    source = _source(tmp_path)
    helper = _helper()
    runner = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(sourcecapture, "require_snapshot", lambda *_args: None)
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    installimage.build_source_image(runner, config, identity=helper, source=source)
    receipt = config.path(config.install.builder.source_identity_file)
    document = json.loads(receipt.read_text(encoding="utf-8"))
    receipt.write_text(
        json.dumps({**document, "last_used_at": float("nan")}),
        encoding="utf-8",
    )

    installimage.build_source_image(runner, config, identity=helper, source=source)

    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == 2
    assert any("missing or invalid" in note for note in runner.notes)


def test_install_image_receipt_survives_a_successful_prefix(tmp_path: Path, monkeypatch) -> None:
    from capsem_builder.gate import buildcache, installbuilder, installimage, sourcecapture

    base = _config(tmp_path)
    first_root = Path(base.prefix.parent) / "aaaaaaaa"
    second_root = Path(base.prefix.parent) / "bbbbbbbb"
    first_root.mkdir(parents=True)
    first = base.model_copy(update={"root": first_root})
    source = _source(tmp_path)
    helper = _helper()
    runner = RecordingRunner(PROJECT_ROOT)
    monkeypatch.setattr(sourcecapture, "require_snapshot", lambda *_args: None)
    monkeypatch.setattr(
        installbuilder, "require_local_image", lambda *_args, **_kwargs: helper.input_key
    )
    installimage.build_source_image(runner, first, identity=helper, source=source)
    builds = len(runner.matching(r"docker build .*Dockerfile.install-test"))

    assert "cache/target/build/install-image" in buildcache.salvage(first, first_root)
    retained_receipt = first_root / first.install.builder.source_identity_file
    shared_receipt = buildcache.root(first) / first.install.builder.source_identity_file
    assert retained_receipt.is_file()
    assert shared_receipt.read_bytes() == retained_receipt.read_bytes()
    second_root.mkdir()
    assert "cache/target/build/install-image" in buildcache.lend(first, second_root)
    assert retained_receipt.is_file(), "lending erased a resumable prefix's Docker pins"
    second = first.model_copy(update={"root": second_root})
    installimage.build_source_image(runner, second, identity=helper, source=source)

    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == builds


def test_only_configured_tiny_authorities_are_duplicated_for_resume(tmp_path: Path) -> None:
    from capsem_builder.gate import buildcache

    config = _config(tmp_path)
    assert config.prefix.resumable == ("cache/target/build/install-image",)
    assert set(config.prefix.resumable) <= set(config.prefix.lent)
    assert set(config.prefix.resumable) <= set(config.prefix.produced)

    prefix_root = Path(config.prefix.parent) / "deadbeef"
    for relative in config.prefix.lent:
        product = prefix_root / relative
        product.mkdir(parents=True)
        (product / "authority").write_text(relative, encoding="utf-8")

    buildcache.salvage(config, prefix_root)

    for relative in config.prefix.lent:
        retained = prefix_root / relative
        assert retained.exists() is (relative in config.prefix.resumable)


def test_a_resumable_receipt_symlink_is_refused(tmp_path: Path) -> None:
    from capsem_builder.gate import buildcache

    config = _config(tmp_path)
    prefix_root = Path(config.prefix.parent) / "deadbeef"
    elsewhere = tmp_path / "unowned-receipt"
    elsewhere.mkdir()
    receipt = prefix_root / config.prefix.resumable[0]
    receipt.parent.mkdir(parents=True)
    receipt.symlink_to(elsewhere)

    with pytest.raises(GateError, match=r"resumable cache authority.*must not be a symlink"):
        buildcache.salvage(config, prefix_root)


def test_vm_cache_identity_and_profile_cannot_escape_the_typed_asset_store(
    tmp_path: Path,
) -> None:
    from capsem_builder.gate import assetstore

    config = _config(tmp_path)
    with pytest.raises(GateError, match="canonical digest"):
        assetstore.lane(config, "../outside", profile="code", arch=config.arch("x86_64"))
    with pytest.raises(GateError, match="plain name"):
        assetstore.lane(config, "a" * 64, profile="../outside", arch=config.arch("x86_64"))


def test_common_registry_evicts_old_asset_generation_but_preserves_selector(
    tmp_path: Path,
) -> None:
    configured = load_policy(PROJECT_ROOT)
    assets = configured.stages["assets"].model_copy(
        update={"maximum_count": 1, "warm_size_bytes": 2, "max_size_bytes": 3}
    )
    configured = configured.model_copy(
        update={"stages": {"assets": assets}, "runtimes": {}, "control": None}
    )
    paths = CachePaths(repository_root=tmp_path, policy=configured)
    generations = paths.stage("assets") / assets.entry_root
    old = generations / ("a" * 64)
    current = generations / ("b" * 64)
    for path in (old, current):
        payload = path / "code" / "build-x86_64" / "rootfs.erofs"
        payload.parent.mkdir(parents=True)
        payload.write_bytes(b"vm")
    selector = paths.root / "target" / "assets" / "code" / "build-x86_64"
    selector.parent.mkdir(parents=True)
    selector.symlink_to(current / "code" / "build-x86_64")

    registry = CacheRegistry(paths, configured)
    (result,) = registry.mutate(
        CacheRequest(
            operation=CacheOperation.ENFORCE,
            cache_id="assets",
            apply=True,
            reason="test asset maximum",
        )
    )

    assert result.action_count == 1
    assert not old.exists()
    assert current.is_dir()


def test_an_asset_selector_outside_the_vm_cache_has_no_valid_metadata(
    tmp_path: Path,
) -> None:
    from capsem_builder.gate import assetreceipt

    config = _config(tmp_path)
    arch = config.arch("x86_64")
    identity = "a" * 64
    outside = tmp_path / "outside" / "build-x86_64"
    product = outside / arch.name / "payload"
    product.parent.mkdir(parents=True)
    product.write_bytes(b"x")
    (outside / config.assets.lane_receipt).write_text(
        json.dumps(
            {
                "schema": assetreceipt.SCHEMA,
                "profile": "code",
                "architecture": arch.name,
                "stage": assetreceipt.BUILD_STAGE,
                "input_digest": identity,
                "files": {},
                "created_at": time.time(),
                "last_used_at": time.time(),
                "size_bytes": 1,
            }
        ),
        encoding="utf-8",
    )
    selector = config.path(config.assets.test_root) / "code" / "build-x86_64"
    selector.parent.mkdir(parents=True)
    selector.symlink_to(outside)

    assert assetreceipt.cache_metadata(config, selector) is None


def test_equal_inputs_in_two_prefixes_select_one_vm_image_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import assetstore

    base = _config(tmp_path)
    monkeypatch.setenv(base.environment.source_checkout, str(tmp_path))
    identity = "a" * 64
    roots = tuple(Path(base.prefix.parent) / name for name in ("11111111", "22222222"))
    selected = []
    for checkout in roots:
        checkout.mkdir(parents=True)
        (checkout / "config").mkdir()
        (checkout / "config" / "cache.toml").write_bytes(
            (tmp_path / "config" / "cache.toml").read_bytes()
        )
        config = base.model_copy(update={"root": checkout})
        assetstore.materialize(config, ("code",), identity)
        selected.append(config.path(config.assets.test_root) / "code" / "build-x86_64")

    assert selected[0].resolve() == selected[1].resolve()
    assert (
        selected[0].resolve()
        == assetstore.lane(base, identity, profile="code", arch=base.arch("x86_64")).resolve()
    )


def test_retained_prefix_receipts_pin_both_source_and_helper_images(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import imagecache

    config = _config(tmp_path)
    monkeypatch.delenv(config.environment.source_checkout, raising=False)
    retained = Path(config.prefix.parent) / "deadbeef"
    receipt = retained / config.install.builder.source_identity_file
    receipt.parent.mkdir(parents=True)
    receipt.write_text(
        json.dumps(
            {
                "schema": imagecache.RECEIPT_SCHEMA,
                "input_key": "capsem-install-test:source",
                "input_digest": "a" * 64,
                "image_id": RECORDED_IMAGE_ID,
                "image_reference": "capsem-install-test:source",
                "helper_input_key": "capsem-install-builder:helper",
                "helper_image_id": RECORDED_IMAGE_ID,
                "source_digest": "b" * 64,
                "runtime_digest": "c" * 64,
                "platform": "linux/amd64",
                "image_size_bytes": 1,
                "created_at": 1,
                "last_used_at": 1,
            }
        ),
        encoding="utf-8",
    )

    assert imagecache.protected_tags(config, "capsem-install-test", field="input_key") == (
        "capsem-install-test:source",
    )
    assert imagecache.protected_tags(
        config, "capsem-install-builder", field="helper_input_key"
    ) == ("capsem-install-builder:helper",)


def test_the_active_source_checkout_receipt_pins_its_docker_images(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import imagecache

    config = _config(tmp_path)
    source = tmp_path / "active-source"
    receipt = source / config.install.builder.source_identity_file
    receipt.parent.mkdir(parents=True)
    receipt.write_text(
        json.dumps(
            {
                "schema": imagecache.RECEIPT_SCHEMA,
                "input_key": "capsem-install-test:source",
                "input_digest": "a" * 64,
                "image_id": RECORDED_IMAGE_ID,
                "image_reference": "capsem-install-test:source",
                "helper_input_key": "capsem-install-builder:helper",
                "helper_image_id": RECORDED_IMAGE_ID,
                "source_digest": "b" * 64,
                "runtime_digest": "c" * 64,
                "platform": "linux/amd64",
                "image_size_bytes": 1,
                "created_at": 1,
                "last_used_at": 1,
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv(config.environment.source_checkout, str(source))

    assert imagecache.protected_tags(config, "capsem-install-test", field="input_key") == (
        "capsem-install-test:source",
    )
    assert imagecache.protected_tags(
        config, "capsem-install-builder", field="helper_input_key"
    ) == ("capsem-install-builder:helper",)


def test_a_partial_or_malformed_receipt_cannot_pin_a_docker_image(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import imagecache

    config = _config(tmp_path)
    monkeypatch.delenv(config.environment.source_checkout, raising=False)
    receipt = Path(config.prefix.parent) / "deadbeef" / config.install.builder.source_identity_file
    receipt.parent.mkdir(parents=True)
    receipt.write_text(
        json.dumps(
            {
                "schema": imagecache.RECEIPT_SCHEMA,
                "input_key": "capsem-install-test:attacker-chosen",
            }
        ),
        encoding="utf-8",
    )

    assert imagecache.protected_tags(config, "capsem-install-test", field="input_key") == ()


def test_cache_reclaim_passes_protected_receipts_to_the_policy() -> None:
    from capsem_builder.gate.cachecontrol import CacheControl

    runner = RecordingRunner(PROJECT_ROOT)
    CacheControl(runner).reclaim(
        "capsem-install-test",
        keep="capsem-install-test:current",
        protect=("capsem-install-test:old", "capsem-install-test:current"),
    )

    assert runner.ran(
        r"reclaim-image capsem-install-test --keep capsem-install-test:current "
        r"--protect capsem-install-test:old --apply"
    )


def test_vm_and_asset_cache_contracts_are_declared_through_common_api() -> None:
    from capsem_builder.gate.cachecontrol import CacheControl

    policy = load_policy(PROJECT_ROOT)
    registry = CacheRegistry(CachePaths(repository_root=PROJECT_ROOT, policy=policy), policy)
    assets = registry.contract("assets")
    assert assets.max_size_bytes == 200 * 1024**3
    assert assets.warm_size_bytes == 200 * 1024**3
    cache = CacheControl(RecordingRunner(PROJECT_ROOT))
    source_limits = cache.image_policy("capsem-install-test")
    helper_limits = cache.image_policy("capsem-install-builder")
    ordinary_receipt_lineages = CONFIG.prefix.keep + 2
    assert source_limits.maximum_count == ordinary_receipt_lineages
    assert helper_limits.maximum_count == ordinary_receipt_lineages
    assert source_limits.maximum_age_seconds == 336 * 3600
    assert source_limits.max_size_bytes == 200 * 1024**3
    assert helper_limits.maximum_age_seconds > 0
    assert helper_limits.max_size_bytes > 0

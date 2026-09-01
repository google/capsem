"""VM/install-image reuse is mandatory, receipted, protected, and bounded."""

from __future__ import annotations

import json
import os
import time
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from capsem_builder.policy.cachepolicy import CacheLimits, CacheProduct, plan_reclaim
from helpers.gate import RECORDED_IMAGE_ID, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)


def _config(tmp_path: Path):
    prefix = CONFIG.prefix.model_copy(
        update={
            "parent": str(tmp_path / "prefixes"),
            "build_cache": str(tmp_path / "cache" / "target" / "prefix-products"),
            "vm_image_cache": str(tmp_path / "cache" / "target" / "assets" / "generations"),
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


def test_install_image_receipt_survives_a_successful_prefix(
    tmp_path: Path, monkeypatch
) -> None:
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

    assert "cache/target/install-image" in buildcache.salvage(first, first_root)
    retained_receipt = first_root / first.install.builder.source_identity_file
    shared_receipt = buildcache.root(first) / first.install.builder.source_identity_file
    assert retained_receipt.is_file()
    assert shared_receipt.read_bytes() == retained_receipt.read_bytes()
    second_root.mkdir()
    assert "cache/target/install-image" in buildcache.lend(first, second_root)
    assert retained_receipt.is_file(), "lending erased a resumable prefix's Docker pins"
    second = first.model_copy(update={"root": second_root})
    installimage.build_source_image(runner, second, identity=helper, source=source)

    assert len(runner.matching(r"docker build .*Dockerfile.install-test")) == builds


def test_only_configured_tiny_authorities_are_duplicated_for_resume(tmp_path: Path) -> None:
    from capsem_builder.gate import buildcache

    config = _config(tmp_path)
    assert config.prefix.resumable == ("cache/target/install-image",)
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


def test_vm_cache_roots_and_identities_cannot_escape_their_owned_sibling(
    tmp_path: Path,
) -> None:
    from capsem_builder.gate import assetcache
    from capsem_builder.gate.prefixschema import PrefixConfig

    config = _config(tmp_path)
    fields = config.prefix.model_dump()
    fields["vm_image_cache"] = "{parent}-images/../.."
    with pytest.raises(ValueError, match="repository cache"):
        PrefixConfig.model_validate(fields)

    with pytest.raises(GateError, match="canonical digest"):
        assetcache.lane(config, "../outside", profile="code", arch=config.arch("x86_64"))


def test_lru_reclaim_never_evicts_a_pinned_vm_product() -> None:
    plan = plan_reclaim(
        (
            CacheProduct("old", 40, 1, 2),
            CacheProduct("recent", 40, 2, 4),
            CacheProduct("pinned", 40, 1, 1, protected=True),
        ),
        CacheLimits(maximum_count=2, maximum_age_seconds=100, maximum_bytes=80),
        now=10,
    )

    assert plan.evict == ("old",)
    assert plan.violations == ()


def test_pinned_overflow_is_reported_instead_of_deleted() -> None:
    plan = plan_reclaim(
        (
            CacheProduct("one", 60, 1, 1, protected=True),
            CacheProduct("two", 60, 1, 2, protected=True),
        ),
        CacheLimits(maximum_count=1, maximum_age_seconds=5, maximum_bytes=100),
        now=10,
    )

    assert plan.evict == ()
    assert plan.violations == (
        "count 2 exceeds 1",
        "bytes 120 exceeds 100",
        "expired protected products: one, two",
    )


def test_future_cache_clocks_fail_closed() -> None:
    plan = plan_reclaim(
        (
            CacheProduct("unpinned", 1, 20, 20),
            CacheProduct("pinned", 1, 20, 20, protected=True),
        ),
        CacheLimits(maximum_count=2, maximum_age_seconds=100, maximum_bytes=10),
        now=10,
    )

    assert plan.evict == ("unpinned",)
    assert plan.violations == ("future-dated protected products: pinned",)


def test_asset_cache_evicts_unpinned_lru_before_a_current_vm_lane(tmp_path: Path) -> None:
    from capsem_builder.gate import assetcache

    config = _config(tmp_path)
    cache = config.assets.cache.model_copy(
        update={"maximum_count": 1, "maximum_age_hours": 1, "maximum_bytes": 1024}
    )
    config = config.model_copy(update={"assets": config.assets.model_copy(update={"cache": cache})})
    root = assetcache.root(config)
    old = root / ("a" * 64) / "old-profile" / "build-x86_64"
    current = root / ("b" * 64) / "code" / "build-x86_64"
    for path in (old, current):
        path.mkdir(parents=True)
        (path / "product").write_bytes(b"vm")
    now = time.time()
    os.utime(old, (now - 7200, now - 7200))

    removed = assetcache.enforce(config, protected=frozenset({current}))

    assert removed == (old,)
    assert current.is_dir()


def test_a_resumable_prefix_pins_its_vm_image_generation(tmp_path: Path) -> None:
    from capsem_builder.gate import assetcache

    config = _config(tmp_path)
    policy = config.assets.cache.model_copy(
        update={"maximum_count": 1, "maximum_age_hours": 1, "maximum_bytes": 1024}
    )
    config = config.model_copy(
        update={"assets": config.assets.model_copy(update={"cache": policy})}
    )
    cached = assetcache.root(config) / ("a" * 64) / "code" / "build-x86_64"
    cached.mkdir(parents=True)
    (cached / "rootfs.erofs").write_bytes(b"vm")
    old = Path(config.prefix.parent) / "deadbeef"
    selector = old / config.assets.test_root / "code" / "build-x86_64"
    selector.parent.mkdir(parents=True)
    selector.symlink_to(cached)
    stale = time.time() - 7200
    os.utime(cached, (stale, stale))

    with pytest.raises(GateError, match="active or resumable"):
        assetcache.enforce(config, protected=frozenset())

    assert cached.is_dir(), "a resumable qualification lost the VM image it references"


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


def test_equal_inputs_in_two_prefixes_select_one_vm_image_generation(tmp_path: Path) -> None:
    from capsem_builder.gate import assetcache

    base = _config(tmp_path)
    identity = "a" * 64
    roots = tuple(Path(base.prefix.parent) / name for name in ("11111111", "22222222"))
    selected = []
    for checkout in roots:
        checkout.mkdir(parents=True)
        config = base.model_copy(update={"root": checkout})
        assetcache.materialize(config, ("code",), identity)
        selected.append(
            config.path(config.assets.test_root) / "code" / "build-x86_64"
        )

    assert selected[0].resolve() == selected[1].resolve()
    assert selected[0].resolve() == assetcache.lane(
        base, identity, profile="code", arch=base.arch("x86_64")
    ).resolve()


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

    assert imagecache.protected_tags(
        config, "capsem-install-test", field="input_key"
    ) == ("capsem-install-test:source",)
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

    assert imagecache.protected_tags(
        config, "capsem-install-test", field="input_key"
    ) == ("capsem-install-test:source",)
    assert imagecache.protected_tags(
        config, "capsem-install-builder", field="helper_input_key"
    ) == ("capsem-install-builder:helper",)


def test_a_partial_or_malformed_receipt_cannot_pin_a_docker_image(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import imagecache

    config = _config(tmp_path)
    monkeypatch.delenv(config.environment.source_checkout, raising=False)
    receipt = (
        Path(config.prefix.parent)
        / "deadbeef"
        / config.install.builder.source_identity_file
    )
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

    assert imagecache.protected_tags(
        config, "capsem-install-test", field="input_key"
    ) == ()


def test_storage_reclaim_passes_protected_receipts_to_the_policy() -> None:
    from capsem_builder.gate.storage import Storage

    runner = RecordingRunner(PROJECT_ROOT)
    Storage(runner).reclaim(
        "capsem-install-test",
        keep="capsem-install-test:current",
        protect=("capsem-install-test:old", "capsem-install-test:current"),
    )

    assert runner.ran(
        r"docker-storage-policy\.py reclaim --resource capsem-install-test "
        r"--keep capsem-install-test:current --protect capsem-install-test:old"
    )


def test_vm_and_asset_cache_bounds_are_declared() -> None:
    from capsem_builder.gate.storage import Storage

    assert CONFIG.assets.cache.maximum_count > len(CONFIG.architectures)
    assert CONFIG.assets.cache.maximum_age_hours > 0
    assert CONFIG.assets.cache.maximum_bytes > 0
    storage = Storage(RecordingRunner(PROJECT_ROOT))
    source_limits = storage.image_limits("capsem-install-test")
    helper_limits = storage.image_limits("capsem-install-builder")
    ordinary_receipt_lineages = CONFIG.prefix.keep + 2
    assert source_limits.maximum_count == ordinary_receipt_lineages
    assert helper_limits.maximum_count == ordinary_receipt_lineages
    assert source_limits.maximum_age_seconds == 336 * 3600
    assert source_limits.maximum_bytes == 96 * 1024**3
    assert helper_limits.maximum_age_seconds > 0
    assert helper_limits.maximum_bytes > 0

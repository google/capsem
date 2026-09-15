"""The test image cache fails closed on altered bytes and architecture drift."""

import hashlib
import json

import pytest

from tests.fixtures.oci import prepare_redis


def test_pins_are_native_and_unsupported_platforms_fail():
    for platform in ("linux/arm64", "linux/amd64"):
        pin = prepare_redis.native_pin(platform)
        assert pin["platform"] == platform
        assert pin["image"].startswith("redis@sha256:")
    with pytest.raises(ValueError, match="unsupported"):
        prepare_redis.native_pin("linux/ppc64le")
    for platform in ("linux/arm64", "linux/amd64"):
        pin = prepare_redis.native_pin(platform, "iperf3")
        assert pin["image"].startswith("networkstatic/iperf3@sha256:")
    with pytest.raises(ValueError, match="unknown"):
        prepare_redis.native_pin("linux/arm64", "busybox")


def test_cache_requires_matching_pin_and_verified_bytes(tmp_path):
    pin = prepare_redis.native_pin("linux/arm64")
    assert not prepare_redis.cached(tmp_path, pin)
    archive = tmp_path / "redis-rootfs.tar.gz"
    metadata = tmp_path / "redis-image.json"
    archive.write_bytes(b"pinned image bytes")
    metadata.write_text(
        json.dumps(
            {**pin, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}
        )
    )
    assert prepare_redis.cached(tmp_path, pin)
    assert not prepare_redis.cached(tmp_path, prepare_redis.native_pin("linux/amd64"))
    archive.write_bytes(b"tampered")
    assert not prepare_redis.cached(tmp_path, pin)
    metadata.write_text("{")
    with pytest.raises(json.JSONDecodeError):
        prepare_redis.cached(tmp_path, pin)


def test_each_image_caches_under_its_own_name(tmp_path):
    pin = prepare_redis.native_pin("linux/arm64", "iperf3")
    archive = tmp_path / "iperf3-rootfs.tar.gz"
    archive.write_bytes(b"iperf bytes")
    (tmp_path / "iperf3-image.json").write_text(
        json.dumps(
            {**pin, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}
        )
    )
    assert prepare_redis.cached(tmp_path, pin, "iperf3")
    assert not prepare_redis.cached(tmp_path, prepare_redis.native_pin("linux/arm64"))

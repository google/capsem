"""Release runtime architecture and ownership contract gates.

The runtime is the one VM release unit: per architecture it carries software
rows, images and evidence, and nothing else -- no config, no identity of its
own. The readiness checker is what holds a live channel to that shape.
"""

from __future__ import annotations

import hashlib
import importlib
import json
from copy import deepcopy
from typing import Any

import pytest
from blake3 import blake3
from capsem_builder.release.tools import check_remote_release_readiness as READINESS

SITE = "https://release.capsem.test"
REVISION = "0.7.0-0123456789ab"


def _checker(monkeypatch: pytest.MonkeyPatch):
    checker = importlib.reload(READINESS)
    monkeypatch.setattr(checker, "check_release_graph_artifact", lambda *_args, **_kwargs: [])
    return checker


def _digest(payload: bytes) -> dict[str, str]:
    return {"sha256": hashlib.sha256(payload).hexdigest(), "blake3": blake3(payload).hexdigest()}


def _software_row(name: str, version: str, arch: str, evidence: str) -> dict[str, Any]:
    row = {
        "name": name,
        "version": version,
        "source": "debian",
        "architecture": arch,
        "evidence": evidence,
    }
    return {**row, "digest": _digest(json.dumps(row, separators=(",", ":")).encode())}


def _architecture(arch: str) -> dict[str, Any]:
    base = f"/runtime/releases/stable/{REVISION}/{arch}"
    inventory = f"{base}/software-inventory.json"
    return {
        "architecture": arch,
        "package_inventory_revision": REVISION,
        "image_revision": REVISION,
        "software": [
            _software_row("python", "3.13.5", arch, inventory),
            _software_row("nodejs", "22.17.0", arch, inventory),
        ],
        "images": [
            {
                "kind": kind,
                "name": name,
                "url": f"{base}/{name}",
                "bytes": 1,
                "digest": _digest(f"{arch}-{name}".encode()),
                "status": "current",
            }
            for kind, name in (
                ("kernel", "vmlinuz"),
                ("initrd", "initrd.img"),
                ("rootfs", "rootfs.erofs"),
            )
        ],
        "evidence": [
            {
                "kind": "software_inventory",
                "url": inventory,
                "bytes": 1,
                "digest": _digest(f"{arch}-inventory".encode()),
            },
            *(
                {
                    "kind": kind,
                    "url": f"{base}/{kind}.cdx.json",
                    "bytes": 1,
                    "digest": _digest(f"{arch}-{kind}".encode()),
                }
                for kind in ("abom", "obom")
            ),
        ],
    }


def _runtime() -> dict[str, Any]:
    return {
        "revision": REVISION,
        "status": "current",
        "min_capsem_version": "0.7.0",
        "architectures": [_architecture("arm64"), _architecture("x86_64")],
    }


def test_a_well_formed_runtime_passes(monkeypatch: pytest.MonkeyPatch) -> None:
    checker = _checker(monkeypatch)

    assert checker.check_release_graph_runtime(SITE, _runtime()) == []


def test_runtime_carries_no_config_or_identity_of_its_own(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    runtime["id"] = "runtime"
    runtime["name"] = "Runtime"
    runtime["architectures"][0]["config"] = [{"kind": "settings"}]

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert "runtime must not declare id" in failures
    assert "runtime must not declare name" in failures
    assert "runtime architecture arm64 must not publish config" in failures


def test_image_evidence_is_architecture_scoped(monkeypatch: pytest.MonkeyPatch) -> None:
    checker = _checker(monkeypatch)
    for arch, wrong in (("arm64", "x86_64"), ("x86_64", "arm64")):
        runtime = _runtime()
        architecture = next(
            item for item in runtime["architectures"] if item["architecture"] == arch
        )
        evidence = next(item for item in architecture["evidence"] if item["kind"] == "obom")
        evidence["url"] = evidence["url"].replace(f"/{arch}/", f"/{wrong}/")

        failures = checker.check_release_graph_runtime(SITE, runtime)

        assert f"runtime architecture {arch} evidence obom url must include /{arch}/" in failures


def test_image_removal_is_absence_not_status(monkeypatch: pytest.MonkeyPatch) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    architecture = runtime["architectures"][0]
    removed = next(item for item in architecture["images"] if item["kind"] == "initrd")
    architecture["images"].append({**removed, "status": "removed"})

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert any("status removed is not allowed" in failure for failure in failures)


def test_every_architecture_ships_the_complete_image_set(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checker = _checker(monkeypatch)
    for kind in ("kernel", "initrd", "rootfs"):
        runtime = _runtime()
        architecture = runtime["architectures"][1]
        architecture["images"] = [
            image for image in architecture["images"] if image["kind"] != kind
        ]

        failures = checker.check_release_graph_runtime(SITE, runtime)

        assert f"runtime architecture x86_64 images missing {kind}" in failures


@pytest.mark.parametrize(
    ("version", "expected"),
    [
        ("", "version missing"),
        ("unversioned", "version is unversioned"),
        ("unknown", "version is unknown"),
        ("latest", "version is latest"),
    ],
)
def test_software_versions_are_real(
    monkeypatch: pytest.MonkeyPatch,
    version: str,
    expected: str,
) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    runtime["architectures"][0]["software"][0]["version"] = version

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert f"runtime architecture arm64 software python {expected}" in failures


def test_software_rows_are_architecture_owned(monkeypatch: pytest.MonkeyPatch) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    runtime["architectures"][0]["software"][0]["architecture"] = "all"

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert (
        "runtime architecture arm64 software python architecture mismatch: "
        "expected arm64, got all" in failures
    )


def test_software_rows_do_not_reuse_inventory_digest(monkeypatch: pytest.MonkeyPatch) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    architecture = runtime["architectures"][0]
    architecture["software"][0]["digest"] = next(
        item["digest"] for item in architecture["evidence"] if item["kind"] == "software_inventory"
    )

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert (
        "runtime architecture arm64 software python digest reuses software_inventory "
        "evidence digest" in failures
    )


def test_software_inventory_rejects_repeated_hashes_for_distinct_rows(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checker = _checker(monkeypatch)
    runtime = _runtime()
    first, second = runtime["architectures"][0]["software"][:2]
    second["digest"] = deepcopy(first["digest"])

    failures = checker.check_release_graph_runtime(SITE, runtime)

    assert (
        "runtime architecture arm64 software "
        f"digest {first['digest']['sha256']} is reused by {first['name']} and {second['name']}"
        in failures
    )


def test_runtime_artifacts_live_under_runtime_or_asset_releases(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    checker = importlib.reload(READINESS)
    monkeypatch.setattr(
        checker,
        "fetch_bytes",
        lambda _url: checker.FetchBytes(data=b"x"),
    )
    image = deepcopy(_architecture("arm64")["images"][0])
    image["digest"] = _digest(b"x")

    assert checker.check_release_graph_artifact(SITE, image, "runtime image") == []

    retired = {**image, "url": f"/elsewhere/releases/stable/{REVISION}/arm64/vmlinuz"}
    failures = checker.check_release_graph_artifact(SITE, retired, "runtime image")

    assert any(
        "must be under one of /assets/releases/, /runtime/releases/" in failure
        for failure in failures
    )

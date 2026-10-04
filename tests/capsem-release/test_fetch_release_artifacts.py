from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import blake3
import pytest
from capsem_builder.release.tools import fetch_channel_source_manifest as SOURCE
from capsem_builder.release.tools import fetch_release_artifacts as FETCH
from capsem_builder.release.tools import prove_release_runtime_assets as BOOT
from capsem_builder.release.tools import stage_release_test_inputs as STAGE
from capsem_builder.release.tools import verify_release_inputs as VERIFY

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SCRIPT = ROOT / "build_system" / "scripts" / "release" / "fetch-channel-source-manifest.py"
RUNTIME_REVISION = "0.7.0-0123456789ab"


def _digest(payload: bytes) -> dict[str, str]:
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }


def test_channel_source_script_bootstraps_checkout_src_in_isolated_python(
    tmp_path: Path,
) -> None:
    completed = subprocess.run(
        [sys.executable, "-I", str(SOURCE_SCRIPT), "--help"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr
    assert "--source-commit" in completed.stdout


def test_latest_channel_source_manifest_is_selected_without_parallel_state() -> None:
    releases = [
        {
            "draft": False,
            "prerelease": False,
            "published_at": "2026-07-23T12:00:00Z",
            "assets": [
                {
                    "name": "channel-source-nightly.json",
                    "url": "https://api.github.test/assets/older",
                }
            ],
        },
        {
            "draft": False,
            "prerelease": False,
            "published_at": "2026-07-24T12:00:00Z",
            "assets": [
                {
                    "name": "channel-source-stable.json",
                    "url": "https://api.github.test/assets/stable",
                },
                {
                    "name": "channel-source-nightly.json",
                    "url": "https://api.github.test/assets/newer",
                },
            ],
        },
        {
            "draft": True,
            "prerelease": False,
            "published_at": "2026-07-25T12:00:00Z",
            "assets": [
                {
                    "name": "channel-source-nightly.json",
                    "url": "https://api.github.test/assets/draft",
                }
            ],
        },
    ]

    selected = SOURCE.select_latest_source_asset(releases, "nightly")

    assert selected == {
        "name": "channel-source-nightly.json",
        "url": "https://api.github.test/assets/newer",
    }
    assert SOURCE.select_latest_source_asset(releases, "experimental") is None


def test_source_selection_uses_asset_mutation_time_for_resumed_publication() -> None:
    releases = [
        {
            "draft": False,
            "prerelease": False,
            "published_at": "2026-07-23T12:00:00Z",
            "assets": [
                {
                    "id": 43,
                    "name": "channel-source-nightly.json",
                    "created_at": "2026-07-26T12:00:00Z",
                    "updated_at": "2026-07-26T12:00:00Z",
                    "url": "https://api.github.test/assets/resumed-runtime",
                }
            ],
        },
        {
            "draft": False,
            "prerelease": False,
            "published_at": "2026-07-25T12:00:00Z",
            "assets": [
                {
                    "id": 42,
                    "name": "channel-source-nightly.json",
                    "created_at": "2026-07-25T12:00:00Z",
                    "updated_at": "2026-07-25T12:00:00Z",
                    "url": "https://api.github.test/assets/earlier-binary",
                }
            ],
        },
    ]

    selected = SOURCE.select_latest_source_asset(releases, "nightly")

    assert selected == releases[0]["assets"][0]


def test_channel_source_discovery_paginates_past_daily_nightly_releases(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    page_one = [
        {
            "draft": False,
            "prerelease": False,
            "published_at": f"2026-07-{index + 1:02d}T12:00:00Z",
            "assets": [],
        }
        for index in range(100)
    ]
    stable_source = {
        "draft": False,
        "prerelease": False,
        "published_at": "2026-04-01T12:00:00Z",
        "assets": [
            {
                "name": "channel-source-stable.json",
                "url": "https://api.github.test/assets/staged-stable",
            }
        ],
    }
    requested: list[str] = []

    def read_url(url: str, **_kwargs: object) -> bytes:
        requested.append(url)
        if url.endswith("page=1"):
            return json.dumps(page_one).encode()
        if url.endswith("page=2"):
            return json.dumps([stable_source]).encode()
        raise AssertionError(f"unexpected release page: {url}")

    monkeypatch.setattr(SOURCE, "_read_url", read_url)

    releases = SOURCE._github_releases("google/capsem", "token")

    assert SOURCE.select_latest_source_asset(releases, "stable") == {
        "name": "channel-source-stable.json",
        "url": "https://api.github.test/assets/staged-stable",
    }
    assert requested == [
        "https://api.github.com/repos/google/capsem/releases?per_page=100&page=1",
        "https://api.github.com/repos/google/capsem/releases?per_page=100&page=2",
    ]


def test_channel_source_discovery_rejects_malformed_later_page(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def read_url(url: str, **_kwargs: object) -> bytes:
        if url.endswith("page=1"):
            return json.dumps([{}] * 100).encode()
        if url.endswith("page=2"):
            return b'{"message":"pagination drift"}'
        raise AssertionError(f"unexpected release page: {url}")

    monkeypatch.setattr(SOURCE, "_read_url", read_url)

    with pytest.raises(ValueError, match="page 2 is not an array"):
        SOURCE._github_releases("google/capsem", "token")


def test_channel_source_manifest_validation_is_channel_scoped() -> None:
    payload = json.dumps({"channel": "nightly", "runtime": {}, "packages": []}).encode()

    assert SOURCE.validate_source_manifest(payload, "nightly")["channel"] == "nightly"
    with pytest.raises(ValueError, match="expected 'stable'"):
        SOURCE.validate_source_manifest(payload, "stable")


def test_invalid_serialized_source_never_falls_back_to_channel_bootstrap(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    releases = [
        {
            "draft": False,
            "prerelease": False,
            "published_at": "2026-07-25T12:00:00Z",
            "assets": [
                {
                    "name": "channel-source-nightly.json",
                    "url": "https://api.github.test/assets/nightly",
                }
            ],
        }
    ]
    monkeypatch.setattr(SOURCE, "_github_releases", lambda *_args: releases)
    monkeypatch.setattr(
        SOURCE,
        "_read_url",
        lambda *_args, **_kwargs: b'{"channel":"wrong","packages":[]}',
    )

    with pytest.raises(ValueError, match="expected 'nightly'") as error:
        SOURCE.resolve_source_manifest(
            channel="nightly",
            repository="google/capsem",
            token="test",
            fallback_url="https://release.example/assets/nightly/manifest.json",
        )

    assert not isinstance(error.value, SOURCE.ChannelSourceUnavailable)


def test_missing_first_party_channel_bootstraps_through_capsem_admin(
    tmp_path: Path,
) -> None:
    donor = json.dumps(
        {
            "version": "1.0.143",
            "channel": "stable",
            "status": "current",
            "packages": [{"name": "Capsem.pkg"}],
            "runtime": {"revision": "stable-only"},
        }
    ).encode()
    output = tmp_path / "nightly.json"
    calls: list[list[str]] = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        del kwargs
        calls.append(command)
        donor_path = Path(command[command.index("--bootstrap-from-manifest") + 1])
        assert json.loads(donor_path.read_bytes())["channel"] == "stable"
        output_path = Path(command[command.index("--bootstrap-output") + 1])
        output_path.write_text(
            json.dumps(
                {
                    "version": "1.0.143",
                    "channel": "nightly",
                    "status": "current",
                    "packages": [{"name": "Capsem.pkg"}],
                }
            ),
            encoding="utf-8",
        )
        return subprocess.CompletedProcess(command, 0)

    payload = SOURCE.bootstrap_source_manifest(
        channel=SOURCE.FirstPartyChannel.NIGHTLY,
        source_commit=SOURCE.SourceCommit("a" * 40),
        input_payload=donor,
        output=output,
        runner=run,
    )

    assert "runtime" not in SOURCE.validate_source_manifest(payload, "nightly")
    assert len(calls) == 1
    command = calls[0]
    assert command[:6] == ["cargo", "run", "-p", "capsem-admin", "--", "release"]
    assert command[command.index("--channel") + 1] == "nightly"
    assert "--profile" not in command
    assert command[command.index("--source-commit") + 1] == "a" * 40


def test_exact_retired_public_graph_uses_the_same_channel_admin_author(
    tmp_path: Path,
) -> None:
    retired = json.dumps(
        {
            "version": "1.0.143",
            "channel": "stable",
            "status": "current",
            "packages": [{"name": "dead.deb"}],
            "runtime": {"revision": "retired"},
        }
    ).encode()
    digest = hashlib.sha256(retired).hexdigest()
    output = tmp_path / "stable.json"
    calls: list[list[str]] = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        del kwargs
        calls.append(command)
        input_path = Path(command[command.index("--bootstrap-retired-manifest") + 1])
        assert input_path.read_bytes() == retired
        output_path = Path(command[command.index("--bootstrap-output") + 1])
        output_path.write_text(
            json.dumps(
                {
                    "version": "1.0.143",
                    "channel": "stable",
                    "status": "current",
                    "packages": [],
                }
            ),
            encoding="utf-8",
        )
        return subprocess.CompletedProcess(command, 0)

    payload = SOURCE.bootstrap_source_manifest(
        channel=SOURCE.FirstPartyChannel.STABLE,
        source_commit=SOURCE.SourceCommit("b" * 40),
        input_payload=retired,
        output=output,
        retired_graph=SOURCE.retirement.RetiredPublicGraph(
            channel=SOURCE.FirstPartyChannel.STABLE,
            sha256=digest,
        ),
        runner=run,
    )

    assert SOURCE.validate_source_manifest(payload, "stable")["packages"] == []
    command = calls[0]
    assert command[command.index("--bootstrap-retired-sha256") + 1] == digest
    assert "--bootstrap-from-manifest" not in command


def test_retired_fallback_requires_config_catalog_and_payload_digest() -> None:
    payload = b'{"channel":"stable","packages":[]}'
    digest = hashlib.sha256(payload).hexdigest()
    catalog = json.dumps(
        {
            "channels": {
                "stable": {
                    "manifests": [
                        {
                            "status": "current",
                            "url": "/assets/stable/manifest.json",
                            "digest": {"sha256": digest},
                        }
                    ]
                }
            }
        }
    ).encode()

    retired = SOURCE.retirement.retired_public_fallback(
        channel=SOURCE.FirstPartyChannel.STABLE,
        fallback_url="https://release.example/assets/stable/manifest.json",
        payload=payload,
        retired_public_graphs={
            SOURCE.FirstPartyChannel.STABLE: SOURCE.retirement.RetiredPublicGraph(
                channel=SOURCE.FirstPartyChannel.STABLE,
                sha256=digest,
            )
        },
        read_url=lambda _url: catalog,
    )

    assert retired is not None
    assert retired.channel is SOURCE.FirstPartyChannel.STABLE
    assert retired.sha256 == digest


def test_missing_channel_bootstrap_requires_absence_from_public_catalog() -> None:
    catalog = json.dumps({"channels": {"stable": {}}}).encode()

    assert SOURCE.retirement.public_channel_is_absent(catalog, SOURCE.FirstPartyChannel.NIGHTLY)
    assert not SOURCE.retirement.public_channel_is_absent(catalog, SOURCE.FirstPartyChannel.STABLE)
    with pytest.raises(ValueError, match="channels object"):
        SOURCE.retirement.public_channel_is_absent(
            b'{"channels":[]}', SOURCE.FirstPartyChannel.NIGHTLY
        )


def test_bootstrap_baseline_allows_only_an_explicitly_absent_runtime(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    manifest = tmp_path / "nightly.json"
    manifest.write_text(
        json.dumps(
            {
                "version": "1.0.143",
                "channel": "nightly",
                "status": "current",
                "packages": [],
            }
        ),
        encoding="utf-8",
    )
    output = tmp_path / "runtime"
    url = manifest.as_uri()

    with pytest.raises(ValueError, match="contains no runtime"):
        FETCH.fetch_release_inputs(url, "runtime", output)

    primary_url = "https://release.example/assets/nightly/manifest.json"
    original_read = FETCH._read_url

    def read_url(requested: str) -> bytes:
        if requested == primary_url:
            raise OSError("nightly is not published")
        if requested == "https://release.example/channels.json":
            return b'{"channels":{"stable":{}}}'
        return original_read(requested)

    monkeypatch.setattr(FETCH, "_read_url", read_url)
    report = FETCH.fetch_release_inputs(
        primary_url,
        "runtime",
        output,
        allow_empty_runtime=True,
        bootstrap_manifest_url=url,
    )

    assert report["artifacts"] == []
    assert report["allow_empty_runtime"] is True
    assert report["manifest_url"] == url
    verification = VERIFY.verify_release_inputs(output)
    assert verification["verified"] == []


def test_bootstrap_release_inputs_reject_existing_public_channel(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bootstrap = tmp_path / "nightly.json"
    bootstrap.write_text(
        json.dumps(
            {
                "version": "1.0.143",
                "channel": "nightly",
                "status": "current",
                "packages": [],
            }
        ),
        encoding="utf-8",
    )
    primary_url = "https://release.example/assets/nightly/manifest.json"

    def read_url(requested: str) -> bytes:
        if requested == primary_url:
            raise OSError("published channel manifest is invalid")
        if requested == "https://release.example/channels.json":
            return b'{"channels":{"nightly":{}}}'
        return Path(requested.removeprefix("file://")).read_bytes()

    monkeypatch.setattr(FETCH, "_read_url", read_url)

    with pytest.raises(ValueError, match="exists but its manifest could not be resolved"):
        FETCH.fetch_release_inputs(
            primary_url,
            "runtime",
            tmp_path / "runtime",
            allow_empty_runtime=True,
            bootstrap_manifest_url=bootstrap.as_uri(),
        )


def _record(url: str, payload: bytes, **extra: object) -> dict[str, object]:
    return {
        "url": url,
        "bytes": len(payload),
        "digest": _digest(payload),
        **extra,
    }


def _write_manifest(tmp_path: Path) -> tuple[Path, dict[str, bytes]]:
    artifacts = {
        "capsem.deb": b"package",
        "package.spdx.json": b'{"spdxVersion":"SPDX-2.3"}',
        "vmlinuz": b"kernel",
        "initrd.img": b"initrd",
        "rootfs.erofs": b"rootfs",
        "obom.cdx.json": b'{"bomFormat":"CycloneDX"}',
        "software-inventory.json": b'{"architecture":"x86_64","packages":[]}',
    }
    for name, payload in artifacts.items():
        (tmp_path / name).write_bytes(payload)

    manifest = {
        "version": "1.0.0",
        "channel": "nightly",
        "status": "current",
        "packages": [
            _record(
                "capsem.deb",
                artifacts["capsem.deb"],
                name="capsem.deb",
                status="current",
                evidence=[
                    _record(
                        "package.spdx.json",
                        artifacts["package.spdx.json"],
                        kind="sbom",
                        status="current",
                    )
                ],
            )
        ],
        "runtime": {
            "revision": RUNTIME_REVISION,
            "status": "current",
            "architectures": [
                {
                    "architecture": "x86_64",
                    "package_inventory_revision": RUNTIME_REVISION,
                    "image_revision": RUNTIME_REVISION,
                    "images": [
                        _record(
                            name,
                            artifacts[name],
                            kind=kind,
                            name=name,
                            status="current",
                        )
                        for name, kind in (
                            ("vmlinuz", "kernel"),
                            ("initrd.img", "initrd"),
                            ("rootfs.erofs", "rootfs"),
                        )
                    ],
                    "evidence": [
                        _record(
                            "obom.cdx.json",
                            artifacts["obom.cdx.json"],
                            kind="obom",
                            status="current",
                        ),
                        _record(
                            "software-inventory.json",
                            artifacts["software-inventory.json"],
                            kind="software_inventory",
                            status="current",
                        ),
                    ],
                }
            ],
        },
    }
    path = tmp_path / "manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    return path, artifacts


def _add_arm64(manifest: Path, tmp_path: Path) -> None:
    """Give the runtime a second architecture with its own distinct bytes."""
    document = json.loads(manifest.read_text(encoding="utf-8"))
    x86 = document["runtime"]["architectures"][0]
    arm = json.loads(json.dumps(x86))
    arm["architecture"] = "arm64"
    for section in ("images", "evidence"):
        for index, record in enumerate(arm[section]):
            original = (tmp_path / record["url"]).read_bytes()
            name = f"arm64-{index}-{Path(record['url']).name}"
            payload = b"arm64-" + original
            (tmp_path / name).write_bytes(payload)
            arm[section][index] = _record(
                name,
                payload,
                **{
                    key: value
                    for key, value in record.items()
                    if key not in {"url", "bytes", "digest"}
                },
            )
    document["runtime"]["architectures"].append(arm)
    manifest.write_text(json.dumps(document), encoding="utf-8")


def test_runtime_fetch_can_limit_downloads_to_one_native_architecture(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    _add_arm64(manifest, tmp_path)

    output = tmp_path / "x86-runtime-inputs"
    report = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        output,
        architecture="x86_64",
    )

    assert report["architecture"] == "x86_64"
    assert report["artifacts"]
    assert all(row["path"].startswith("runtime/x86_64/") for row in report["artifacts"])
    assert not any("arm64-" in row["url"] for row in report["artifacts"])
    VERIFY.verify_release_inputs(output)


def test_runtime_fetch_reuses_manifest_digest_cache_and_prunes_old_blobs(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    cache = tmp_path / "artifact-cache"
    reads: list[str] = []
    original_read = FETCH._read_url

    def read_url(url: str) -> bytes:
        reads.append(url)
        return original_read(url)

    monkeypatch.setattr(FETCH, "_read_url", read_url)
    first = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        tmp_path / "first",
        architecture="x86_64",
        cache_dir=cache,
        prune_cache=True,
    )
    first_artifact_reads = [url for url in reads if url != manifest.as_uri()]
    reads.clear()
    stale = cache / "sha256" / "00" / ("0" * 64)
    stale.parent.mkdir(parents=True)
    stale.write_bytes(b"stale")

    second = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        tmp_path / "second",
        architecture="x86_64",
        cache_dir=cache,
        prune_cache=True,
    )

    assert first["cache"] == {"hits": 0, "misses": len(first["artifacts"])}
    assert second["cache"] == {"hits": len(second["artifacts"]), "misses": 0}
    assert first_artifact_reads
    assert reads == [manifest.as_uri()]
    assert not stale.exists()
    VERIFY.verify_release_inputs(tmp_path / "second")


def test_artifact_cache_identity_is_shared_across_channel_manifests(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    nightly_root = tmp_path / "nightly"
    stable_root = tmp_path / "stable"
    nightly_root.mkdir()
    stable_root.mkdir()
    nightly, _ = _write_manifest(nightly_root)
    stable, _ = _write_manifest(stable_root)
    stable_document = json.loads(stable.read_text(encoding="utf-8"))
    stable_document["channel"] = "stable"
    stable.write_text(json.dumps(stable_document), encoding="utf-8")
    cache = tmp_path / "artifact-cache"

    first = FETCH.fetch_release_inputs(
        nightly.as_uri(),
        "packages",
        tmp_path / "nightly-output",
        cache_dir=cache,
    )
    reads: list[str] = []
    original_read = FETCH._read_url

    def read_url(url: str) -> bytes:
        reads.append(url)
        return original_read(url)

    monkeypatch.setattr(FETCH, "_read_url", read_url)
    second = FETCH.fetch_release_inputs(
        stable.as_uri(),
        "packages",
        tmp_path / "stable-output",
        cache_dir=cache,
    )

    assert first["cache"] == {"hits": 0, "misses": len(first["artifacts"])}
    assert second["cache"] == {"hits": len(second["artifacts"]), "misses": 0}
    assert reads == [stable.as_uri()]
    assert (tmp_path / "stable-output" / "manifest.json").read_bytes() == stable.read_bytes()
    VERIFY.verify_release_inputs(tmp_path / "stable-output")


def test_corrupt_manifest_digest_cache_entry_is_replaced(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    cache = tmp_path / "artifact-cache"
    first = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        tmp_path / "first",
        architecture="x86_64",
        cache_dir=cache,
    )
    corrupt = first["artifacts"][0]
    cache_path = cache / "sha256" / corrupt["sha256"][:2] / corrupt["sha256"]
    cache_path.write_bytes(b"corrupt")
    reads: list[str] = []
    original_read = FETCH._read_url

    def read_url(url: str) -> bytes:
        reads.append(url)
        return original_read(url)

    monkeypatch.setattr(FETCH, "_read_url", read_url)
    second = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        tmp_path / "second",
        architecture="x86_64",
        cache_dir=cache,
    )

    assert second["cache"] == {
        "hits": len(second["artifacts"]) - 1,
        "misses": 1,
    }
    assert reads == [manifest.as_uri(), corrupt["url"]]
    assert cache_path.read_bytes() == (tmp_path / "second" / corrupt["path"]).read_bytes()
    VERIFY.verify_release_inputs(tmp_path / "second")


def test_fetches_only_current_packages_and_verifies_both_digests(
    tmp_path: Path,
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "packages"

    report = FETCH.fetch_release_inputs(manifest.as_uri(), "packages", output)

    assert report["kind"] == "packages"
    assert (output / "capsem.deb").read_bytes() == artifacts["capsem.deb"]
    assert (output / "manifest.json").read_bytes() == manifest.read_bytes()
    assert (output / "release-inputs.json").is_file()
    assert {row["path"] for row in report["artifacts"]} == {
        "capsem.deb",
        "package-evidence/package-0/package.spdx.json",
    }
    assert (output / "package-evidence/package-0/package.spdx.json").read_bytes() == artifacts[
        "package.spdx.json"
    ]


def test_fetches_every_runtime_input(tmp_path: Path) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "runtime"

    report = FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", output)

    paths = {row["path"] for row in report["artifacts"]}
    assert paths == {
        "runtime/x86_64/images/vmlinuz",
        "runtime/x86_64/images/initrd.img",
        "runtime/x86_64/images/rootfs.erofs",
        "runtime/x86_64/evidence/obom.cdx.json",
        "runtime/x86_64/evidence/software-inventory.json",
    }
    assert (output / "runtime/x86_64/images/rootfs.erofs").read_bytes() == artifacts["rootfs.erofs"]


def test_revoked_runtime_contributes_no_inputs(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["runtime"]["status"] = "revoked"
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(ValueError, match="resolved no runtime"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", tmp_path / "out")
    # An explicitly absent runtime is the only empty shape, never a revoked one.
    with pytest.raises(ValueError, match="resolved no runtime"):
        FETCH.fetch_release_inputs(
            manifest.as_uri(), "runtime", tmp_path / "out", allow_empty_runtime=True
        )


def test_runtime_boot_proof_uses_exact_manifest_selected_images_without_builders(
    tmp_path: Path,
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        output,
        architecture="x86_64",
    )
    calls: list[list[str]] = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        assert kwargs == {"check": True}
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    BOOT.prove_runtime_assets(
        output,
        architecture="x86_64",
        timeout=41,
        runner=run,
    )

    assert len(calls) == 1
    command = calls[0]
    assert command[:6] == ["cargo", "run", "--locked", "-p", "capsem-core", "--example"]
    assert command[command.index("--timeout") + 1] == "41"
    for kind, filename in (
        ("kernel", "vmlinuz"),
        ("initrd", "initrd.img"),
        ("rootfs", "rootfs.erofs"),
    ):
        path = Path(command[command.index(f"--{kind}") + 1])
        digest = command[command.index(f"--{kind}-blake3") + 1]
        assert path.read_bytes() == artifacts[filename]
        assert digest == blake3.blake3(artifacts[filename]).hexdigest()
    joined = " ".join(command)
    for forbidden in (
        "capsem-admin",
        "_build-assets",
        "_build-kernel",
        "_build-rootfs",
    ):
        assert forbidden not in joined


def test_runtime_boot_proof_rejects_inputs_for_another_architecture(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        output,
        architecture="x86_64",
    )

    with pytest.raises(ValueError, match="select x86_64, not host arm64"):
        BOOT.resolve_runtime_boot_inputs(output, "arm64")


def test_runtime_boot_proof_rejects_duplicate_boot_image_kind(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    images = document["runtime"]["architectures"][0]["images"]
    images.append(dict(images[0]))
    manifest.write_text(json.dumps(document), encoding="utf-8")
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        output,
        architecture="x86_64",
    )

    with pytest.raises(ValueError, match="repeats kernel image"):
        BOOT.resolve_runtime_boot_inputs(output, "x86_64")


def test_runtime_boot_proof_rejects_transport_missing_manifest_image(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "runtime",
        output,
        architecture="x86_64",
    )
    report = json.loads((output / "release-inputs.json").read_text(encoding="utf-8"))
    report["artifacts"] = [
        row for row in report["artifacts"] if not row["path"].endswith("/images/rootfs.erofs")
    ]
    (output / "release-inputs.json").write_text(json.dumps(report), encoding="utf-8")

    with pytest.raises(ValueError, match="does not match the resolved manifest artifact set"):
        BOOT.resolve_runtime_boot_inputs(output, "x86_64")


def _stage_local_runtime_publication(
    manifest_path: Path,
    publication_dir: Path,
) -> str:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    publication_base = f"https://github.test/releases/download/runtime-nightly-{RUNTIME_REVISION}"
    publication_dir.mkdir()
    for architecture in manifest["runtime"]["architectures"]:
        arch = architecture["architecture"]
        for section in ("images", "evidence"):
            for row in architecture[section]:
                source = manifest_path.parent / Path(row["url"]).name
                name = f"{arch}-{source.name}"
                row["url"] = f"{publication_base}/{name}"
                (publication_dir / name).write_bytes(source.read_bytes())
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    (publication_dir / "channel-source-nightly.json").write_bytes(manifest_path.read_bytes())
    return publication_base


def test_candidate_runtime_inputs_mix_staged_publication_with_manifest_urls(
    tmp_path: Path,
) -> None:
    manifest_path, artifacts = _write_manifest(tmp_path)
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_runtime_publication(
        manifest_path,
        publication_dir,
    )
    output = tmp_path / "candidate-runtime"

    report = FETCH.fetch_release_inputs(
        manifest_path.as_uri(),
        "runtime",
        output,
        local_publication_base=publication_base,
        local_publication_dir=publication_dir,
    )

    assert (output / "manifest.json").read_bytes() == manifest_path.read_bytes()
    assert {row["url"] for row in report["artifacts"]} == {
        f"{publication_base}/x86_64-vmlinuz",
        f"{publication_base}/x86_64-initrd.img",
        f"{publication_base}/x86_64-rootfs.erofs",
        f"{publication_base}/x86_64-obom.cdx.json",
        f"{publication_base}/x86_64-software-inventory.json",
    }
    assert (output / "runtime/x86_64/images/x86_64-rootfs.erofs").read_bytes() == artifacts[
        "rootfs.erofs"
    ]
    VERIFY.verify_release_inputs(output)


def test_candidate_runtime_architecture_filter_accepts_manifest_owned_siblings_only(
    tmp_path: Path,
) -> None:
    manifest_path, _ = _write_manifest(tmp_path)
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    x86 = document["runtime"]["architectures"][0]
    arm = json.loads(json.dumps(x86))
    arm["architecture"] = "arm64"
    document["runtime"]["architectures"].append(arm)
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_runtime_publication(
        manifest_path,
        publication_dir,
    )

    output = tmp_path / "candidate-arm64"
    report = FETCH.fetch_release_inputs(
        manifest_path.as_uri(),
        "runtime",
        output,
        architecture="arm64",
        local_publication_base=publication_base,
        local_publication_dir=publication_dir,
    )

    assert report["architecture"] == "arm64"
    assert report["artifacts"]
    assert all(row["path"].startswith("runtime/arm64/") for row in report["artifacts"])
    VERIFY.verify_release_inputs(output)

    (publication_dir / "not-selected-by-manifest").write_bytes(b"extra")
    with pytest.raises(ValueError, match="file set mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "runtime",
            tmp_path / "candidate-with-extra",
            architecture="arm64",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )


def test_candidate_runtime_override_is_all_or_nothing_and_exact(
    tmp_path: Path,
) -> None:
    manifest_path, _ = _write_manifest(tmp_path)
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_runtime_publication(
        manifest_path,
        publication_dir,
    )

    with pytest.raises(ValueError, match="supplied together"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "runtime",
            tmp_path / "partial",
            local_publication_base=publication_base,
        )

    (publication_dir / "unexpected").write_bytes(b"extra")
    with pytest.raises(ValueError, match="file set mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "runtime",
            tmp_path / "extra",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )
    (publication_dir / "unexpected").unlink()

    (publication_dir / "x86_64-rootfs.erofs").write_bytes(b"tampered")
    with pytest.raises(ValueError, match="byte size mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "runtime",
            tmp_path / "tampered",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )


@pytest.mark.parametrize("field", ["sha256", "blake3"])
def test_rejects_tampered_runtime_digest(tmp_path: Path, field: str) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["runtime"]["architectures"][0]["images"][0]["digest"][field] = "0" * 64
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(
        ValueError, match=field.replace("sha256", "SHA-256").replace("blake3", "BLAKE3")
    ):
        FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", tmp_path / "out")


def test_rejects_manifest_without_owned_inputs(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text('{"packages":[]}', encoding="utf-8")

    with pytest.raises(ValueError, match="no packages"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "packages", tmp_path / "out")
    with pytest.raises(ValueError, match="contains no runtime"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", tmp_path / "out")


def test_verifier_rejects_a_tampered_resolved_input(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "packages"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", output)
    (output / "capsem.deb").write_bytes(b"tampered")

    with pytest.raises(ValueError, match="byte size mismatch"):
        VERIFY.verify_release_inputs(output)


def test_verifier_rejects_an_artifact_omitted_from_its_manifest_derivation(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", output)
    report_path = output / "release-inputs.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["artifacts"].pop()
    report_path.write_text(json.dumps(report), encoding="utf-8")

    with pytest.raises(ValueError, match="does not match the resolved manifest"):
        VERIFY.verify_release_inputs(output)


def test_verifier_rejects_a_report_identity_substituted_after_resolution(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "runtime"
    FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", output)
    report_path = output / "release-inputs.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["artifacts"][0]["url"] = "https://attacker.invalid/substitute"
    report_path.write_text(json.dumps(report), encoding="utf-8")

    with pytest.raises(ValueError, match="does not match the resolved manifest"):
        VERIFY.verify_release_inputs(output)


def test_fetch_rejects_runtime_architecture_path_traversal(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["runtime"]["architectures"][0]["architecture"] = ".."
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(ValueError, match="unsafe runtime architecture identity"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", tmp_path / "out")

    assert not (tmp_path / "out" / "runtime").exists()


def _stage_runtime_inputs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, manifest: Path) -> Path:
    inputs = tmp_path / "runtime-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    return inputs


def test_stages_every_verified_runtime_image_and_evidence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    _add_arm64(manifest, tmp_path)
    inputs = _stage_runtime_inputs(tmp_path, monkeypatch, manifest)
    assets = tmp_path / "assets"
    (assets / "stale").mkdir(parents=True)

    staged_manifest = STAGE.stage_runtime(inputs, assets)

    assert staged_manifest == assets / "manifest.json"
    document = json.loads(staged_manifest.read_text(encoding="utf-8"))
    image_url = document["runtime"]["architectures"][0]["images"][0]["url"]
    assert image_url.startswith("file://")
    assert not (assets / "stale").exists()
    assert not (assets / "arm64").exists()
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        payload = artifacts[logical_name]
        assert (assets / "x86_64" / logical_name).read_bytes() == payload
        digest = blake3.blake3(payload).hexdigest()
        hashed = assets / "x86_64" / STAGE.hash_filename(logical_name, digest)
        assert hashed.read_bytes() == payload


@pytest.mark.parametrize("source_commit", ["A" * 40, "a" * 39, "main", 7])
def test_runtime_staging_rejects_a_malformed_source_commit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    source_commit: object,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["runtime"]["source_commit"] = source_commit
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = _stage_runtime_inputs(tmp_path, monkeypatch, manifest)

    with pytest.raises(ValueError, match="malformed source_commit"):
        STAGE.stage_runtime(inputs, tmp_path / "assets")


def test_runtime_staging_refuses_missing_configured_evidence_before_package_work(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    evidence = document["runtime"]["architectures"][0]["evidence"]
    document["runtime"]["architectures"][0]["evidence"] = [
        record for record in evidence if record["kind"] != "obom"
    ]
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = _stage_runtime_inputs(tmp_path, monkeypatch, manifest)

    with pytest.raises(ValueError, match=r"runtime/x86_64.*obom\.cdx\.json"):
        STAGE.stage_runtime(inputs, tmp_path / "assets")


def test_runtime_staging_refuses_a_runtime_without_the_host_architecture(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = _stage_runtime_inputs(tmp_path, monkeypatch, manifest)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "arm64")
    assets = tmp_path / "assets"
    assets.mkdir()
    sentinel = assets / "keep-on-validation-failure"
    sentinel.write_text("preserved\n", encoding="utf-8")

    with pytest.raises(ValueError, match="exactly one arm64 architecture"):
        STAGE.stage_runtime(inputs, assets)

    assert sentinel.read_text(encoding="utf-8") == "preserved\n"


def test_runtime_staging_requires_runtime_release_inputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match="requires runtime release inputs"):
        STAGE.stage_runtime(inputs, tmp_path / "assets")


def test_selected_install_transport_keeps_the_verified_source_graph(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The immutable input report, not generated file URLs, binds its bytes.

    The hosted install lane fetches and verifies the public release graph into
    ``inputs/``. Runtime staging rewrites a separate projection to its local
    immutable payloads. Requiring the original graph itself to contain those
    generated URLs rejected the real stable channel only after the package and
    sealed install image had spent nearly an hour building.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.content import RuntimeContent, SelectedInstallContent

    manifest, _ = _write_manifest(tmp_path)
    root = tmp_path / "selected-content"
    inputs = root / "inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "runtime", inputs)
    original_manifest = (inputs / "manifest.json").read_bytes()
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    staged_manifest = STAGE.stage_runtime(inputs, root / "assets")
    config = gate_config.load(ROOT)
    content = RuntimeContent.isolated(config, root)
    config_manifest = content.config_manifest(config)
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(staged_manifest.read_bytes())
    # The service catalog is materialized from the checkout, never staged from
    # release inputs; stand one in so only the runtime projection is under test.
    catalog_entry = content.profiles(config) / "code" / "profile.toml"
    catalog_entry.parent.mkdir(parents=True)
    catalog_entry.write_text('id = "code"\n', encoding="utf-8")

    selected = SelectedInstallContent(content)
    selected.require_complete(config, arches=(config.architectures["x86_64"],))

    assert (inputs / "manifest.json").read_bytes() == original_manifest
    assert b"file://" in staged_manifest.read_bytes()


def test_staging_reverifies_inputs_instead_of_trusting_the_fetch_report(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = _stage_runtime_inputs(tmp_path, monkeypatch, manifest)
    report = json.loads((inputs / "release-inputs.json").read_text(encoding="utf-8"))
    (inputs / report["artifacts"][0]["path"]).write_bytes(b"tampered")

    with pytest.raises(ValueError, match="byte size mismatch"):
        STAGE.stage_runtime(inputs, tmp_path / "assets")


def _package_with_binary_inventory(
    manifest: Path,
    binaries: dict[str, bytes],
) -> None:
    document = json.loads(manifest.read_text(encoding="utf-8"))
    package = document["packages"][0]
    package.update({"platform": "linux", "architecture": "amd64"})
    package["binaries"] = [
        _record(
            f"/usr/bin/{name}",
            payload,
            name=name,
            installed_path=f"/usr/bin/{name}",
            platform="linux",
            architecture="amd64",
            status="current",
        )
        for name, payload in binaries.items()
    ]
    manifest.write_text(json.dumps(document), encoding="utf-8")


def test_pulled_binary_package_staging_uses_and_verifies_complete_inventory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    binary_payloads = {
        "capsem": b"resolved-capsem",
        "capsem-service": b"resolved-service",
    }
    _package_with_binary_inventory(manifest, binary_payloads)
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    monkeypatch.setattr(
        STAGE,
        "deb_payload_files",
        lambda _path, **_: {
            f"/usr/bin/{name}": payload for name, payload in binary_payloads.items()
        },
    )
    binary_dir = tmp_path / "cache/target/cargo/debug"
    binary_dir.mkdir(parents=True)
    stale = binary_dir / "capsem-source-built"
    stale.write_bytes(b"must-not-survive")

    staged = STAGE.stage_package_binaries(inputs, binary_dir)

    assert STAGE.select_host_package_path(inputs) == inputs / "capsem.deb"
    assert {path.name for path in staged} == set(binary_payloads)
    assert not stale.exists()
    for name, payload in binary_payloads.items():
        assert (binary_dir / name).read_bytes() == payload


def test_runtime_lane_marks_old_binary_cohort_incomplete_without_building(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    binary_payloads = {
        name: f"resolved-{name}".encode()
        for name in STAGE.REQUIRED_LINUX_RELEASE_BINARIES
        if name not in {"capsem-mock-server", "capsem-bench-rs"}
    }
    _package_with_binary_inventory(manifest, binary_payloads)
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    readiness = STAGE.functional_binary_cohort_readiness(inputs)

    assert readiness == {
        "ready": False,
        "missing": ["capsem-bench-rs", "capsem-mock-server"],
        "unexpected": [],
    }


def test_runtime_lane_accepts_only_the_complete_manifest_binary_cohort(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    binary_payloads = {
        name: f"resolved-{name}".encode() for name in STAGE.REQUIRED_LINUX_RELEASE_BINARIES
    }
    _package_with_binary_inventory(manifest, binary_payloads)
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    readiness = STAGE.functional_binary_cohort_readiness(inputs)

    assert readiness == {"ready": True, "missing": [], "unexpected": []}


def test_package_staging_accepts_manifest_selected_x86_64_debian_package(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    _package_with_binary_inventory(manifest, {"capsem": b"resolved-capsem"})
    document = json.loads(manifest.read_text(encoding="utf-8"))
    package = document["packages"][0]
    package["architecture"] = "x86_64"
    for binary in package["binaries"]:
        binary["architecture"] = "x86_64"
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    selected = STAGE.select_host_package_path(inputs)

    assert selected == inputs / "capsem.deb"
    assert json.loads((inputs / "manifest.json").read_text(encoding="utf-8")) == document


def test_package_staging_rejects_inventory_missing_from_the_package(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    _package_with_binary_inventory(
        manifest,
        {"capsem": b"resolved-capsem", "capsem-service": b"resolved-service"},
    )
    inputs = tmp_path / "package-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    monkeypatch.setattr(
        STAGE,
        "deb_payload_files",
        lambda _path, **_: {
            "/usr/bin/capsem": b"resolved-capsem",
        },
    )

    with pytest.raises(ValueError, match="capsem-service"):
        STAGE.stage_package_binaries(inputs, tmp_path / "cache/target/cargo/debug")


def test_candidate_package_staging_cannot_fall_back_to_source_binaries(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    package = tmp_path / "candidate.deb"
    package.write_bytes(b"candidate-package")
    payloads = {"capsem": b"candidate-capsem", "capsem-service": b"candidate-service"}

    def read_payload(path: Path, **_) -> dict[str, bytes]:
        assert path == package
        return {f"/usr/bin/{name}": payload for name, payload in payloads.items()}

    monkeypatch.setattr(STAGE, "deb_payload_files", read_payload)
    binary_dir = tmp_path / "cache/target/cargo/debug"
    binary_dir.mkdir(parents=True)
    (binary_dir / "capsem").write_bytes(b"source-capsem")
    (binary_dir / "capsem-mcp").write_bytes(b"source-only-fallback")

    staged = STAGE.stage_candidate_package(package, binary_dir)

    assert {path.name for path in staged} == set(payloads)
    assert not (binary_dir / "capsem-mcp").exists()
    assert (binary_dir / "capsem").read_bytes() == payloads["capsem"]


def test_empty_package_cohort_is_permitted_only_when_stated(tmp_path: Path) -> None:
    """A cold-started channel has no package cohort, and says so explicitly.

    The before-state of an absent channel is empty of both families. Silence is
    still an error: a live channel whose packages stopped resolving is exactly
    the breakage users would hit, so it must never be mistaken for a channel
    that has not shipped yet.
    """
    manifest = tmp_path / "nightly.json"
    manifest.write_text(
        json.dumps(
            {
                "version": "1.0.143",
                "channel": "nightly",
                "status": "current",
                "packages": [],
            }
        ),
        encoding="utf-8",
    )
    url = manifest.as_uri()

    with pytest.raises(ValueError, match="contains no packages"):
        FETCH.fetch_release_inputs(url, "packages", tmp_path / "strict")

    report = FETCH.fetch_release_inputs(
        url,
        "packages",
        tmp_path / "cold",
        allow_empty_packages=True,
    )

    assert report["artifacts"] == []
    assert report["allow_empty_packages"] is True
    assert VERIFY.verify_release_inputs(tmp_path / "cold")["verified"] == []


def test_empty_package_tolerance_is_rejected_for_the_runtime_family(
    tmp_path: Path,
) -> None:
    manifest = tmp_path / "nightly.json"
    manifest.write_text(
        json.dumps(
            {
                "version": "1.0.143",
                "channel": "nightly",
                "status": "current",
                "packages": [],
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match=r"package-only|only for packages"):
        FETCH.fetch_release_inputs(
            manifest.as_uri(),
            "runtime",
            tmp_path / "runtime",
            allow_empty_packages=True,
        )

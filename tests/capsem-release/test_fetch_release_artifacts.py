from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import blake3
import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "fetch-release-artifacts.py"
SPEC = importlib.util.spec_from_file_location("fetch_release_artifacts", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
FETCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FETCH)
VERIFY_SPEC = importlib.util.spec_from_file_location(
    "verify_release_inputs", ROOT / "scripts" / "verify-release-inputs.py"
)
assert VERIFY_SPEC is not None and VERIFY_SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(VERIFY_SPEC)
VERIFY_SPEC.loader.exec_module(VERIFY)
STAGE_SPEC = importlib.util.spec_from_file_location(
    "stage_release_test_inputs", ROOT / "scripts" / "stage-release-test-inputs.py"
)
assert STAGE_SPEC is not None and STAGE_SPEC.loader is not None
STAGE = importlib.util.module_from_spec(STAGE_SPEC)
STAGE_SPEC.loader.exec_module(STAGE)
PROFILE_STAGE_SPEC = importlib.util.spec_from_file_location(
    "stage_profile_assets", ROOT / "scripts" / "stage_profile_assets.py"
)
assert PROFILE_STAGE_SPEC is not None and PROFILE_STAGE_SPEC.loader is not None
PROFILE_STAGE = importlib.util.module_from_spec(PROFILE_STAGE_SPEC)
PROFILE_STAGE_SPEC.loader.exec_module(PROFILE_STAGE)
BOOT_SPEC = importlib.util.spec_from_file_location(
    "prove_release_profile_assets",
    ROOT / "scripts" / "prove-release-profile-assets.py",
)
assert BOOT_SPEC is not None and BOOT_SPEC.loader is not None
BOOT = importlib.util.module_from_spec(BOOT_SPEC)
BOOT_SPEC.loader.exec_module(BOOT)
SOURCE_SPEC = importlib.util.spec_from_file_location(
    "fetch_channel_source_manifest",
    ROOT / "scripts" / "fetch-channel-source-manifest.py",
)
SOURCE_SCRIPT = ROOT / "scripts" / "fetch-channel-source-manifest.py"
assert SOURCE_SPEC is not None and SOURCE_SPEC.loader is not None
SOURCE = importlib.util.module_from_spec(SOURCE_SPEC)
SOURCE_SPEC.loader.exec_module(SOURCE)
PROFILE_AXIS_SPEC = importlib.util.spec_from_file_location(
    "release_test_profiles",
    ROOT / "scripts" / "release-test-profiles.py",
)
assert PROFILE_AXIS_SPEC is not None and PROFILE_AXIS_SPEC.loader is not None
PROFILE_AXIS = importlib.util.module_from_spec(PROFILE_AXIS_SPEC)
PROFILE_AXIS_SPEC.loader.exec_module(PROFILE_AXIS)


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
                    "url": "https://api.github.test/assets/resumed-profile",
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
    payload = json.dumps({"channel": "nightly", "profiles": {"code": {}}, "packages": []}).encode()

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
        lambda *_args, **_kwargs: b'{"channel":"wrong","profiles":{},"packages":[]}',
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
            "profiles": {"code": {"revision": "stable-only"}},
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
                    "profiles": {},
                }
            ),
            encoding="utf-8",
        )
        return subprocess.CompletedProcess(command, 0)

    payload = SOURCE.bootstrap_source_manifest(
        channel=SOURCE.FirstPartyChannel.NIGHTLY,
        profile="code",
        source_commit=SOURCE.SourceCommit("a" * 40),
        input_payload=donor,
        output=output,
        runner=run,
    )

    assert SOURCE.validate_source_manifest(payload, "nightly")["profiles"] == {}
    assert len(calls) == 1
    command = calls[0]
    assert command[:6] == ["cargo", "run", "-p", "capsem-admin", "--", "release"]
    assert command[command.index("--channel") + 1] == "nightly"
    assert command[command.index("--profile") + 1] == "code"
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
            "profiles": {"code": {"revision": "legacy"}},
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
                    "profiles": {},
                }
            ),
            encoding="utf-8",
        )
        return subprocess.CompletedProcess(command, 0)

    payload = SOURCE.bootstrap_source_manifest(
        channel=SOURCE.FirstPartyChannel.STABLE,
        profile="code",
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
    payload = b'{"channel":"stable","packages":[],"profiles":{}}'
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


def test_bootstrap_baseline_allows_only_explicit_empty_profile_membership(
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
                "profiles": {},
            }
        ),
        encoding="utf-8",
    )
    output = tmp_path / "profiles"
    url = manifest.as_uri()

    with pytest.raises(ValueError, match="contains no profiles"):
        FETCH.fetch_release_inputs(url, "profiles", output)

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
        "profiles",
        output,
        allow_empty_profiles=True,
        bootstrap_manifest_url=url,
    )

    assert report["artifacts"] == []
    assert report["allow_empty_profiles"] is True
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
                "profiles": {},
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
            "profiles",
            tmp_path / "profiles",
            allow_empty_profiles=True,
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
        "profile.toml": b"[profile]\nid='code'\n",
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
        "profiles": {
            "code": {
                "version": "code-1",
                "id": "code",
                "name": "Code",
                "revision": "code-1",
                "status": "current",
                "architectures": [
                    {
                        "architecture": "x86_64",
                        "config": [
                            _record(
                                "profile.toml",
                                artifacts["profile.toml"],
                                kind="profile",
                                path="profiles/code/profile.toml",
                                status="current",
                            )
                        ],
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
            }
        },
    }
    path = tmp_path / "manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    return path, artifacts


def test_profile_fetch_can_limit_downloads_to_one_native_architecture(
    tmp_path: Path,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    x86 = document["profiles"]["code"]["architectures"][0]
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
    document["profiles"]["code"]["architectures"].append(arm)
    manifest.write_text(json.dumps(document), encoding="utf-8")

    output = tmp_path / "x86-profile-inputs"
    report = FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "profiles",
        output,
        architecture="x86_64",
    )

    assert report["architecture"] == "x86_64"
    assert report["artifacts"]
    assert all("/x86_64/" in row["path"] for row in report["artifacts"])
    assert not any("arm64-" in row["url"] for row in report["artifacts"])
    VERIFY.verify_release_inputs(output)


def test_profile_fetch_reuses_manifest_digest_cache_and_prunes_old_blobs(
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
        "profiles",
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
        "profiles",
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
        "profiles",
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
        "profiles",
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


def test_fetches_every_profile_owned_input(tmp_path: Path) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "profiles"

    report = FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", output)

    paths = {row["path"] for row in report["artifacts"]}
    assert paths == {
        "profiles/code/x86_64/config/profile.toml",
        "profiles/code/x86_64/images/vmlinuz",
        "profiles/code/x86_64/images/initrd.img",
        "profiles/code/x86_64/images/rootfs.erofs",
        "profiles/code/x86_64/evidence/obom.cdx.json",
        "profiles/code/x86_64/evidence/software-inventory.json",
    }
    assert (output / "profiles/code/x86_64/images/rootfs.erofs").read_bytes() == artifacts[
        "rootfs.erofs"
    ]


def test_profile_boot_proof_uses_exact_manifest_selected_images_without_builders(
    tmp_path: Path,
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "profiles",
        output,
        architecture="x86_64",
    )
    calls: list[list[str]] = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        assert kwargs == {"check": True}
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    BOOT.prove_profile_assets(
        output,
        "code",
        architecture="x86_64",
        timeout=41,
        runner=run,
    )

    assert len(calls) == 1
    command = calls[0]
    assert command[:8] == [
        "cargo",
        "run",
        "--locked",
        "-p",
        "capsem-core",
        "--example",
        "release_profile_boot",
        "--",
    ]
    assert command[command.index("--profile") + 1] == "code"
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
    for forbidden in ("capsem-admin", "_build-assets", "_build-kernel", "_build-rootfs"):
        assert forbidden not in joined


def test_profile_boot_proof_rejects_profile_absent_from_manifest(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "profiles",
        output,
        architecture="x86_64",
    )

    with pytest.raises(ValueError, match="does not select profile missing"):
        BOOT.resolve_profile_boot_inputs(output, "missing", "x86_64")


def test_profile_boot_proof_rejects_duplicate_boot_image_kind(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    images = document["profiles"]["code"]["architectures"][0]["images"]
    images.append(dict(images[0]))
    manifest.write_text(json.dumps(document), encoding="utf-8")
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "profiles",
        output,
        architecture="x86_64",
    )

    with pytest.raises(ValueError, match="repeats kernel image"):
        BOOT.resolve_profile_boot_inputs(output, "code", "x86_64")


def test_profile_boot_proof_rejects_transport_missing_manifest_image(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(
        manifest.as_uri(),
        "profiles",
        output,
        architecture="x86_64",
    )
    report = json.loads((output / "release-inputs.json").read_text(encoding="utf-8"))
    report["artifacts"] = [
        row for row in report["artifacts"] if not row["path"].endswith("/images/rootfs.erofs")
    ]
    (output / "release-inputs.json").write_text(json.dumps(report), encoding="utf-8")

    with pytest.raises(ValueError, match="does not match the resolved manifest artifact set"):
        BOOT.resolve_profile_boot_inputs(output, "code", "x86_64")


def _stage_local_profile_publication(
    manifest_path: Path,
    publication_dir: Path,
) -> str:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    publication_base = "https://github.test/releases/download/profile-nightly-code-code-1"
    publication_dir.mkdir()
    profile = manifest["profiles"]["code"]
    for architecture in profile["architectures"]:
        arch = architecture["architecture"]
        for section in ("config", "images", "evidence"):
            for row in architecture[section]:
                source = manifest_path.parent / Path(row["url"]).name
                name = f"{arch}-{source.name}"
                row["url"] = f"{publication_base}/{name}"
                (publication_dir / name).write_bytes(source.read_bytes())
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    (publication_dir / "channel-source-nightly.json").write_bytes(manifest_path.read_bytes())
    return publication_base


def test_candidate_profile_inputs_mix_staged_publication_with_manifest_urls(
    tmp_path: Path,
) -> None:
    manifest_path, artifacts = _write_manifest(tmp_path)
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_profile_publication(
        manifest_path,
        publication_dir,
    )
    output = tmp_path / "candidate-profiles"

    report = FETCH.fetch_release_inputs(
        manifest_path.as_uri(),
        "profiles",
        output,
        local_publication_base=publication_base,
        local_publication_dir=publication_dir,
    )

    assert (output / "manifest.json").read_bytes() == manifest_path.read_bytes()
    assert {row["url"] for row in report["artifacts"]} == {
        f"{publication_base}/x86_64-profile.toml",
        f"{publication_base}/x86_64-vmlinuz",
        f"{publication_base}/x86_64-initrd.img",
        f"{publication_base}/x86_64-rootfs.erofs",
        f"{publication_base}/x86_64-obom.cdx.json",
        f"{publication_base}/x86_64-software-inventory.json",
    }
    assert (output / "profiles/code/x86_64/images/x86_64-rootfs.erofs").read_bytes() == artifacts[
        "rootfs.erofs"
    ]
    VERIFY.verify_release_inputs(output)


def test_candidate_profile_architecture_filter_accepts_manifest_owned_siblings_only(
    tmp_path: Path,
) -> None:
    manifest_path, _ = _write_manifest(tmp_path)
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    x86 = document["profiles"]["code"]["architectures"][0]
    arm = json.loads(json.dumps(x86))
    arm["architecture"] = "arm64"
    document["profiles"]["code"]["architectures"].append(arm)
    manifest_path.write_text(json.dumps(document), encoding="utf-8")
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_profile_publication(
        manifest_path,
        publication_dir,
    )

    output = tmp_path / "candidate-arm64"
    report = FETCH.fetch_release_inputs(
        manifest_path.as_uri(),
        "profiles",
        output,
        architecture="arm64",
        local_publication_base=publication_base,
        local_publication_dir=publication_dir,
    )

    assert report["architecture"] == "arm64"
    assert report["artifacts"]
    assert all("/arm64/" in row["path"] for row in report["artifacts"])
    assert not any("/x86_64/" in row["path"] for row in report["artifacts"])
    VERIFY.verify_release_inputs(output)

    (publication_dir / "not-selected-by-manifest").write_bytes(b"extra")
    with pytest.raises(ValueError, match="file set mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "profiles",
            tmp_path / "candidate-with-extra",
            architecture="arm64",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )


def test_candidate_profile_override_is_all_or_nothing_and_exact(
    tmp_path: Path,
) -> None:
    manifest_path, _ = _write_manifest(tmp_path)
    publication_dir = tmp_path / "publication"
    publication_base = _stage_local_profile_publication(
        manifest_path,
        publication_dir,
    )

    with pytest.raises(ValueError, match="supplied together"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "profiles",
            tmp_path / "partial",
            local_publication_base=publication_base,
        )

    (publication_dir / "unexpected").write_bytes(b"extra")
    with pytest.raises(ValueError, match="file set mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "profiles",
            tmp_path / "extra",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )
    (publication_dir / "unexpected").unlink()

    (publication_dir / "x86_64-rootfs.erofs").write_bytes(b"tampered")
    with pytest.raises(ValueError, match="byte size mismatch"):
        FETCH.fetch_release_inputs(
            manifest_path.as_uri(),
            "profiles",
            tmp_path / "tampered",
            local_publication_base=publication_base,
            local_publication_dir=publication_dir,
        )


@pytest.mark.parametrize("field", ["sha256", "blake3"])
def test_rejects_tampered_profile_digest(tmp_path: Path, field: str) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["profiles"]["code"]["architectures"][0]["images"][0]["digest"][field] = "0" * 64
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(
        ValueError, match=field.replace("sha256", "SHA-256").replace("blake3", "BLAKE3")
    ):
        FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", tmp_path / "out")


def test_rejects_manifest_without_owned_inputs(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text('{"packages":[],"profiles":{}}', encoding="utf-8")

    with pytest.raises(ValueError, match="no packages"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "packages", tmp_path / "out")


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
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", output)
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
    output = tmp_path / "profiles"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", output)
    report_path = output / "release-inputs.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["artifacts"][0]["url"] = "https://attacker.invalid/substitute"
    report_path.write_text(json.dumps(report), encoding="utf-8")

    with pytest.raises(ValueError, match="does not match the resolved manifest"):
        VERIFY.verify_release_inputs(output)


def test_fetch_rejects_profile_identity_path_traversal(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["profiles"]["../../outside"] = document["profiles"].pop("code")
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(ValueError, match="unsafe profile identity"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", tmp_path / "out")

    assert not (tmp_path / "outside").exists()


def _add_distinct_profile(
    manifest: Path,
    tmp_path: Path,
) -> dict[str, bytes]:
    document = json.loads(manifest.read_text(encoding="utf-8"))
    artifacts = {
        "co-work.toml": b"[profile]\nid='co-work'\n",
        "co-work-vmlinuz": b"co-work-kernel",
        "co-work-initrd.img": b"co-work-initrd",
        "co-work-rootfs.erofs": b"co-work-rootfs",
        "co-work-obom.cdx.json": b'{"bomFormat":"CycloneDX","profile":"co-work"}',
        "co-work-software-inventory.json": b'{"architecture":"x86_64","profile":"co-work"}',
    }
    for name, payload in artifacts.items():
        (tmp_path / name).write_bytes(payload)
    document["profiles"]["co-work"] = {
        "version": "co-work-1",
        "id": "co-work",
        "name": "Co-work",
        "revision": "co-work-1",
        "status": "current",
        "architectures": [
            {
                "architecture": "x86_64",
                "config": [
                    _record(
                        "co-work.toml",
                        artifacts["co-work.toml"],
                        kind="profile",
                        path="profiles/co-work/profile.toml",
                        status="current",
                    )
                ],
                "images": [
                    _record(
                        name,
                        artifacts[name],
                        kind=kind,
                        name=logical_name,
                        status="current",
                    )
                    for name, kind, logical_name in (
                        ("co-work-vmlinuz", "kernel", "vmlinuz"),
                        ("co-work-initrd.img", "initrd", "initrd.img"),
                        ("co-work-rootfs.erofs", "rootfs", "rootfs.erofs"),
                    )
                ],
                "evidence": [
                    _record(
                        "co-work-obom.cdx.json",
                        artifacts["co-work-obom.cdx.json"],
                        kind="obom",
                        status="current",
                    ),
                    _record(
                        "co-work-software-inventory.json",
                        artifacts["co-work-software-inventory.json"],
                        kind="software_inventory",
                        status="current",
                    ),
                ],
            }
        ],
    }
    manifest.write_text(json.dumps(document), encoding="utf-8")
    return artifacts


def test_stages_every_verified_profile_image_and_exact_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    co_work = _add_distinct_profile(manifest, tmp_path)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    assets = tmp_path / "assets"
    config_root = tmp_path / "release-config"

    staged_manifest = STAGE.stage_profiles(
        inputs,
        assets,
        config_root,
        ROOT / "config",
    )

    document = json.loads(staged_manifest.read_text(encoding="utf-8"))
    image_url = document["profiles"]["code"]["architectures"][0]["images"][0]["url"]
    assert image_url.startswith("file://")
    for relative in (
        Path("settings/settings.toml"),
        Path("settings/schema.generated.json"),
        Path("corp/corp.toml"),
        Path("corp/enforcement.toml"),
        Path("corp/detection.yaml"),
    ):
        assert (config_root / relative).read_bytes() == (ROOT / "config" / relative).read_bytes()
    assert (config_root / "profiles/code/profile.toml").read_bytes() == artifacts["profile.toml"]
    assert (config_root / "profiles/co-work/profile.toml").read_bytes() == co_work["co-work.toml"]
    assert (assets / "x86_64/vmlinuz").read_bytes() == artifacts["vmlinuz"]
    assert (assets / "x86_64/initrd.img").read_bytes() == artifacts["initrd.img"]
    assert (assets / "x86_64/rootfs.erofs").read_bytes() == artifacts["rootfs.erofs"]
    assert (assets / "x86_64/obom.cdx.json").read_bytes() == artifacts["obom.cdx.json"]
    assert (assets / "x86_64/software-inventory.json").read_bytes() == artifacts[
        "software-inventory.json"
    ]
    for logical_name, payload in (
        ("vmlinuz", artifacts["vmlinuz"]),
        ("initrd.img", artifacts["initrd.img"]),
        ("rootfs.erofs", artifacts["rootfs.erofs"]),
        ("vmlinuz", co_work["co-work-vmlinuz"]),
        ("initrd.img", co_work["co-work-initrd.img"]),
        ("rootfs.erofs", co_work["co-work-rootfs.erofs"]),
        ("obom.cdx.json", artifacts["obom.cdx.json"]),
        ("software-inventory.json", artifacts["software-inventory.json"]),
        ("obom.cdx.json", co_work["co-work-obom.cdx.json"]),
        ("software-inventory.json", co_work["co-work-software-inventory.json"]),
    ):
        digest = blake3.blake3(payload).hexdigest()
        staged = assets / "x86_64" / PROFILE_STAGE.hash_filename(logical_name, digest)
        assert staged.read_bytes() == payload


def _add_python_dependency_pair(
    manifest: Path,
    tmp_path: Path,
    *,
    declare_lock: bool = True,
    publish_lock: bool,
) -> tuple[bytes, bytes]:
    requirements = b"requests==2.32.5\n"
    lock = b"requests==2.32.5 --hash=sha256:" + (b"a" * 64) + b"\n"
    profile = b"""[profile]
id = "code"

[files.python_requirements]
path = "profiles/code/python-requirements.txt"
"""
    if declare_lock:
        profile += b"""
[files.python_requirements_lock]
path = "profiles/code/python-requirements.lock"
"""
    for name, payload in (
        ("profile.toml", profile),
        ("python-requirements.txt", requirements),
        ("python-requirements.lock", lock),
    ):
        (tmp_path / name).write_bytes(payload)

    document = json.loads(manifest.read_text(encoding="utf-8"))
    config = document["profiles"]["code"]["architectures"][0]["config"]
    config[0] = _record(
        "profile.toml",
        profile,
        kind="profile",
        path="profiles/code/profile.toml",
        status="current",
    )
    config.append(
        _record(
            "python-requirements.txt",
            requirements,
            kind="python_requirements",
            path="profiles/code/python-requirements.txt",
            status="current",
        )
    )
    if publish_lock:
        config.append(
            _record(
                "python-requirements.lock",
                lock,
                kind="python_requirements_lock",
                path="profiles/code/python-requirements.lock",
                status="current",
            )
        )
    manifest.write_text(json.dumps(document), encoding="utf-8")
    return requirements, lock


def test_profile_staging_refuses_a_declared_python_lock_missing_from_release_inputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    _add_python_dependency_pair(manifest, tmp_path, publish_lock=False)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match=r"code.*python-requirements\.lock"):
        STAGE.stage_profiles(inputs, tmp_path / "assets", tmp_path / "config", ROOT / "config")


def test_profile_staging_warns_about_legacy_unlocked_python_requirements(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    _add_python_dependency_pair(
        manifest,
        tmp_path,
        declare_lock=False,
        publish_lock=False,
    )
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    # Warns, and stages anyway. Refusing here deadlocks the release lanes
    # against each other: staging reads the already-published profile, the
    # published profiles carry no lock, and only a release can produce one --
    # which is the release this refusal was blocking. Sixteen consecutive
    # trunk failures made that concrete. See `require_paired_files`.
    staged = STAGE.stage_profiles(inputs, tmp_path / "assets", tmp_path / "config", ROOT / "config")
    assert staged, "a legacy unlocked profile must still stage"

    warned = capsys.readouterr().err
    assert "python_requirements without python_requirements_lock" in warned
    assert "unsealed resolver" in warned


def test_profile_staging_refuses_unlocked_requirements_from_a_current_writer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The legacy bridge must close automatically for newly authored profiles."""
    manifest, _ = _write_manifest(tmp_path)
    _add_python_dependency_pair(
        manifest,
        tmp_path,
        declare_lock=False,
        publish_lock=False,
    )
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["profiles"]["code"]["source_commit"] = "a" * 40
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match="python_requirements without python_requirements_lock"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            tmp_path / "config",
            ROOT / "config",
        )


@pytest.mark.parametrize("source_commit", [None, "A" * 40, "a" * 39, "main"])
def test_profile_staging_rejects_a_malformed_source_commit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    source_commit: object,
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["profiles"]["code"]["source_commit"] = source_commit
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match="malformed source_commit"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            tmp_path / "config",
            ROOT / "config",
        )


def test_profile_staging_carries_the_exact_python_requirements_and_lock_together(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    requirements, lock = _add_python_dependency_pair(manifest, tmp_path, publish_lock=True)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    config = tmp_path / "config"

    STAGE.stage_profiles(inputs, tmp_path / "assets", config, ROOT / "config")

    assert (config / "profiles/code/python-requirements.txt").read_bytes() == requirements
    assert (config / "profiles/code/python-requirements.lock").read_bytes() == lock


def test_profile_staging_refuses_missing_configured_evidence_before_package_work(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    evidence = document["profiles"]["code"]["architectures"][0]["evidence"]
    document["profiles"]["code"]["architectures"][0]["evidence"] = [
        record for record in evidence if record["kind"] != "obom"
    ]
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match=r"code/x86_64.*obom\.cdx\.json"):
        STAGE.stage_profiles(inputs, tmp_path / "assets", tmp_path / "config", ROOT / "config")


def test_selected_install_transport_keeps_the_verified_source_graph(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The immutable input report, not generated file URLs, binds its bytes.

    The hosted install lane fetches and verifies the public release graph into
    ``inputs/``. Profile staging rewrites a separate runtime projection to its
    local immutable payloads. Requiring the original graph itself to contain
    those generated URLs rejected the real stable channel only after the
    package and sealed install image had spent nearly an hour building.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.content import ProfileContent, SelectedInstallContent

    manifest, _ = _write_manifest(tmp_path)
    _add_distinct_profile(manifest, tmp_path)
    root = tmp_path / "selected-content"
    inputs = root / "inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    original_manifest = (inputs / "manifest.json").read_bytes()
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    assets = root / "assets"
    config_root = root / "config"

    staged_manifest = STAGE.stage_profiles(inputs, assets, config_root, ROOT / "config")
    config_manifest = config_root / "assets/manifest.json"
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(staged_manifest.read_bytes())

    config = gate_config.load(ROOT)
    selected = SelectedInstallContent(ProfileContent.isolated(config, root))
    selected.require_complete(config, arches=(config.architectures["x86_64"],))

    assert (inputs / "manifest.json").read_bytes() == original_manifest
    assert b"file://" in staged_manifest.read_bytes()


def test_stages_manifest_owned_profile_root_payload_without_checkout_fallback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    root_payload = b"manifest-owned profile payload\n"
    root_manifest = json.dumps(
        {
            "format": "capsem.profile-root.v1",
            "files": [
                {
                    "path": "root/.profile",
                    "hash": f"blake3:{blake3.blake3(root_payload).hexdigest()}",
                    "size": len(root_payload),
                }
            ],
        }
    ).encode()
    (tmp_path / "root.manifest.json").write_bytes(root_manifest)
    (tmp_path / "root-payload").write_bytes(root_payload)
    config = document["profiles"]["code"]["architectures"][0]["config"]
    config.extend(
        [
            _record(
                "root.manifest.json",
                root_manifest,
                kind="root_manifest",
                path="profiles/code/root.manifest.json",
                status="current",
            ),
            _record(
                "root-payload",
                root_payload,
                kind="root_payload",
                path="profiles/code/root/root/.profile",
                status="current",
            ),
        ]
    )
    manifest.write_text(json.dumps(document), encoding="utf-8")
    checkout_payload = ROOT / "config/profiles/code/root/root/.profile"
    assert checkout_payload.read_bytes() != root_payload

    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    config_root = tmp_path / "release-config"
    STAGE.stage_profiles(
        inputs,
        tmp_path / "assets",
        config_root,
        ROOT / "config",
    )

    assert (config_root / "profiles/code/root.manifest.json").read_bytes() == root_manifest
    assert (config_root / "profiles/code/root/root/.profile").read_bytes() == root_payload


def test_legacy_root_manifest_rehydrates_only_exact_verified_checkout_bytes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Old stable graphs select root bytes through their nested manifest."""
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    source_profile = ROOT / "config/profiles/code"
    root_manifest = (source_profile / "root.manifest.json").read_bytes()
    (tmp_path / "root.manifest.json").write_bytes(root_manifest)
    document["profiles"]["code"]["architectures"][0]["config"].append(
        _record(
            "root.manifest.json",
            root_manifest,
            kind="root_manifest",
            path="profiles/code/root.manifest.json",
            status="current",
        )
    )
    manifest.write_text(json.dumps(document), encoding="utf-8")

    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    config_root = tmp_path / "release-config"
    STAGE.stage_profiles(inputs, tmp_path / "assets", config_root, ROOT / "config")

    nested = json.loads(root_manifest)
    for entry in nested["files"]:
        relative = Path(entry["path"])
        assert (config_root / "profiles/code/root" / relative).read_bytes() == (
            source_profile / "root" / relative
        ).read_bytes()


def test_legacy_root_manifest_rejects_a_same_size_checkout_substitution(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    source_profile = ROOT / "config/profiles/code"
    root_manifest = (source_profile / "root.manifest.json").read_bytes()
    (tmp_path / "root.manifest.json").write_bytes(root_manifest)
    document["profiles"]["code"]["architectures"][0]["config"].append(
        _record(
            "root.manifest.json",
            root_manifest,
            kind="root_manifest",
            path="profiles/code/root.manifest.json",
            status="current",
        )
    )
    manifest.write_text(json.dumps(document), encoding="utf-8")
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)

    shared = tmp_path / "shared-config"
    shutil.copytree(ROOT / "config", shared)
    first = json.loads(root_manifest)["files"][0]["path"]
    substitute = shared / "profiles/code/root" / first
    original = substitute.read_bytes()
    substitute.write_bytes(bytes([original[0] ^ 1]) + original[1:])
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match="BLAKE3 mismatch"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            tmp_path / "release-config",
            shared,
        )


def test_staging_reverifies_inputs_instead_of_trusting_the_fetch_report(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = tmp_path / "profile-inputs"
    report = FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    first = inputs / report["artifacts"][0]["path"]
    first.write_bytes(b"tampered")
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")

    with pytest.raises(ValueError, match="byte size mismatch"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            tmp_path / "config",
            ROOT / "config",
        )


def test_profile_staging_rejects_missing_shared_config_before_resetting_destination(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    config_root = tmp_path / "release-config"
    config_root.mkdir()
    sentinel = config_root / "keep-on-validation-failure"
    sentinel.write_text("preserved\n", encoding="utf-8")

    with pytest.raises(ValueError, match="shared config root is missing"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            config_root,
            tmp_path / "missing-shared-config",
        )

    assert sentinel.read_text(encoding="utf-8") == "preserved\n"


def test_profile_staging_rejects_symlinked_or_overlapping_shared_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, _ = _write_manifest(tmp_path)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    shared = tmp_path / "shared"
    (shared / "settings").mkdir(parents=True)
    (shared / "corp").mkdir()
    (shared / "settings/settings.toml").write_text("[app]\n", encoding="utf-8")
    (shared / "corp/corp.toml").write_text(
        'refresh_policy = "24h"\n',
        encoding="utf-8",
    )
    (shared / "corp/enforcement.toml").symlink_to(ROOT / "config/corp/enforcement.toml")

    with pytest.raises(ValueError, match="must not contain symlinks"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            tmp_path / "release-config",
            shared,
        )

    with pytest.raises(ValueError, match="must not overlap"):
        STAGE.stage_profiles(
            inputs,
            tmp_path / "assets",
            shared / "nested-output",
            shared,
        )


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

    def fake_extract(command: tuple[str, ...], check: bool) -> None:
        assert command[:2] == ("dpkg-deb", "--extract")
        assert check is True
        root = Path(command[-1]) / "usr/bin"
        root.mkdir(parents=True)
        for name, payload in binary_payloads.items():
            (root / name).write_bytes(payload)

    monkeypatch.setattr(STAGE.subprocess, "run", fake_extract)
    binary_dir = tmp_path / "target/debug"
    binary_dir.mkdir(parents=True)
    stale = binary_dir / "capsem-source-built"
    stale.write_bytes(b"must-not-survive")

    staged = STAGE.stage_package_binaries(inputs, binary_dir)

    assert STAGE.select_host_package_path(inputs) == inputs / "capsem.deb"
    assert {path.name for path in staged} == set(binary_payloads)
    assert not stale.exists()
    for name, payload in binary_payloads.items():
        assert (binary_dir / name).read_bytes() == payload


def test_profile_lane_marks_old_binary_cohort_incomplete_without_building(
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


def test_profile_lane_accepts_only_the_complete_manifest_binary_cohort(
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

    def fake_extract(command: tuple[str, ...], check: bool) -> None:
        root = Path(command[-1]) / "usr/bin"
        root.mkdir(parents=True)
        (root / "capsem").write_bytes(b"resolved-capsem")

    monkeypatch.setattr(STAGE.subprocess, "run", fake_extract)

    with pytest.raises(ValueError, match="capsem-service"):
        STAGE.stage_package_binaries(inputs, tmp_path / "target/debug")


def test_candidate_package_staging_cannot_fall_back_to_source_binaries(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    package = tmp_path / "candidate.deb"
    package.write_bytes(b"candidate-package")
    payloads = {"capsem": b"candidate-capsem", "capsem-service": b"candidate-service"}

    def fake_extract(command: tuple[str, ...], check: bool) -> None:
        assert Path(command[2]) == package
        root = Path(command[-1]) / "usr/bin"
        root.mkdir(parents=True)
        for name, payload in payloads.items():
            (root / name).write_bytes(payload)

    monkeypatch.setattr(STAGE.subprocess, "run", fake_extract)
    binary_dir = tmp_path / "target/debug"
    binary_dir.mkdir(parents=True)
    (binary_dir / "capsem").write_bytes(b"source-capsem")
    (binary_dir / "capsem-mcp").write_bytes(b"source-only-fallback")

    staged = STAGE.stage_candidate_package(package, binary_dir)

    assert {path.name for path in staged} == set(payloads)
    assert not (binary_dir / "capsem-mcp").exists()
    assert (binary_dir / "capsem").read_bytes() == payloads["capsem"]


def test_release_profile_axis_is_exactly_the_active_manifest_catalog(
    tmp_path: Path,
) -> None:
    profiles_dir = tmp_path / "config/profiles"
    for profile_id in ("code", "experimental"):
        path = profiles_dir / profile_id
        path.mkdir(parents=True)
        (path / "profile.toml").write_text(
            f'id = "{profile_id}"\nname = "{profile_id}"\n',
            encoding="utf-8",
        )
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps(
            {
                "profiles": {
                    "code": {"status": "current"},
                    "experimental": {"status": "staged"},
                    "retired": {"status": "revoked"},
                }
            }
        ),
        encoding="utf-8",
    )

    assert PROFILE_AXIS.release_test_profiles(profiles_dir, manifest) == [
        "code",
        "experimental",
    ]


def test_release_profile_axis_rejects_source_profile_fallback(
    tmp_path: Path,
) -> None:
    profiles_dir = tmp_path / "config/profiles/code"
    profiles_dir.mkdir(parents=True)
    (profiles_dir / "profile.toml").write_text(
        'id = "code"\nname = "Code"\n',
        encoding="utf-8",
    )
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        '{"profiles":{"code":{"status":"current"},"experimental":{"status":"staged"}}}',
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="does not match the selected manifest"):
        PROFILE_AXIS.release_test_profiles(profiles_dir.parent, manifest)


def test_staged_profile_keeps_everything_outside_the_architecture_table(
    tmp_path: Path,
) -> None:
    """Scoping must drop unstaged architectures and nothing else -- the profile
    identity, revision, and rules still have to survive into the package."""
    profile = tmp_path / "profile.toml"
    shutil.copy2(ROOT / "config/profiles/co-work/profile.toml", profile)
    before = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert {"arm64", "x86_64"} <= set(before["assets"]["arch"])

    PROFILE_STAGE.scope_profile_to_arch(profile, "x86_64", "co-work")

    after = tomllib.loads(profile.read_text(encoding="utf-8"))
    assert set(after["assets"]["arch"]) == {"x86_64"}
    assert after["assets"]["arch"]["x86_64"] == before["assets"]["arch"]["x86_64"]
    assert {key: value for key, value in after.items() if key != "assets"} == {
        key: value for key, value in before.items() if key != "assets"
    }


def test_staging_refuses_a_profile_without_the_host_architecture(
    tmp_path: Path,
) -> None:
    """A profile that cannot serve this host is a staging error, not something
    to silently emit an empty architecture table for."""
    profile = tmp_path / "profile.toml"
    shutil.copy2(ROOT / "config/profiles/co-work/profile.toml", profile)

    with pytest.raises(ValueError, match="declares no riscv64 assets"):
        PROFILE_STAGE.scope_profile_to_arch(profile, "riscv64", "co-work")


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
                "profiles": {},
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


def test_empty_package_tolerance_is_rejected_for_the_profile_family(
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
                "profiles": {},
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match=r"package-only|only for packages"):
        FETCH.fetch_release_inputs(
            manifest.as_uri(),
            "profiles",
            tmp_path / "profiles",
            allow_empty_packages=True,
        )

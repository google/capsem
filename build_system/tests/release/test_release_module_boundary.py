from __future__ import annotations

import json
import sys
from copy import deepcopy
from pathlib import Path

import pytest
from capsem_builder.release import (
    obom,
    release_retirement,
    release_source_bootstrap,
    releasechannel,
    runtime_preflight_manifest,
)


def test_release_modules_resolve_from_the_owned_namespace() -> None:
    modules = (
        obom,
        release_retirement,
        release_source_bootstrap,
        releasechannel,
        runtime_preflight_manifest,
    )

    assert {module.__name__ for module in modules} == {
        "capsem_builder.release.obom",
        "capsem_builder.release.release_retirement",
        "capsem_builder.release.release_source_bootstrap",
        "capsem_builder.release.releasechannel",
        "capsem_builder.release.runtime_preflight_manifest",
    }


def test_first_party_channel_dependencies_remain_closed_and_ordered() -> None:
    channel = releasechannel.FirstPartyChannel

    assert channel.STABLE.dependencies == ()
    assert channel.NIGHTLY.dependencies == ("stable",)


@pytest.mark.parametrize(
    ("catalog", "message"),
    [
        ({}, "channels object"),
        ({"channels": {"stable": []}}, "entry must be an object"),
        ({"channels": {"stable": {}}}, "manifests must be an array"),
        ({"channels": {"stable": {"manifests": []}}}, "exactly one current"),
        (
            {"channels": {"stable": {"manifests": [{"status": "current"}]}}},
            "URL is missing",
        ),
        (
            {
                "channels": {
                    "stable": {
                        "manifests": [{"status": "current", "url": "manifest.json"}]
                    }
                }
            },
            "digest must be an object",
        ),
        (
            {
                "channels": {
                    "stable": {
                        "manifests": [
                            {
                                "status": "current",
                                "url": "manifest.json",
                                "digest": {"sha256": "invalid"},
                            }
                        ]
                    }
                }
            },
            "lowercase 64-hex",
        ),
    ],
)
def test_manifest_authority_rejects_ambiguous_or_mutable_selection(
    catalog: dict[str, object], message: str
) -> None:
    with pytest.raises(ValueError, match=message):
        runtime_preflight_manifest._current_manifest_authority(
            catalog,
            release_site="https://release.example",
            channel=releasechannel.FirstPartyChannel.STABLE,
        )


def test_runtime_preflight_main_emits_classification_and_github_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    output = tmp_path / "github-output"
    resolution = runtime_preflight_manifest.ChannelResolution(
        releasechannel.FirstPartyChannel.STABLE,
        runtime_preflight_manifest.ChannelState.PUBLISHED,
        "verified",
        "https://release.example/assets/stable/manifest.json",
        "a" * 64,
    )
    monkeypatch.setattr(
        runtime_preflight_manifest.retirement,
        "load_retired_public_graphs",
        dict,
    )
    monkeypatch.setattr(
        runtime_preflight_manifest,
        "resolve_remote_channel",
        lambda **_kwargs: resolution,
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "runtime-preflight",
            "--channel",
            "stable",
            "--classify-only",
            "--github-output",
            str(output),
        ],
    )

    assert runtime_preflight_manifest.main() == 0
    assert json.loads(capsys.readouterr().out)["state"] == "published"
    assert "state=published" in output.read_text(encoding="utf-8")


def test_github_output_serializes_boolean_selection(tmp_path: Path) -> None:
    output = tmp_path / "github-output"

    runtime_preflight_manifest._write_github_output(
        output, {"bootstrap": True, "retired": False}
    )

    assert output.read_text(encoding="utf-8").splitlines() == [
        "bootstrap=true",
        "retired=false",
    ]


def test_source_manifest_validation_preserves_channel_and_membership() -> None:
    payload = json.dumps(
        {"channel": "nightly", "packages": [], "profiles": {"code": {}}}
    ).encode()

    assert release_source_bootstrap.validate_source_manifest(payload, "nightly") == {
        "channel": "nightly",
        "packages": [],
        "profiles": {"code": {}},
    }


@pytest.mark.parametrize(
    ("document", "message"),
    [
        ([], "JSON object"),
        ({"channel": "stable", "packages": [], "profiles": {}}, "expected 'nightly'"),
        ({"channel": "nightly", "packages": [], "profiles": []}, "profiles"),
        ({"channel": "nightly", "packages": {}, "profiles": {}}, "packages"),
    ],
)
def test_source_manifest_validation_rejects_incomplete_authority(
    document: object, message: str
) -> None:
    with pytest.raises(ValueError, match=message):
        release_source_bootstrap.validate_source_manifest(
            json.dumps(document).encode(), "nightly"
        )

    empty = json.dumps(
        {"channel": "nightly", "packages": [], "profiles": {}}
    ).encode()
    with pytest.raises(ValueError, match="no staged profiles"):
        release_source_bootstrap.validate_binary_source_manifest(empty, "nightly")


def _valid_obom() -> dict[str, object]:
    return {
        "bomFormat": "CycloneDX",
        "metadata": {
            "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
            "component": {
                "type": "operating-system",
                "name": "capsem-rootfs-arm64",
                "version": "guest-rootfs",
                "properties": [
                    "ignored",
                    {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                    {"name": "capsem:guest:architecture", "value": "arm64"},
                ],
            },
        },
        "components": [{"purl": "pkg:deb/debian/bash@1"}],
    }


def _write_obom(path: Path, document: object) -> None:
    path.write_text(json.dumps(document), encoding="utf-8")


def test_obom_validator_accepts_normalized_exported_rootfs(tmp_path: Path) -> None:
    path = tmp_path / "obom.json"
    _write_obom(path, _valid_obom())

    obom.validate_exported_rootfs_obom(path, architecture="arm64")


@pytest.mark.parametrize(
    ("mutation", "message"),
    [
        (("bomFormat", "SPDX"), "CycloneDX"),
        (("metadata", None), "metadata"),
        (("tools", []), "cdxgen"),
        (("component", None), "metadata.component"),
        (("properties", []), "exported rootfs"),
        (("architecture", "x86_64"), "guest architecture"),
        (("components", []), "inventory"),
        (("purl", "pkg:npm/example"), "Debian"),
        (("live-host", True), "live-host"),
    ],
)
def test_obom_validator_rejects_unpublishable_evidence(
    tmp_path: Path, mutation: tuple[str, object], message: str
) -> None:
    document = deepcopy(_valid_obom())
    field, value = mutation
    metadata = document["metadata"]
    assert isinstance(metadata, dict)
    component = metadata["component"]
    assert isinstance(component, dict)
    components = document["components"]
    assert isinstance(components, list)
    if field in {"bomFormat", "metadata", "components"}:
        document[field] = value
    elif field in {"tools", "component"}:
        metadata[field] = value
    elif field == "properties":
        component[field] = value
    elif field == "architecture":
        component["name"] = "capsem-rootfs-x86_64"
    elif field == "purl":
        components[0][field] = value
    else:
        components[0]["properties"] = [
            {"name": "cdx:osquery:category", "value": "process"}
        ]
    path = tmp_path / f"{field}.json"
    _write_obom(path, document)

    with pytest.raises(RuntimeError, match=message):
        obom.validate_exported_rootfs_obom(path, architecture="arm64")


def test_obom_validator_rejects_non_json(tmp_path: Path) -> None:
    path = tmp_path / "broken.json"
    path.write_text("not-json", encoding="utf-8")

    with pytest.raises(RuntimeError, match="invalid JSON"):
        obom.validate_exported_rootfs_obom(path)

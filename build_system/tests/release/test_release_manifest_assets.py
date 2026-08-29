from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
from capsem_builder.release.tools import list_release_manifest_assets as MANIFEST_ASSETS
from capsem_builder.release.tools import verify_channel_downloads

PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT = PROJECT_ROOT / "scripts/list-release-manifest-assets.py"


def _module():
    return MANIFEST_ASSETS


def _record(*, url: str, name: str | None = None, path: str | None = None) -> dict:
    value = {
        "bytes": 12,
        "digest": {"blake3": "a" * 64, "sha256": "b" * 64},
        "url": url,
    }
    if name is not None:
        value["name"] = name
    if path is not None:
        value["path"] = path
    return value


def test_lists_and_deduplicates_every_public_release_graph_artifact() -> None:
    architecture = {
        "architecture": "arm64",
        "image_revision": "2030.0101.1",
        "images": [_record(url="https://cdn.example/vmlinuz", name="vmlinuz")],
        "evidence": [_record(url="/evidence/obom.json", name="obom.json")],
        "config": [_record(url="/profiles/code/profile.toml", path="profiles/code/profile.toml")],
    }
    manifest = {
        "profiles": {
            "code": {"architectures": [architecture]},
            "co-work": {"architectures": [json.loads(json.dumps(architecture))]},
        }
    }

    rows = _module().manifest_asset_rows(
        manifest, "https://release.example/assets/stable/manifest.json"
    )

    assert len(rows) == 3
    assert {row[5] for row in rows} == {
        "https://cdn.example/vmlinuz",
        "https://release.example/evidence/obom.json",
        "https://release.example/profiles/code/profile.toml",
    }


def test_lists_legacy_asset_manifest_urls() -> None:
    manifest = {
        "asset_base": "/assets/releases",
        "assets": {
            "current": "2030.0101.1",
            "releases": {
                "2030.0101.1": {
                    "arches": {
                        "arm64": {
                            "vmlinuz": {"hash": "a" * 64, "size": 12},
                        }
                    }
                }
            },
        },
    }

    rows = _module().manifest_asset_rows(
        manifest, "https://release.example/assets/stable/manifest.json"
    )

    assert rows == [
        (
            "2030.0101.1",
            "arm64",
            "vmlinuz",
            "a" * 64,
            12,
            "https://release.example/assets/releases/2030.0101.1/arm64-vmlinuz",
        )
    ]


@pytest.mark.parametrize(
    "manifest",
    [{}, {"profiles": {}}, {"profiles": {"code": {"architectures": []}}}],
)
def test_rejects_unknown_or_incomplete_manifest_shapes(manifest: dict) -> None:
    with pytest.raises(ValueError):
        _module().manifest_asset_rows(
            manifest, "https://release.example/assets/stable/manifest.json"
        )


def test_release_workflow_uses_shared_dual_schema_asset_lister() -> None:
    """The enumeration is shared, and the verification is now testable.

    This used to read the workflow step's shell body, because that is where the
    `curl` loop, the byte comparison and a blake3 check written as an indented
    Python heredoc lived. A program inside YAML is one no test can call, which
    is why the last publication step had no coverage at all -- and it is the
    step whose absence let a channel serve `status: current` with three dead
    package URLs for a month.

    So the assertions follow the code down: the step runs the verifier, and the
    verifier enumerates through the one shared dual-schema lister rather than
    parsing a manifest shape of its own.
    """
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    step = workflow.split(
        "- name: Verify every URL in the asset channel manifest is reachable",
        maxsplit=1,
    )[1].split(
        "- name: Verify public release packages and installer contract",
        maxsplit=1,
    )[0]
    assert "scripts/verify-channel-downloads.py" in step

    verifier = Path(verify_channel_downloads.__file__).read_text()
    assert "manifest_asset_rows" in verifier
    assert "m['assets']['current']" not in verifier
    # The three questions a published row has to answer.
    assert "not reachable" in verifier
    assert "expected_bytes" in verifier
    assert "blake3" in verifier


def test_cli_emits_tab_separated_rows(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps(
            {
                "assets": {
                    "current": "2030.0101.1",
                    "releases": {
                        "2030.0101.1": {
                            "arches": {
                                "arm64": {
                                    "vmlinuz": {"hash": "a" * 64, "size": 12}
                                }
                            }
                        }
                    },
                }
            }
        )
    )
    result = subprocess.run(
        [
            "python3",
            str(SCRIPT),
            "--manifest-path",
            str(manifest),
            "--manifest-url",
            "https://release.example/assets/stable/manifest.json",
        ],
        text=True,
        capture_output=True,
        check=True,
    )

    assert result.stdout.count("\t") == 5

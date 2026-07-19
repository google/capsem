from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = PROJECT_ROOT / "scripts/list-release-manifest-assets.py"


def _module():
    spec = importlib.util.spec_from_file_location("list_release_manifest_assets", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    step = workflow.split(
        "- name: Verify every URL in the asset channel manifest is reachable",
        maxsplit=1,
    )[1].split(
        "- name: Verify public release packages and installer contract",
        maxsplit=1,
    )[0]

    assert "scripts/list-release-manifest-assets.py" in step
    assert "m['assets']['current']" not in step
    assert 'blob="/tmp/verify/${hash#blake3:}"' in step
    assert 'actual_bytes=$(wc -c < "$blob"' in step
    assert 'if [ "$actual_bytes" != "$bytes" ]; then' in step


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

"""Manual VM asset release no-op gate tests."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-asset-release-delta.py"


def _manifest(path: Path, *, version: str, rootfs_hash: str = "a" * 64) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": version,
            "releases": {
                version: {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_binary": "1.4.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": {"hash": "b" * 64, "size": 11},
                            "initrd.img": {"hash": "c" * 64, "size": 12},
                            "rootfs.erofs": {"hash": rootfs_hash, "size": 13},
                            "obom.cdx.json": {"hash": "d" * 64, "size": 14},
                        }
                    },
                }
            },
        },
        "binaries": {"current": "1.4.1", "releases": {}},
    }
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    return path


def _run_delta(
    new_manifest: Path, previous_manifest: Path, output_path: Path, summary_path: Path
) -> dict:
    env = os.environ.copy()
    env["GITHUB_OUTPUT"] = str(output_path)
    result = subprocess.run(
        [
            str(SCRIPT),
            "--new-manifest",
            str(new_manifest),
            "--previous-manifest-url",
            f"file://{previous_manifest}",
            "--summary",
            str(summary_path),
        ],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        check=True,
    )
    return json.loads(result.stdout)


def test_asset_release_noop_detects_unchanged_hashes_and_sets_outputs(tmp_path: Path) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0101.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0101.2")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"

    result = _run_delta(new, previous, output, summary)

    assert result == {
        "changed": False,
        "reason": "asset_hashes_unchanged",
        "previous_assets": "2030.0101.1",
        "new_assets": "2030.0101.2",
    }
    assert "changed=false" in output.read_text(encoding="utf-8")
    assert "reason=asset_hashes_unchanged" in output.read_text(encoding="utf-8")
    assert "release-channel deploy will be skipped" in summary.read_text(encoding="utf-8")


def test_asset_release_noop_allows_changed_hashes_to_publish(tmp_path: Path) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0101.1")
    new = _manifest(
        tmp_path / "new" / "manifest.json",
        version="2030.0101.2",
        rootfs_hash="e" * 64,
    )
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"

    result = _run_delta(new, previous, output, summary)

    assert result["changed"] is True
    assert result["reason"] == "asset_hashes_changed"
    assert result["previous_assets"] == "2030.0101.1"
    assert result["new_assets"] == "2030.0101.2"
    assert "changed=true" in output.read_text(encoding="utf-8")
    assert "Asset publication should continue" in summary.read_text(encoding="utf-8")


def test_asset_release_noop_rejects_missing_previous_manifest_by_default(tmp_path: Path) -> None:
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0101.2")

    result = subprocess.run(
        [
            str(SCRIPT),
            "--new-manifest",
            str(new),
            "--previous-manifest-url",
            f"file://{tmp_path / 'missing' / 'manifest.json'}",
        ],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )

    assert result.returncode != 0
    assert "could not read previous manifest" in result.stderr


def test_asset_release_noop_gate_controls_preview_and_deploy_workflow() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-assets.yaml").read_text()

    assert "scripts/check-asset-release-delta.py" in workflow
    assert (
        '--previous-manifest-url "https://release.capsem.org/assets/$CHANNEL/manifest.json"'
        in workflow
    )
    assert "outputs:" in workflow
    assert "asset_changed: ${{ steps.asset-delta.outputs.changed }}" in workflow
    assert "if: ${{ steps.asset-delta.outputs.changed == 'true' }}" in workflow
    assert "name: asset-channel-preview" in workflow
    assert (
        "if: ${{ inputs.dry_run == false && needs.assemble-channel.outputs.asset_changed == 'true' }}"
        in workflow
    )

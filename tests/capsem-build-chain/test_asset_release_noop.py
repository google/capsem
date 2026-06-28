"""Manual VM asset release no-op gate tests."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-asset-release-delta.py"
PRESERVE_BINARY_SCRIPT = (
    PROJECT_ROOT / "scripts" / "preserve-binary-channel-metadata.py"
)


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


def _add_historical_release(
    manifest_path: Path,
    *,
    version: str = "2030.0100.1",
    deprecated: bool = False,
    deprecated_date: str | None = None,
) -> None:
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    release = json.loads(
        json.dumps(data["assets"]["releases"][data["assets"]["current"]])
    )
    release["date"] = "2030-01-01"
    release["deprecated"] = deprecated
    if deprecated_date is None:
        release.pop("deprecated_date", None)
    else:
        release["deprecated_date"] = deprecated_date
    data["assets"]["releases"][version] = release
    manifest_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def _update_current_release_metadata(manifest_path: Path, **updates: object) -> None:
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    current = data["assets"]["current"]
    data["assets"]["releases"][current].update(updates)
    manifest_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def _update_manifest_metadata(manifest_path: Path, **updates: object) -> None:
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    data.update(updates)
    manifest_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def _add_binary_release_metadata(manifest_path: Path, *, version: str) -> None:
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    data["binaries"] = {
        "current": version,
        "releases": {
            version: {
                "date": "2030-01-05",
                "deprecated": False,
                "min_assets": data["assets"]["current"],
                "version": version,
                "files": [
                    {
                        "name": f"Capsem-{version}.pkg",
                        "size": 42,
                        "sha256": "1" * 64,
                    },
                    {
                        "name": "capsem-sbom.spdx.json",
                        "size": 43,
                        "sha256": "2" * 64,
                    },
                ],
            }
        },
    }
    manifest_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def _run_delta(
    new_manifest: Path,
    previous_manifest: Path,
    output_path: Path,
    summary_path: Path,
    json_output_path: Path | None = None,
) -> dict:
    env = os.environ.copy()
    env["GITHUB_OUTPUT"] = str(output_path)
    command = [
        str(SCRIPT),
        "--new-manifest",
        str(new_manifest),
        "--previous-manifest-url",
        f"file://{previous_manifest}",
        "--summary",
        str(summary_path),
    ]
    if json_output_path is not None:
        command.extend(["--json-output", str(json_output_path)])
    result = subprocess.run(
        command,
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
        "asset_blobs_changed": False,
        "reason": "asset_hashes_unchanged",
        "previous_assets": "2030.0101.1",
        "new_assets": "2030.0101.2",
    }
    assert "changed=false" in output.read_text(encoding="utf-8")
    assert "asset_blobs_changed=false" in output.read_text(encoding="utf-8")
    assert "reason=asset_hashes_unchanged" in output.read_text(encoding="utf-8")
    assert "release-channel deploy will be skipped" in summary.read_text(encoding="utf-8")


def test_asset_release_delta_deploys_deprecation_without_republishing_blobs(
    tmp_path: Path,
) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0102.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0102.1")
    _add_historical_release(previous, deprecated=False)
    _add_historical_release(new, deprecated=True, deprecated_date="2030-01-03")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"

    result = _run_delta(new, previous, output, summary)

    assert result == {
        "changed": True,
        "asset_blobs_changed": False,
        "reason": "asset_release_metadata_changed",
        "previous_assets": "2030.0102.1",
        "new_assets": "2030.0102.1",
    }
    assert "changed=true" in output.read_text(encoding="utf-8")
    assert "asset_blobs_changed=false" in output.read_text(encoding="utf-8")
    assert "reason=asset_release_metadata_changed" in output.read_text(encoding="utf-8")
    assert "Release-channel metadata changed" in summary.read_text(encoding="utf-8")
    assert "immutable VM blobs will not be republished" in summary.read_text(
        encoding="utf-8"
    )


def test_asset_release_delta_deploys_current_metadata_without_republishing_blobs(
    tmp_path: Path,
) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0102.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0102.1")
    _update_current_release_metadata(new, min_binary="1.4.2")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"

    result = _run_delta(new, previous, output, summary)

    assert result == {
        "changed": True,
        "asset_blobs_changed": False,
        "reason": "asset_release_metadata_changed",
        "previous_assets": "2030.0102.1",
        "new_assets": "2030.0102.1",
    }
    assert "changed=true" in output.read_text(encoding="utf-8")
    assert "asset_blobs_changed=false" in output.read_text(encoding="utf-8")
    assert "Release-channel metadata changed" in summary.read_text(encoding="utf-8")


def test_asset_release_delta_deploys_manifest_policy_without_republishing_blobs(
    tmp_path: Path,
) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0102.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0102.1")
    _update_manifest_metadata(new, refresh_policy="12h")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"

    result = _run_delta(new, previous, output, summary)

    assert result == {
        "changed": True,
        "asset_blobs_changed": False,
        "reason": "asset_release_metadata_changed",
        "previous_assets": "2030.0102.1",
        "new_assets": "2030.0102.1",
    }
    assert "changed=true" in output.read_text(encoding="utf-8")
    assert "asset_blobs_changed=false" in output.read_text(encoding="utf-8")
    assert "Release-channel metadata changed" in summary.read_text(encoding="utf-8")


def test_asset_release_preserves_live_binary_metadata_before_channel_build(
    tmp_path: Path,
) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0102.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0103.1")
    _add_binary_release_metadata(previous, version="1.4.2234567890")

    subprocess.run(
        [
            str(PRESERVE_BINARY_SCRIPT),
            "--manifest-path",
            str(new),
            "--previous-manifest-url",
            f"file://{previous}",
        ],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    merged = json.loads(new.read_text(encoding="utf-8"))
    previous_data = json.loads(previous.read_text(encoding="utf-8"))
    assert merged["assets"]["current"] == "2030.0103.1"
    assert merged["binaries"] == previous_data["binaries"]


def test_asset_release_binary_metadata_preserver_bootstraps_when_previous_missing(
    tmp_path: Path,
) -> None:
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0103.1")
    before = json.loads(new.read_text(encoding="utf-8"))

    result = subprocess.run(
        [
            str(PRESERVE_BINARY_SCRIPT),
            "--manifest",
            str(new),
            "--previous-manifest-url",
            f"file://{tmp_path / 'missing' / 'manifest.json'}",
            "--allow-missing-previous",
        ],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )

    assert json.loads(new.read_text(encoding="utf-8")) == before
    assert "could not read previous manifest" in result.stderr
    assert json.loads(result.stdout)["binary_metadata_preserved"] is False


def test_asset_release_delta_writes_reviewable_json_output(tmp_path: Path) -> None:
    previous = _manifest(tmp_path / "previous" / "manifest.json", version="2030.0101.1")
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0101.2")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"
    json_output = tmp_path / "artifact" / "delta.json"

    result = _run_delta(new, previous, output, summary, json_output)

    assert json.loads(json_output.read_text(encoding="utf-8")) == result
    assert json_output.read_text(encoding="utf-8").endswith("\n")


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
    assert result["asset_blobs_changed"] is True
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


def test_asset_workflow_allows_missing_previous_manifest_for_first_channel_publish(
    tmp_path: Path,
) -> None:
    new = _manifest(tmp_path / "new" / "manifest.json", version="2030.0101.2")
    output = tmp_path / "github-output"
    summary = tmp_path / "summary.md"
    env = os.environ.copy()
    env["GITHUB_OUTPUT"] = str(output)

    result = subprocess.run(
        [
            str(SCRIPT),
            "--new-manifest",
            str(new),
            "--previous-manifest-url",
            f"file://{tmp_path / 'missing' / 'manifest.json'}",
            "--allow-missing-previous",
            "--summary",
            str(summary),
        ],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        check=True,
    )

    assert json.loads(result.stdout) == {
        "changed": True,
        "asset_blobs_changed": True,
        "reason": "previous_manifest_unavailable",
        "new_assets": "2030.0101.2",
    }
    assert "warning: could not read previous manifest" in result.stderr
    assert "changed=true" in output.read_text(encoding="utf-8")
    assert "reason=previous_manifest_unavailable" in output.read_text(encoding="utf-8")
    assert "Asset publication should continue" in summary.read_text(encoding="utf-8")


def test_asset_release_noop_gate_controls_preview_and_deploy_workflow() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-assets.yaml").read_text()
    assemble_channel = workflow.split("  assemble-channel:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]

    assert "scripts/check-asset-release-delta.py" in workflow
    assert "binary_version:" not in workflow
    assert "BINARY_VERSION" not in workflow
    assert '--version "$BINARY_VERSION"' not in workflow
    assert "scripts/preserve-binary-channel-metadata.py" in workflow
    assert "--manifest-path assets/manifest.json" in workflow
    assert "--manifest assets/manifest.json" not in workflow
    assert assemble_channel.index("scripts/preserve-binary-channel-metadata.py") < (
        assemble_channel.index("scripts/check-asset-release-delta.py")
    )
    assert assemble_channel.index("scripts/preserve-binary-channel-metadata.py") < (
        assemble_channel.index("cargo run -p capsem-admin -- assets channel build")
    )
    assert (
        '--previous-manifest-url "https://release.capsem.org/assets/$CHANNEL/manifest.json"'
        in workflow
    )
    assert "--allow-missing-previous" in workflow
    assert "outputs:" in workflow
    assert "asset_changed: ${{ steps.asset-delta.outputs.changed }}" in workflow
    assert "asset_blobs_changed: ${{ steps.asset-delta.outputs.asset_blobs_changed }}" in workflow
    assert "if: ${{ steps.asset-delta.outputs.changed == 'true' }}" in workflow
    assert "name: asset-release-plan" in workflow
    assert (
        "if: ${{ inputs.dry_run == true && steps.asset-delta.outputs.asset_blobs_changed == 'true' }}"
        in workflow
    )
    assert "path: target/asset-release/" in workflow
    assert "--json-output target/asset-release-delta/delta.json" in workflow
    assert "name: asset-release-delta" in workflow
    assert "path: target/asset-release-delta/" in workflow
    assert "name: asset-channel-preview" in workflow
    assert (
        "if: ${{ inputs.dry_run == false && needs.assemble-channel.outputs.asset_changed == 'true' }}"
        in workflow
    )


def test_asset_release_upload_publishes_arch_prefixed_immutable_release_only_when_live() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-assets.yaml").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    upload_step = workflow.split("- name: Publish immutable GitHub asset release", maxsplit=1)[
        1
    ].split("\n      - uses: actions/upload-artifact@v7", maxsplit=1)[0]

    assert "contents: write" in workflow
    assert "if: ${{ steps.asset-delta.outputs.asset_blobs_changed == 'true' }}" in upload_step
    assert "DRY_RUN: ${{ inputs.dry_run }}" in upload_step
    assert "GH_TOKEN: ${{ github.token }}" in upload_step
    assert "ASSET_VERSION=$(python - <<" in upload_step
    assert 'json.load(handle)["assets"]["current"]' in upload_step
    assert 'TAG="assets-v$ASSET_VERSION"' in upload_step
    for logical_name in ("vmlinuz", "initrd.img", "rootfs.erofs", "obom.cdx.json"):
        assert logical_name in upload_step
    assert 'cp "$src" "$RELEASE_DIR/$arch-$logical_name"' in upload_step
    assert "gh release view %q" in upload_step
    assert "gh release upload %q" in upload_step
    assert "--clobber" in upload_step
    assert "gh release create %q" in upload_step
    assert "--target %q" in upload_step
    assert 'if [[ "$DRY_RUN" == "true" ]]; then' in upload_step
    assert "DRY-RUN:" in upload_step
    assert '"$UPLOAD_SCRIPT"' in upload_step
    assert 'UPLOAD_SCRIPT="target/asset-release/upload-assets.sh"' in upload_step

    for text in (docs, release_skill):
        assert "assets-v<asset-version>" in text
        assert "arch-prefixed" in text
        assert "asset-release-plan" in text
        assert "asset-release-delta" in text
        for logical_name in ("`vmlinuz`", "`initrd.img`", "`rootfs.erofs`", "`obom.cdx.json`"):
            assert logical_name in text

"""Corporate manifest authoring stays inside capsem-admin ownership boundaries."""

from __future__ import annotations

import json
import subprocess
from copy import deepcopy
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
RUNTIME_BASE = "https://releases.acme.test/acme/"
SOURCE_COMMIT = "1" * 40
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)


def _run_admin(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


def _write_authoring_inputs(tmp_path: Path) -> tuple[Path, dict, Path, dict, dict]:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = graph["manifests"]["stable"]["1.0.2"]
    nightly = graph["manifests"]["nightly"]["1.0.2"]
    official = {
        "version": "1.0.0",
        "status": "current",
        "packages": stable["packages"] + nightly["packages"],
    }
    corporate_runtime = deepcopy(stable["runtime"])
    _rewrite_runtime_references(corporate_runtime)
    runtime_source = {
        "version": "1.0.0",
        "status": "current",
        "packages": deepcopy(nightly["packages"]),
        "runtime": corporate_runtime,
    }
    official_path = tmp_path / "official-capsem.json"
    runtime_path = tmp_path / "acme-runtime.json"
    official_path.write_text(json.dumps(official), encoding="utf-8")
    runtime_path.write_text(json.dumps(runtime_source), encoding="utf-8")
    return official_path, official, runtime_path, runtime_source, stable


def _rewrite_runtime_references(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"url", "evidence"} and isinstance(child, str):
                # Keep the architecture directory: a runtime URL names its arch.
                value[key] = RUNTIME_BASE + "/".join(Path(child).parts[-2:])
            else:
                _rewrite_runtime_references(child)
    elif isinstance(value, list):
        for child in value:
            _rewrite_runtime_references(child)


def test_corporate_manifest_contract_supports_latest_and_exact_pin(
    tmp_path: Path,
) -> None:
    official_path, official, runtime_path, runtime_source, stable = _write_authoring_inputs(
        tmp_path
    )
    official_before = official_path.read_bytes()
    runtime_before = runtime_path.read_bytes()
    output_root = tmp_path / "corporate"

    latest = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "engineering",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(runtime_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "latest",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
        "--json",
    )

    assert latest.returncode == 0, latest.stderr
    latest_report = json.loads(latest.stdout)
    latest_manifest = json.loads(
        (output_root / "acme" / "engineering" / "manifest.json").read_text(encoding="utf-8")
    )
    assert latest_report["schema"] == "capsem.admin.corporate_manifest.v1"
    assert latest_report["binary_policy"] == "latest"
    assert latest_report["resolved_binary_version"] == "1.5.0-nightly.20260702"
    assert {package["version"] for package in latest_manifest["packages"]} == {
        "1.5.0-nightly.20260702"
    }
    expected_runtime = deepcopy(runtime_source["runtime"])
    expected_runtime["source_commit"] = SOURCE_COMMIT
    assert latest_manifest["runtime"] == expected_runtime
    assert latest_report["runtime_revision"] == expected_runtime["revision"]
    assert "source_commit" not in latest_manifest
    assert all("source_commit" not in package for package in latest_manifest["packages"])

    pinned_runtime_source = deepcopy(runtime_source)
    pinned_runtime_source["packages"] = deepcopy(stable["packages"])
    pinned_runtime_path = tmp_path / "acme-pinned-runtime.json"
    pinned_runtime_path.write_text(json.dumps(pinned_runtime_source), encoding="utf-8")
    pinned = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "production",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(pinned_runtime_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "1.4.0",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
        "--json",
    )

    assert pinned.returncode == 0, pinned.stderr
    pinned_report = json.loads(pinned.stdout)
    pinned_manifest = json.loads(
        (output_root / "acme" / "production" / "manifest.json").read_text(encoding="utf-8")
    )
    assert pinned_report["binary_policy"] == "1.4.0"
    assert pinned_report["resolved_binary_version"] == "1.4.0"
    assert pinned_manifest["packages"] == stable["packages"]
    assert official_path.read_bytes() == official_before
    assert runtime_path.read_bytes() == runtime_before
    assert "runtime" not in official


def test_corporate_manifest_contract_rejects_foreign_writes(tmp_path: Path) -> None:
    official_path, _, runtime_path, runtime_source, _ = _write_authoring_inputs(tmp_path)
    output_root = tmp_path / "corporate"

    tampered = deepcopy(runtime_source)
    tampered["packages"][0]["digest"]["sha256"] = "f" * 64
    tampered_path = tmp_path / "tampered-runtime.json"
    tampered_path.write_text(json.dumps(tampered), encoding="utf-8")
    binary_write = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "engineering",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(tampered_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "latest",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
    )
    assert binary_write.returncode != 0
    assert "may reference only the selected official packages" in binary_write.stderr
    assert not (output_root / "acme" / "engineering" / "manifest.json").exists()

    for corporation, channel in (
        ("acme", "stable"),
        ("acme", "nightly"),
        ("capsem", "corp"),
    ):
        first_party_write = _run_admin(
            "manifest",
            "corporate",
            "--corporation",
            corporation,
            "--channel",
            channel,
            "--official-manifest",
            str(official_path),
            "--runtime-manifest",
            str(runtime_path),
            "--runtime-base",
            RUNTIME_BASE,
            "--binary",
            "latest",
            "--source-commit",
            SOURCE_COMMIT,
            "--output-root",
            str(output_root),
        )
        assert first_party_write.returncode != 0
        assert "first-party namespace" in first_party_write.stderr

    unsupported_pin = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "engineering",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(runtime_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "9.9.9",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
    )
    assert unsupported_pin.returncode != 0
    assert "official manifest does not publish Capsem 9.9.9" in unsupported_pin.stderr

    foreign_runtime = deepcopy(runtime_source)
    foreign_runtime["runtime"]["architectures"][0]["images"][0]["url"] = (
        "https://release.capsem.org/runtime/vmlinuz"
    )
    foreign_runtime_path = tmp_path / "foreign-runtime.json"
    foreign_runtime_path.write_text(json.dumps(foreign_runtime), encoding="utf-8")
    foreign_runtime_write = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "security",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(foreign_runtime_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "latest",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
    )
    assert foreign_runtime_write.returncode != 0
    assert "outside the owned runtime base" in foreign_runtime_write.stderr
    assert not (output_root / "acme" / "security" / "manifest.json").exists()

    incompatible = deepcopy(runtime_source)
    incompatible["runtime"]["min_capsem_version"] = "9.0.0"
    incompatible_path = tmp_path / "incompatible-runtime.json"
    incompatible_path.write_text(json.dumps(incompatible), encoding="utf-8")
    incompatible_selection = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "research",
        "--official-manifest",
        str(official_path),
        "--runtime-manifest",
        str(incompatible_path),
        "--runtime-base",
        RUNTIME_BASE,
        "--binary",
        "latest",
        "--source-commit",
        SOURCE_COMMIT,
        "--output-root",
        str(output_root),
    )
    assert incompatible_selection.returncode != 0
    assert "requires Capsem 9.0.0 or newer" in incompatible_selection.stderr
    assert not (output_root / "acme" / "research" / "manifest.json").exists()


def test_corporate_manifest_has_no_non_admin_authoring_entrypoint() -> None:
    admin = (PROJECT_ROOT / "crates/capsem-admin/src/main.rs").read_text(encoding="utf-8")
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    workflows = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (PROJECT_ROOT / ".github/workflows").glob("*.yaml")
    )

    assert "Corporate(ManifestCorporateArgs)" in admin
    assert "corporate_manifest_command" in admin
    assert "\nrelease-corporate" not in justfile
    assert "manifest corporate" not in justfile
    assert "manifest corporate" not in workflows

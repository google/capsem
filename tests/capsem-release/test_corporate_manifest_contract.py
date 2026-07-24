"""Corporate manifest authoring stays inside capsem-admin ownership boundaries."""

from __future__ import annotations

import json
import subprocess
from copy import deepcopy
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILE_BASE = "https://releases.acme.test/acme/"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def _run_admin(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        cwd=PROJECT_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
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
        "profiles": {},
    }
    corporate_profiles = deepcopy(stable["profiles"])
    _rewrite_profile_references(corporate_profiles)
    profile_source = {
        "version": "1.0.0",
        "status": "current",
        "packages": deepcopy(nightly["packages"]),
        "profiles": corporate_profiles,
    }
    official_path = tmp_path / "official-capsem.json"
    profile_path = tmp_path / "acme-profiles.json"
    official_path.write_text(json.dumps(official), encoding="utf-8")
    profile_path.write_text(json.dumps(profile_source), encoding="utf-8")
    return official_path, official, profile_path, profile_source, stable


def _rewrite_profile_references(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {"url", "evidence"} and isinstance(child, str):
                value[key] = PROFILE_BASE + Path(child).name
            else:
                _rewrite_profile_references(child)
    elif isinstance(value, list):
        for child in value:
            _rewrite_profile_references(child)


def test_corporate_manifest_contract_supports_latest_and_exact_pin(tmp_path: Path) -> None:
    official_path, official, profile_path, profile_source, stable = (
        _write_authoring_inputs(tmp_path)
    )
    official_before = official_path.read_bytes()
    profile_before = profile_path.read_bytes()
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
        "--profile-manifest",
        str(profile_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "latest",
        "--output-root",
        str(output_root),
        "--json",
    )

    assert latest.returncode == 0, latest.stderr
    latest_report = json.loads(latest.stdout)
    latest_manifest = json.loads(
        (output_root / "acme" / "engineering" / "manifest.json").read_text(
            encoding="utf-8"
        )
    )
    assert latest_report["schema"] == "capsem.admin.corporate_manifest.v1"
    assert latest_report["binary_policy"] == "latest"
    assert latest_report["resolved_binary_version"] == "1.5.0-nightly.20260702"
    assert {package["version"] for package in latest_manifest["packages"]} == {
        "1.5.0-nightly.20260702"
    }
    assert latest_manifest["profiles"] == profile_source["profiles"]

    pinned_profile_source = deepcopy(profile_source)
    pinned_profile_source["packages"] = deepcopy(stable["packages"])
    pinned_profile_path = tmp_path / "acme-pinned-profiles.json"
    pinned_profile_path.write_text(json.dumps(pinned_profile_source), encoding="utf-8")
    pinned = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "production",
        "--official-manifest",
        str(official_path),
        "--profile-manifest",
        str(pinned_profile_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "1.4.0",
        "--output-root",
        str(output_root),
        "--json",
    )

    assert pinned.returncode == 0, pinned.stderr
    pinned_report = json.loads(pinned.stdout)
    pinned_manifest = json.loads(
        (output_root / "acme" / "production" / "manifest.json").read_text(
            encoding="utf-8"
        )
    )
    assert pinned_report["binary_policy"] == "1.4.0"
    assert pinned_report["resolved_binary_version"] == "1.4.0"
    assert pinned_manifest["packages"] == stable["packages"]
    assert official_path.read_bytes() == official_before
    assert profile_path.read_bytes() == profile_before
    assert official["profiles"] == {}


def test_corporate_manifest_contract_rejects_foreign_writes(tmp_path: Path) -> None:
    official_path, _, profile_path, profile_source, _ = _write_authoring_inputs(
        tmp_path
    )
    output_root = tmp_path / "corporate"

    tampered = deepcopy(profile_source)
    tampered["packages"][0]["digest"]["sha256"] = "f" * 64
    tampered_path = tmp_path / "tampered-profiles.json"
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
        "--profile-manifest",
        str(tampered_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "latest",
        "--output-root",
        str(output_root),
    )
    assert binary_write.returncode != 0
    assert "may reference only the selected official packages" in binary_write.stderr
    assert not (output_root / "acme" / "engineering" / "manifest.json").exists()

    for corporation, channel in (("acme", "stable"), ("acme", "nightly"), ("capsem", "corp")):
        first_party_write = _run_admin(
            "manifest",
            "corporate",
            "--corporation",
            corporation,
            "--channel",
            channel,
            "--official-manifest",
            str(official_path),
            "--profile-manifest",
            str(profile_path),
            "--profile-base",
            PROFILE_BASE,
            "--binary",
            "latest",
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
        "--profile-manifest",
        str(profile_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "9.9.9",
        "--output-root",
        str(output_root),
    )
    assert unsupported_pin.returncode != 0
    assert "official manifest does not publish Capsem 9.9.9" in unsupported_pin.stderr

    foreign_profile = deepcopy(profile_source)
    foreign_profile["profiles"]["code"]["architectures"][0]["config"][0][
        "url"
    ] = "https://release.capsem.org/profiles/code.toml"
    foreign_profile_path = tmp_path / "foreign-profile.json"
    foreign_profile_path.write_text(json.dumps(foreign_profile), encoding="utf-8")
    foreign_profile_write = _run_admin(
        "manifest",
        "corporate",
        "--corporation",
        "acme",
        "--channel",
        "security",
        "--official-manifest",
        str(official_path),
        "--profile-manifest",
        str(foreign_profile_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "latest",
        "--output-root",
        str(output_root),
    )
    assert foreign_profile_write.returncode != 0
    assert "outside the owned profile base" in foreign_profile_write.stderr
    assert not (output_root / "acme" / "security" / "manifest.json").exists()

    incompatible = deepcopy(profile_source)
    incompatible["profiles"]["code"]["min_capsem_version"] = "9.0.0"
    incompatible_path = tmp_path / "incompatible-profile.json"
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
        "--profile-manifest",
        str(incompatible_path),
        "--profile-base",
        PROFILE_BASE,
        "--binary",
        "latest",
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

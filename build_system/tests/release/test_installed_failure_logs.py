"""Installed release proof for post-mortem session logs."""

from pathlib import Path

import pytest
from capsem_builder.release.tools.verify_installed_release import (
    verify_failed_session_logs,
)


def test_installed_probe_reads_and_removes_preserved_failure(tmp_path: Path) -> None:
    capsem_home = tmp_path / "home"
    capsem = tmp_path / "capsem"
    capsem.write_text(
        """#!/usr/bin/env python3
import os
from pathlib import Path
import sys

session_id = sys.argv[2]
root = Path(os.environ["CAPSEM_RUN_DIR"]) / "sessions"
failed = next(root.glob(f"{session_id}-failed-*"))
print((failed / "process.log").read_text(), end="")
""",
        encoding="utf-8",
    )
    capsem.chmod(0o755)

    verify_failed_session_logs(capsem, capsem_home)

    sessions = capsem_home / "run" / "sessions"
    assert list(sessions.iterdir()) == []


def test_help_needs_no_home_directory(monkeypatch, capsys) -> None:
    """The verifier runs staged in a bare guest and under service managers.

    An OS Login account has no /etc/passwd entry, so with HOME unset
    Path.home() raises; the parser used it for a default and failed on
    `--help` before any option was even read.
    """
    import pytest
    from capsem_builder.release.tools import verify_installed_release

    def unresolvable() -> Path:
        raise RuntimeError("Could not determine home directory.")

    monkeypatch.setattr(Path, "home", staticmethod(unresolvable))
    monkeypatch.setattr("sys.argv", ["verify-installed-release.py", "--help"])
    with pytest.raises(SystemExit) as exited:
        verify_installed_release.main()
    assert exited.value.code == 0
    assert "--capsem-home" in capsys.readouterr().out


def _installed_status_fixture(tmp_path, monkeypatch, *, legacy, status_line, damage=""):
    """Real verifier/parser/artifact joins over source-labelled synthetic inputs."""
    import json

    from capsem_builder import crypto
    from capsem_builder.release.tools import verify_installed_release as verify

    from build_system.tests.packaging.test_macos_binary_apply import cohort
    from build_system.tests.packaging.test_macos_public_upgrade import PUBLIC

    package, manifest_path, _sbom = cohort(tmp_path, "before" if legacy else "after")
    manifest = json.loads(manifest_path.read_text())
    version = "0.6.6" if legacy else "0.7.1"
    manifest["channel"] = "stable"
    manifest["packages"][0]["version"] = version
    for binary in manifest["packages"][0]["binaries"]:
        binary["version"] = version
    if legacy and damage != "profile_domain":
        manifest.pop("runtime")
        manifest["profiles"] = json.loads(PUBLIC.read_text())["profiles"]
    home = tmp_path / "home"
    assets = home / "assets"
    assets.mkdir(parents=True)
    selected = json.dumps(manifest).encode()
    (assets / "manifest.json").write_bytes(selected)
    url = "https://source-fixture.invalid/assets/stable/manifest.json"
    metadata = {
        "schema": verify.METADATA_SCHEMA, "manifest_url": url, "channel": "stable",
        "package_version": version, "validation_status": "valid", "validation_error": None,
        "checked_url": url, "channel_locked": True, "update_available": False,
        "installed_at": 1, "refreshed_at": 2, "checked_at": 3,
    }
    if damage == "metadata_validation":
        metadata["validation_status"] = "invalid"
    if damage == "metadata_source":
        metadata["manifest_url"] = "https://foreign.invalid/manifest.json"
    (assets / "manifest-metadata.json").write_text(json.dumps(metadata))
    status = (f"Installed: true\nRunning:   true\nService:   ok\nGateway:   ok\n"
              f"{status_line}\nManifest source: {url}\nVersion:   {version}\n")
    if damage == "service":
        status = status.replace("Service:   ok", "Service:   failed")
    elif damage == "gateway":
        status = status.replace("Gateway:   ok", "Gateway:   failed")
    elif damage == "status_source":
        status = status.replace(f"Manifest source: {url}", "Manifest source: foreign")
    elif damage == "version":
        status = status.replace(f"Version:   {version}", "Version:   0.0.0")
    capsem = tmp_path / "capsem"
    capsem.write_text("""#!/usr/bin/env python3
from pathlib import Path
import os
import sys
if sys.argv[1] == 'status':
    print(Path(__file__).with_name('status').read_text(), end='')
else:
    failed = next((Path(os.environ['CAPSEM_RUN_DIR']) / 'sessions').glob(f'{sys.argv[2]}-failed-*'))
    print((failed / 'process.log').read_text(), end='')
""")
    capsem.chmod(0o755)
    (tmp_path / "status").write_text(status)
    monkeypatch.setattr(verify, "fetch_bytes", lambda _url: selected)
    evidence = tmp_path / "verified.json"
    argv = ["verify-installed-release", "--capsem", str(capsem), "--capsem-home", str(home),
            "--manifest-url", url, "--channel", "stable", "--package-version", version,
            "--artifact", str(package), "--platform", "macos", "--architecture", "arm64",
            "--evidence-out", str(evidence)]
    if legacy:
        argv.append("--legacy-profile-predecessor")
    if damage == "artifact":
        package.write_bytes(b"corrupt")
    elif damage == "manifest_bytes":
        (assets / "manifest.json").write_bytes(selected + b" ")
    elif damage == "missing_artifact":
        index = argv.index("--artifact")
        del argv[index:index + 2]
    elif damage in {"platform", "architecture"}:
        argv[argv.index("--" + damage) + 1] = "linux" if damage == "platform" else "amd64"
    monkeypatch.setattr("sys.argv", argv)
    return verify, evidence, crypto.digest_file(PUBLIC)


def test_explicit_original_status_literal_passes_all_remaining_verifier_joins(tmp_path, monkeypatch):
    import json

    verify, evidence, original_fixture_digest = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=True, status_line="Manifest status: valid",
    )
    assert verify.main() == 0
    assert json.loads(evidence.read_text())["service"] == "ok"
    assert original_fixture_digest == "2b04fb53657ee3146c66054c7bd4177248eec6ba21bb0e51f33482ebb4b966c8"


def test_default_modern_status_literal_stays_strict(tmp_path, monkeypatch):
    verify, evidence, _ = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=False, status_line="Manifest status: Valid",
    )
    assert verify.main() == 0
    assert evidence.exists()


@pytest.mark.parametrize("legacy,line", [
    (True, "Manifest status: invalid"), (True, "Manifest status: Valid"),
    (True, "Manifest status: valid-foreign"), (True, "Manifest status: unknown"),
    (False, "Manifest status: valid"), (False, "Manifest status: Invalid"),
    (False, "Manifest status: Valid-foreign"), (False, "Manifest status: unknown"),
    (True, ""), (False, ""),
    (True, "Manifest status: valid\nManifest status: invalid"),
    (False, "Manifest status: Valid\nManifest status: Invalid"),
    (True, "Manifest status: valid\nManifest status: valid"),
])
def test_status_literal_never_accepts_wrong_case_prefix_or_ambiguous_lines(
    tmp_path, monkeypatch, legacy, line,
):
    verify, evidence, _ = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=legacy, status_line=line,
    )
    with pytest.raises(SystemExit, match="Manifest status"):
        verify.main()
    assert not evidence.exists()


@pytest.mark.parametrize("damage", ["metadata_source", "metadata_validation", "service", "gateway", "status_source", "version", "artifact", "manifest_bytes", "missing_artifact", "platform", "architecture", "profile_domain"])
def test_explicit_original_mode_keeps_provenance_health_version_guards(tmp_path, monkeypatch, damage):
    verify, evidence, _ = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=True, status_line="Manifest status: valid", damage=damage,
    )
    expected = ValueError if damage == "profile_domain" else (SystemExit, RuntimeError)
    with pytest.raises(expected):
        verify.main()
    assert not evidence.exists()


def test_manifest_status_failure_exposes_only_fixed_mode_and_count(tmp_path, monkeypatch):
    verify, evidence, _ = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=True, status_line="Manifest status: synthetic-secret-canary",
    )
    with pytest.raises(SystemExit) as refused:
        verify.main()
    message = str(refused.value)
    assert "legacy_predecessor=True, line_count=1" in message
    assert "synthetic-secret-canary" not in message
    assert not evidence.exists()

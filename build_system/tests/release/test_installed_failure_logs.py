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
        "schema": verify.METADATA_SCHEMA, "origin": "package", "manifest_url": url, "channel": "stable",
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
              f"Version:   {version}\n")
    if not legacy:
        status += f"{status_line}\nManifest source: {url}\n"
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
with Path(__file__).with_name('invocations').open('a') as output:
    print(sys.argv[1], file=output)
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
        receipt = json.loads((PUBLIC.parent / "public-stable-066-assets-status.json").read_text())
        receipt["manifest"].update(path=str(assets / "manifest.json"), origin_path=str(assets / "manifest-metadata.json"), origin_source=url, blake3=crypto.blake3(selected).hexdigest())
        if damage == "status_source":
            receipt["manifest"]["origin_source"] = "https://foreign.invalid/manifest.json"
        for row in receipt["assets"]:
            row["path"] = str(assets / "arm64" / row["name"])
        snapshot = tmp_path / "assets-after-install.json"
        snapshot.write_text(json.dumps(receipt))
        argv += ["--legacy-profile-predecessor", "--predecessor-assets-status", str(snapshot), "--predecessor-manifest-blake3", crypto.blake3(selected).hexdigest(), "--predecessor-manifest-sha256", crypto.sha256(selected).hexdigest(), "--predecessor-assets-maximum", "1048576"]
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


def test_original_status_without_manifest_lines_keeps_all_remaining_verifier_joins(tmp_path, monkeypatch):
    import json

    verify, evidence, original_fixture_digest = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=True, status_line="",
    )
    assert verify.main() == 0
    assert json.loads(evidence.read_text())["service"] == "ok"
    status = (tmp_path / "status").read_text()
    assert "Manifest source:" not in status and "Manifest status:" not in status
    assert json.loads(evidence.with_suffix(".diagnostic.json").read_text())["verified"] is True
    assert original_fixture_digest == "2b04fb53657ee3146c66054c7bd4177248eec6ba21bb0e51f33482ebb4b966c8"


def test_default_modern_status_literal_stays_strict(tmp_path, monkeypatch):
    verify, evidence, _ = _installed_status_fixture(
        tmp_path, monkeypatch, legacy=False, status_line="Manifest status: Valid",
    )
    assert verify.main() == 0
    assert evidence.exists()


@pytest.mark.parametrize("legacy,line", [
    (False, "Manifest status: valid"), (False, "Manifest status: Invalid"),
    (False, "Manifest status: Valid-foreign"), (False, "Manifest status: unknown"),
    (False, ""),
    (False, "Manifest status: Valid\nManifest status: Invalid"),
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
        tmp_path, monkeypatch, legacy=True, status_line="", damage=damage,
    )
    expected = ValueError if damage == "profile_domain" else (SystemExit, RuntimeError)
    with pytest.raises(expected):
        verify.main()
    assert not evidence.exists()



@pytest.mark.parametrize("damage", ["missing", "corrupt", "oversize", "manifest", "hash", "path", "origin_path", "source", "origin", "invalid", "validation_error", "notready", "downloading", "error", "reconcile_error", "missingasset", "duplicateasset", "foreignasset", "assetpath", "assetstatus", "sha", "expectedhash"])
def test_original_assets_json_fails_closed_without_reexecution(tmp_path, monkeypatch, damage, capsys):
    import json
    import sys

    verify, evidence, _ = _installed_status_fixture(tmp_path, monkeypatch, legacy=True, status_line="")
    path = tmp_path / "assets-after-install.json"
    value = json.loads(path.read_text())
    canary = "unexpected-secret-canary"
    if damage == "missing":
        path.unlink()
    elif damage == "corrupt":
        path.write_text("{unclosed" + canary)
    elif damage == "oversize":
        path.write_bytes(b" " * 1048577)
    elif damage == "manifest":
        del value["manifest"]
    elif damage in {"hash", "path", "origin_path", "source", "origin", "invalid", "validation_error"}:
        field = {"hash": "blake3", "source": "origin_source", "invalid": "validation_status"}.get(damage, damage)
        value["manifest"][field] = canary
    elif damage in {"notready", "downloading", "error", "reconcile_error"}:
        value[{"notready": "ready"}.get(damage, damage)] = damage != "notready"
    elif damage == "missingasset":
        value["assets"].pop()
    elif damage == "duplicateasset":
        value["assets"].append(value["assets"][0])
    elif damage == "foreignasset":
        value["assets"][0]["name"] = canary
    elif damage == "assetpath":
        value["assets"][0]["path"] += "/../foreign"
    elif damage == "assetstatus":
        value["assets"][0]["status"] = "missing"
    else:
        option = "--predecessor-manifest-sha256" if damage == "sha" else "--predecessor-manifest-blake3"
        sys.argv[sys.argv.index(option) + 1] = "0" * 64
    if damage not in {"missing", "corrupt", "oversize"}:
        path.write_text(json.dumps(value))
    with pytest.raises(SystemExit) as refused:
        verify.main()
    assert canary not in str(refused.value)
    assert not evidence.exists()
    assert (tmp_path / "invocations").read_text().splitlines() == ["status"]
    diagnostic = json.loads(evidence.with_suffix(".diagnostic.json").read_text())
    assert diagnostic["command"] == ["assets", "status", "--json"]
    assert diagnostic["producer"] == "same_fixture_install_hydration"
    assert canary not in json.dumps(diagnostic) + capsys.readouterr().err


def test_complete_staged_original_assets_reader_is_stdlib_only_on_guest_python(tmp_path, monkeypatch):
    import json
    import subprocess
    import sys

    from build_system.packaging.macos.macos_candidate_content import stage_guest_scripts

    _verify, evidence, _ = _installed_status_fixture(tmp_path, monkeypatch, legacy=True, status_line="")
    root = Path(__file__).resolve().parents[3]
    share = tmp_path / "share"
    stage_guest_scripts(root, share)
    argv = tmp_path / "argv.json"
    argv.write_text(json.dumps([*sys.argv, "--capsem-home", str(tmp_path / "home")]))
    code = """
import importlib.util
import json
from pathlib import Path
import sys
assert importlib.util.find_spec('blake3') is None
from capsem_builder.release.tools import verify_installed_release as verify
sys.argv = json.loads(Path(sys.argv[1]).read_text())
verify.fetch_bytes = lambda _url: (Path(sys.argv[sys.argv.index('--capsem-home') + 1]) / 'assets' / 'manifest.json').read_bytes()
raise SystemExit(verify.main())
"""
    result = subprocess.run(["/usr/bin/python3", "-S", "-c", code, str(argv)],
                            cwd=share, env={"PATH": "/usr/bin:/bin"}, capture_output=True,
                            text=True, timeout=15)
    assert result.returncode == 0, result.stderr
    assert json.loads(evidence.read_text())["package_version"] == "0.6.6"


def test_failed_status_keeps_only_safe_diagnostics_and_never_reexecutes(tmp_path, monkeypatch):
    import json

    verify, evidence, _ = _installed_status_fixture(tmp_path, monkeypatch, legacy=True, status_line="")
    command = tmp_path / "capsem"
    command.write_text(command.read_text().replace("print(Path(__file__).with_name('status').read_text(), end='')", "print('synthetic-secret-canary'); sys.exit(23)"))
    with pytest.raises(SystemExit, match="capsem status exited 23") as refused:
        verify.main()
    assert "synthetic-secret-canary" not in str(refused.value)
    assert (tmp_path / "invocations").read_text().splitlines() == ["status"]
    summary = evidence.with_suffix(".status-diagnostic.json").read_text()
    assert "synthetic-secret-canary" not in summary and json.loads(summary)["exit"] == 23
    assert not evidence.exists()

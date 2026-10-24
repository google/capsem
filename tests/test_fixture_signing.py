"""Fixture consumers must not rewrite the artifact they are qualifying."""

import hashlib
import os
import plistlib
import shutil
import subprocess
from pathlib import Path

import pytest
from helpers import sign


@pytest.fixture
def binary(tmp_path, monkeypatch):
    directory = tmp_path / "selected"
    directory.mkdir()
    monkeypatch.setenv("CAPSEM_RELEASE_BIN_DIR", str(directory))
    monkeypatch.delenv("CAPSEM_RELEASE_INPUT_DIR", raising=False)
    monkeypatch.delenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", raising=False)
    path = directory / "capsem-tray"
    path.write_bytes(b"original signed package bytes")
    path.chmod(0o755)
    return path


def codesign(monkeypatch, *, valid=True, entitlements=""):
    calls = []

    def run(argv, **kwargs):
        calls.append(argv)
        if "--sign" in argv:
            Path(argv[-1]).write_bytes(b"rewritten by fixture")
        return subprocess.CompletedProcess(
            argv, 0 if valid or "--sign" in argv else 1,
            stdout=entitlements if "--entitlements" in argv and "--sign" not in argv else "",
            stderr="invalid signature" if not valid else "",
        )

    monkeypatch.setattr(sign, "IS_MACOS", True)
    monkeypatch.setattr(sign.subprocess, "run", run)
    return calls


@pytest.mark.parametrize("kind", ["cohort", "release"])
def test_selected_artifacts_are_read_only_consumers(binary, monkeypatch, kind):
    calls = codesign(monkeypatch)
    verified = []
    if kind == "cohort":
        monkeypatch.setenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", "original-cohort.json")
        monkeypatch.setattr(sign, "verify_frozen_binary", lambda *args: verified.append(args), raising=False)
    else:
        monkeypatch.setenv("CAPSEM_RELEASE_INPUT_DIR", "original-release-inputs")
    before = binary.read_bytes()
    identity = binary.stat()
    sign.sign_binary(binary)
    assert binary.read_bytes() == before
    assert (binary.stat().st_ino, binary.stat().st_size) == (identity.st_ino, identity.st_size)
    assert not binary.with_suffix(".sign.lock").exists()
    assert not any("--sign" in call for call in calls)
    if kind == "cohort":
        assert verified == [(binary, Path("original-cohort.json"), sign.PROJECT_ROOT)]


@pytest.mark.parametrize("kind", ["cohort", "release"])
def test_invalid_selected_signature_fails_without_repair(binary, monkeypatch, kind):
    calls = codesign(monkeypatch, valid=False)
    monkeypatch.setenv(
        "CAPSEM_LOCAL_TEST_BINARY_COHORT" if kind == "cohort" else "CAPSEM_RELEASE_INPUT_DIR",
        "original-inputs",
    )
    monkeypatch.setattr(sign, "verify_frozen_binary", lambda *args: None, raising=False)
    before = binary.read_bytes()
    with pytest.raises(RuntimeError, match="signature"):
        sign.sign_binary(binary)
    assert binary.read_bytes() == before
    assert not any("--sign" in call for call in calls)
    assert not binary.with_suffix(".sign.lock").exists()


def test_foreign_cohort_provenance_fails_before_signature(binary, monkeypatch):
    calls = codesign(monkeypatch)
    monkeypatch.setenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", "foreign.json")

    def refuse(*args):
        raise RuntimeError("foreign source")

    monkeypatch.setattr(sign, "verify_frozen_binary", refuse, raising=False)
    with pytest.raises(RuntimeError, match="foreign source"):
        sign.sign_binary(binary)
    assert calls == []
    assert not binary.with_suffix(".sign.lock").exists()


def test_linux_still_checks_frozen_source_bytes(binary, monkeypatch):
    monkeypatch.setattr(sign, "IS_MACOS", False)
    monkeypatch.setenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", "original.json")
    verified = []
    monkeypatch.setattr(sign, "verify_frozen_binary", lambda *args: verified.append(args), raising=False)
    sign.sign_binary(binary)
    assert verified == [(binary, Path("original.json"), sign.PROJECT_ROOT)]


@pytest.mark.parametrize("name,required", [("capsem-tray", False), ("capsem-service", True)])
def test_developer_signing_uses_configured_role_entitlements(binary, monkeypatch, name, required):
    target = binary.with_name(name)
    binary.rename(target)
    calls = codesign(monkeypatch, valid=False)
    sign.sign_binary(target)
    (mutation,) = [call for call in calls if "--sign" in call]
    assert ("--entitlements" in mutation) is required


def test_temporary_copied_fixture_gets_no_vm_privilege(binary, tmp_path, monkeypatch):
    other = tmp_path / "capsem-service"
    other.write_bytes(b"temporary copied sleep fixture")
    calls = codesign(monkeypatch, valid=False)
    sign.sign_binary(other)
    (mutation,) = [call for call in calls if "--sign" in call]
    assert "--entitlements" not in mutation


@pytest.mark.parametrize("value", [False, 1, "true"])
def test_vm_entitlement_must_be_true_not_merely_mentioned(binary, monkeypatch, value):
    target = binary.with_name("capsem-service")
    binary.rename(target)
    settings = sign.load(sign.PROJECT_ROOT)
    expected = plistlib.loads(settings.path(settings.signing.entitlements).read_bytes())
    expected["com.apple.security.virtualization"] = value
    codesign(monkeypatch, entitlements=plistlib.dumps(expected).decode())
    assert not sign.verify_signed(target)


def test_vm_role_accepts_actual_configured_entitlements_without_resigning(binary, monkeypatch):
    target = binary.with_name("capsem-service")
    binary.rename(target)
    settings = sign.load(sign.PROJECT_ROOT)
    calls = codesign(
        monkeypatch, entitlements=settings.path(settings.signing.entitlements).read_text()
    )
    monkeypatch.setenv("CAPSEM_RELEASE_INPUT_DIR", "original-release-inputs")
    before = target.stat()
    sign.sign_binary(target)
    assert target.stat() == before
    assert not any("--sign" in call for call in calls)
    assert not target.with_suffix(".sign.lock").exists()


def test_selected_refusal_does_not_truncate_existing_sign_lock(binary, monkeypatch):
    codesign(monkeypatch, valid=False)
    monkeypatch.setenv("CAPSEM_RELEASE_INPUT_DIR", "original-release-inputs")
    lock = binary.with_suffix(".sign.lock")
    lock.write_bytes(b"existing lock contents")
    before = lock.stat()
    with pytest.raises(RuntimeError, match="signature"):
        sign.sign_binary(binary)
    assert lock.read_bytes() == b"existing lock contents"
    assert lock.stat() == before


def test_cohort_and_release_authorities_cannot_mix(binary, monkeypatch):
    calls = codesign(monkeypatch)
    monkeypatch.setenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", "original.json")
    monkeypatch.setenv("CAPSEM_RELEASE_INPUT_DIR", "original-release-inputs")
    with pytest.raises(RuntimeError, match="mixed"):
        sign.sign_binary(binary)
    assert calls == []
    assert not binary.with_suffix(".sign.lock").exists()


def test_malformed_vm_entitlement_receipt_is_not_signed(binary, monkeypatch):
    target = binary.with_name("capsem-service")
    binary.rename(target)
    codesign(monkeypatch, entitlements="not a plist")
    assert not sign.verify_signed(target)


@pytest.mark.skipif(os.uname().sysname != "Darwin", reason="real Apple codesign boundary")
def test_real_valid_non_vm_frozen_binary_keeps_original_bytes(binary, monkeypatch):
    shutil.copyfile("/usr/bin/true", binary)
    binary.chmod(0o755)
    subprocess.run(["codesign", "--sign", "-", "--force", str(binary)], check=True, capture_output=True)
    subprocess.run(["codesign", "--verify", "--strict", str(binary)], check=True, capture_output=True)
    before = hashlib.sha256(binary.read_bytes()).hexdigest()
    monkeypatch.setenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", "original.json")

    def validate_original(path, *args):
        assert hashlib.sha256(path.read_bytes()).hexdigest() == before

    monkeypatch.setattr(sign, "verify_frozen_binary", validate_original, raising=False)
    sign.sign_binary(binary)
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == before
    assert not binary.with_suffix(".sign.lock").exists()

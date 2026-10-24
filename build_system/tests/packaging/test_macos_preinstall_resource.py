"""Source-only Mac resource/privilege doubles; no package or candidate execution."""

import shlex
import subprocess
from dataclasses import replace
from pathlib import Path

import pytest
from capsem_builder import crypto
from capsem_builder.release.tools import preinstall_resource as resource

ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def staged(tmp_path: Path) -> tuple[Path, Path]:
    payload, scripts = tmp_path / "payload", tmp_path / "scripts"
    binary = payload / "usr/local/share/capsem/bin/capsem-ledger"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"original-signed-candidate-format-double")
    binary.chmod(0o755)
    scripts.mkdir()
    (scripts / "preinstall").write_bytes(b"#!/bin/bash\necho original-body\n")
    return payload, scripts


def test_mac_resource_keeps_exact_candidate_and_binds_identity(staged: tuple[Path, Path]) -> None:
    payload, scripts = staged
    binary = (payload / "usr/local/share/capsem/bin/capsem-ledger").read_bytes()
    resource.stage_macos(payload, scripts=scripts, limits=resource.load_limits(ROOT))
    assert (scripts / "capsem-ledger").read_bytes() == binary
    assert (scripts / "capsem-ledger").stat().st_mode & 0o777 == 0o555
    generated = (scripts / "preinstall").read_bytes()
    assert generated.endswith(b"echo original-body\n")
    assert crypto.sha256(binary).hexdigest().encode() in generated
    assert b'/usr/bin/codesign --verify --strict "$resource"' in generated
    assert b'capsem_admit_preinstall_ledger "$1"' in generated
    assert b"base64" not in generated


# Seatbelt silently strips set-id bits on chmod. Concrete modes are exercised
# through the shared production rule in test_preinstall_resource.py.
@pytest.mark.parametrize("bad", ["symlink", "shared", "nonexec", "oversize"])
def test_mac_invalid_resource_preserves_original_script(staged: tuple[Path, Path], bad: str) -> None:
    payload, scripts = staged
    binary = payload / "usr/local/share/capsem/bin/capsem-ledger"
    limits = resource.load_limits(ROOT)
    if bad == "symlink":
        original = binary.with_name("foreign")
        binary.rename(original)
        binary.symlink_to(original)
    elif bad == "shared":
        binary.with_name("shared").hardlink_to(binary)
    elif bad == "nonexec":
        binary.chmod(0o644)
    else:
        limits = replace(limits, max_binary_bytes=1)
    with pytest.raises(ValueError):
        resource.stage_macos(payload, scripts=scripts, limits=limits)
    assert (scripts / "preinstall").read_bytes() == b"#!/bin/bash\necho original-body\n"


def test_mac_admission_precedes_every_old_install_mutation() -> None:
    preinstall = (ROOT / "build_system/packaging/macos/pkg-scripts/preinstall").read_text()
    admission = preinstall.index('capsem_run_preinstall_ledger_admission "$USER"')
    for operation in ('CAPSEM_INSTALL_PHASE="stop_existing_install"', "capsem_stop_launch_agent", "capsem_retire_native_cohort", "rm -rf /Applications", "rm -rf /usr/local/share"):
        assert admission < preinstall.index(operation)
    builder = (ROOT / "build_system/packaging/macos/build-pkg.sh").read_text()
    assert 'codesign --verify --strict "$SHARE_DIR/bin/capsem-ledger"' in builder
    assert "preinstall_resource macos" in builder


@pytest.mark.parametrize("signature_status", [0, 23])
def test_generated_mac_binding_checks_signature_before_shared_stage(
    staged: tuple[Path, Path], signature_status: int,
) -> None:
    payload, scripts = staged
    resource.stage_macos(payload, scripts=scripts, limits=resource.load_limits(ROOT))
    script = scripts / "preinstall"
    script.write_text(script.read_text() + '\ncapsem_run_preinstall_ledger_admission fixture-user\n')
    output = scripts / "observed"
    result = subprocess.run(
        ["bash", "-c", f'''
function /usr/bin/codesign {{
    printf '%s\\n' "$@" > {shlex.quote(str(scripts / 'signature-argv'))}
    return {signature_status}
}}
capsem_admit_preinstall_ledger() {{
    capsem_write_preinstall_resource > {shlex.quote(str(output))}
}}
source "$1"
''', str(script), str(script)], capture_output=True, text=True, timeout=10,
    )
    assert result.returncode == (0 if signature_status == 0 else 1), result.stderr
    assert (scripts / "signature-argv").read_text().splitlines() == [
        "--verify", "--strict", str(scripts / "capsem-ledger"),
    ]
    if signature_status:
        assert not output.exists()
    else:
        assert output.read_bytes() == (scripts / "capsem-ledger").read_bytes()


def test_mac_failed_script_publication_discards_only_own_resource(
    staged: tuple[Path, Path], monkeypatch: pytest.MonkeyPatch,
) -> None:
    payload, scripts = staged
    original = (scripts / "preinstall").read_bytes()

    def fail_replace(*_args: object) -> None:
        raise OSError("source publication fault")

    monkeypatch.setattr(resource.os, "replace", fail_replace)
    with pytest.raises(OSError, match="source publication fault"):
        resource.stage_macos(payload, scripts=scripts, limits=resource.load_limits(ROOT))
    assert (scripts / "preinstall").read_bytes() == original
    assert list(scripts.iterdir()) == [scripts / "preinstall"]


def test_mac_generated_script_budget_refuses_before_publication(staged: tuple[Path, Path]) -> None:
    payload, scripts = staged
    original = (scripts / "preinstall").read_bytes()
    limits = replace(resource.load_limits(ROOT), max_script_bytes=100)
    with pytest.raises(ValueError, match="byte budget"):
        resource.stage_macos(payload, scripts=scripts, limits=limits)
    assert (scripts / "preinstall").read_bytes() == original
    assert not (scripts / "capsem-ledger").exists()

"""NSS database CA trust unit tests."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import Any

import pytest
from inspect_capsem.containers.dockerfile import _CA_NSS_SCRIPT, _IMAGE_CA


def _fake_tool(bin_dir: Path, name: str, log: Path) -> None:
    tool = bin_dir / name
    tool.write_text(f'#!/bin/sh\necho "{name} $*" >> {log}\n')
    tool.chmod(0o755)


@pytest.mark.parametrize("tool", ["certutil", "python3"])
def test_nss_step_uses_certutil_or_ctypes_helper(tmp_path: Path, tool: str) -> None:
    bin_dir, log, home = tmp_path / "bin", tmp_path / "log", tmp_path / "home"
    bin_dir.mkdir()
    home.mkdir()
    _fake_tool(bin_dir, tool, log)
    # Only the fake tool plus the basics the script needs are on PATH.
    for name in ("sh", "mkdir", "awk", "id"):
        real = shutil.which(name)
        assert real is not None
        (bin_dir / name).symlink_to(real)
    sh = shutil.which("sh")
    assert sh is not None
    env = {"PATH": str(bin_dir), "HOME": str(home)}
    subprocess.run([sh, "-c", _CA_NSS_SCRIPT], env=env, check=True)
    calls = log.read_text()
    if tool == "certutil":
        assert calls.splitlines()[-1] == (
            f"certutil -d sql:{home}/.pki/nssdb -A -n capsem-ca -t C,, -i {_IMAGE_CA}"
        )
    else:
        # First the libnss3 probe, then the ctypes helper (multi-line source) on the CA.
        assert "find_library" in calls and "NSS_InitReadWrite" in calls
        assert calls.rstrip().endswith(_IMAGE_CA)


def test_nss_trust_direct_unit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    import inspect_capsem.containers.nss_trust as nss_mod

    monkeypatch.setenv("HOME", str(tmp_path))
    pem = tmp_path / "ca.pem"
    pem.write_text("-----BEGIN CERTIFICATE-----\nAQIDBA==\n-----END CERTIFICATE-----\n")

    class FakeFn:
        def __init__(self, ret: int = 0) -> None:
            self.ret = ret
            self.calls: list[tuple[Any, ...]] = []
            self.restype: Any = None
            self.argtypes: Any = None

        def __call__(self, *args: Any) -> int:
            self.calls.append(args)
            return self.ret

    class FakeNSS:
        def __init__(
            self, *, init_rc: int = 0, need_pin: int = 1, cert_ptr: int = 99, import_rc: int = 0
        ) -> None:
            self.NSS_InitReadWrite = FakeFn(init_rc)
            self.PK11_GetInternalKeySlot = FakeFn(7)
            self.PK11_NeedUserInit = FakeFn(need_pin)
            self.PK11_InitPin = FakeFn(0)
            self.CERT_GetDefaultCertDB = FakeFn(11)
            self.CERT_NewTempCertificate = FakeFn(cert_ptr)
            self.PK11_ImportCert = FakeFn(import_rc)
            self.CERT_DecodeTrustString = FakeFn(0)
            self.CERT_ChangeCertTrust = FakeFn(0)
            self.CERT_DestroyCertificate = FakeFn(0)
            self.PK11_FreeSlot = FakeFn(0)
            self.NSS_Shutdown = FakeFn(0)

    fake = FakeNSS(need_pin=1)
    monkeypatch.setattr(nss_mod.ctypes.util, "find_library", lambda _: "libnss3.so")
    monkeypatch.setattr(nss_mod.ctypes, "CDLL", lambda _: fake)
    nss_mod.trust_ca(str(pem))
    assert len(fake.PK11_InitPin.calls) == 1
    assert len(fake.NSS_Shutdown.calls) == 1

    fake_nopin = FakeNSS(need_pin=0)
    monkeypatch.setattr(nss_mod.ctypes, "CDLL", lambda _: fake_nopin)
    nss_mod.trust_ca(str(pem))
    assert len(fake_nopin.PK11_InitPin.calls) == 0

    monkeypatch.setattr(nss_mod.ctypes, "CDLL", lambda _: FakeNSS(init_rc=1))
    with pytest.raises(SystemExit, match="cannot open NSS DB"):
        nss_mod.trust_ca(str(pem))

    monkeypatch.setattr(nss_mod.ctypes, "CDLL", lambda _: FakeNSS(cert_ptr=0))
    with pytest.raises(SystemExit, match="cannot decode the CA certificate"):
        nss_mod.trust_ca(str(pem))

    monkeypatch.setattr(nss_mod.ctypes, "CDLL", lambda _: FakeNSS(import_rc=1))
    with pytest.raises(SystemExit, match="cannot trust the CA in the NSS DB"):
        nss_mod.trust_ca(str(pem))

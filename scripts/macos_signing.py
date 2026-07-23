#!/usr/bin/env python3
"""Ephemeral Apple signing identity setup for the local release proof."""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import tempfile


APPLICATION_IDENTITY = "Developer ID Application: Elie Bursztein (L8EGK4X86T)"
INSTALLER_IDENTITY = "Developer ID Installer: Elie Bursztein (L8EGK4X86T)"


class SigningError(RuntimeError):
    """The local signing material could not form usable Apple identities."""


def _run(
    command: list[str],
    *,
    capture_output: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        check=True,
        capture_output=capture_output,
        text=True,
    )


def _current_user_keychains() -> list[str]:
    result = _run(
        ["security", "list-keychains", "-d", "user"],
        capture_output=True,
    )
    return shlex.split(result.stdout)


def _require_identity(keychain: Path, identity: str, policy: str | None) -> None:
    command = ["security", "find-identity", "-v"]
    if policy is not None:
        command.extend(["-p", policy])
    command.append(str(keychain))
    result = _run(command, capture_output=True)
    if identity not in result.stdout:
        raise SigningError(f"ephemeral keychain is missing {identity}")


@contextmanager
def ephemeral_signing_environment(
    certificate_dir: Path,
) -> Iterator[dict[str, str]]:
    """Yield signing environment variables, then restore the user keychain list."""

    certificate_dir = certificate_dir.resolve()
    required = {
        "application PKCS#12": certificate_dir / "capsem.p12",
        "installer certificate": certificate_dir / "developer.cer",
        "shared private key": certificate_dir / "capsem.key",
        "PKCS#12 password": certificate_dir / "p12-password.txt",
    }
    missing = [f"{label}: {path}" for label, path in required.items() if not path.is_file()]
    if missing:
        raise SigningError("missing local Apple signing material: " + ", ".join(missing))

    original_keychains = _current_user_keychains()
    keychain_password = secrets.token_urlsafe(32)
    p12_password = required["PKCS#12 password"].read_text(encoding="utf-8").strip()
    if not p12_password:
        raise SigningError("local Apple PKCS#12 password is empty")

    with tempfile.TemporaryDirectory(prefix="capsem-signing-") as temporary:
        temporary_dir = Path(temporary)
        keychain = temporary_dir / "capsem-release.keychain-db"
        installer_pem = temporary_dir / "installer.pem"
        installer_p12 = temporary_dir / "installer.p12"
        created = False
        search_list_changed = False
        try:
            _run(["security", "create-keychain", "-p", keychain_password, str(keychain)])
            created = True
            _run(
                [
                    "security",
                    "set-keychain-settings",
                    "-lut",
                    "21600",
                    str(keychain),
                ]
            )
            _run(
                [
                    "security",
                    "unlock-keychain",
                    "-p",
                    keychain_password,
                    str(keychain),
                ]
            )
            _run(
                [
                    "openssl",
                    "x509",
                    "-inform",
                    "DER",
                    "-in",
                    str(required["installer certificate"]),
                    "-out",
                    str(installer_pem),
                ]
            )
            _run(
                [
                    "openssl",
                    "pkcs12",
                    "-export",
                    "-inkey",
                    str(required["shared private key"]),
                    "-in",
                    str(installer_pem),
                    "-out",
                    str(installer_p12),
                    "-passout",
                    f"file:{required['PKCS#12 password']}",
                ]
            )
            _run(
                [
                    "security",
                    "import",
                    str(required["application PKCS#12"]),
                    "-k",
                    str(keychain),
                    "-P",
                    p12_password,
                    "-A",
                ]
            )
            _run(
                [
                    "security",
                    "import",
                    str(installer_p12),
                    "-k",
                    str(keychain),
                    "-P",
                    p12_password,
                    "-A",
                ]
            )
            _run(
                [
                    "security",
                    "set-key-partition-list",
                    "-S",
                    "apple-tool:,apple:",
                    "-s",
                    "-k",
                    keychain_password,
                    str(keychain),
                ]
            )
            _run(
                [
                    "security",
                    "list-keychains",
                    "-d",
                    "user",
                    "-s",
                    str(keychain),
                    *original_keychains,
                ]
            )
            search_list_changed = True
            _require_identity(keychain, APPLICATION_IDENTITY, "codesigning")
            _require_identity(keychain, INSTALLER_IDENTITY, None)
            yield {
                **os.environ,
                "APPLE_SIGNING_IDENTITY": APPLICATION_IDENTITY,
                "CAPSEM_INSTALLER_SIGNING_IDENTITY": INSTALLER_IDENTITY,
            }
        finally:
            if search_list_changed:
                _run(
                    [
                        "security",
                        "list-keychains",
                        "-d",
                        "user",
                        "-s",
                        *original_keychains,
                    ]
                )
            if created:
                subprocess.run(
                    ["security", "delete-keychain", str(keychain)],
                    check=False,
                    capture_output=True,
                    text=True,
                )

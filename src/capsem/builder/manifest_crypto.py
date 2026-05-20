"""Minisign-backed manifest crypto helpers for capsem-admin."""

from __future__ import annotations

from pathlib import Path
import shutil
import subprocess
from typing import Callable, Literal

from pydantic import BaseModel, ConfigDict, Field


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class MinisignResult(StrictModel):
    ok: bool
    command: list[str]
    returncode: int
    stdout: str = ""
    stderr: str = ""
    failure: Literal["tool_missing", "verification_failed", "sign_failed"] | None = None


class ManifestSignReport(StrictModel):
    schema_: Literal["capsem.manifest-sign.v1"] = Field(
        default="capsem.manifest-sign.v1",
        alias="schema",
    )
    ok: bool
    manifest_path: str
    signature_path: str
    key_path: str
    command: list[str]


class ManifestSignatureVerificationReport(StrictModel):
    schema_: Literal["capsem.manifest-signature-verification.v1"] = Field(
        default="capsem.manifest-signature-verification.v1",
        alias="schema",
    )
    ok: bool
    manifest_path: str
    signature_path: str
    pubkey_path: str
    command: list[str]
    failure: Literal["tool_missing", "verification_failed"] | None = None
    message: str | None = None


MinisignRunner = Callable[..., subprocess.CompletedProcess[str]]


def _minisign_missing(args: list[str], failure: Literal["tool_missing"]) -> MinisignResult:
    return MinisignResult(
        ok=False,
        command=args,
        returncode=127,
        failure=failure,
        stderr="minisign not found on PATH",
    )


def verify_minisign_signature(
    payload_path: Path,
    signature_path: Path,
    pubkey_path: Path,
    *,
    runner: MinisignRunner = subprocess.run,
) -> MinisignResult:
    args = [
        "minisign",
        "-Vm",
        str(payload_path),
        "-x",
        str(signature_path),
        "-p",
        str(pubkey_path),
    ]
    if shutil.which("minisign") is None:
        return _minisign_missing(args, "tool_missing")

    result = runner(args, capture_output=True, text=True, check=False)
    return MinisignResult(
        ok=result.returncode == 0,
        command=args,
        returncode=result.returncode,
        stdout=result.stdout,
        stderr=result.stderr,
        failure=None if result.returncode == 0 else "verification_failed",
    )


def sign_manifest(
    manifest_path: Path,
    key_path: Path,
    *,
    signature_path: Path | None = None,
    password_file: Path | None = None,
    runner: MinisignRunner = subprocess.run,
) -> ManifestSignReport:
    resolved_signature_path = signature_path or manifest_path.with_name(
        manifest_path.name + ".minisig"
    )
    args = [
        "minisign",
        "-S",
        "-s",
        str(key_path),
        "-m",
        str(manifest_path),
        "-x",
        str(resolved_signature_path),
    ]
    if shutil.which("minisign") is None:
        raise RuntimeError("minisign not found on PATH")

    stdin_text = None
    if password_file is not None:
        stdin_text = password_file.read_text(encoding="utf-8")
        if not stdin_text.endswith("\n"):
            stdin_text += "\n"

    result = runner(
        args,
        input=stdin_text,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "minisign failed"
        raise RuntimeError(f"manifest signing failed: {detail}")

    return ManifestSignReport(
        ok=True,
        manifest_path=str(manifest_path),
        signature_path=str(resolved_signature_path),
        key_path=str(key_path),
        command=args,
    )


def verify_manifest_signature(
    manifest_path: Path,
    signature_path: Path,
    pubkey_path: Path,
    *,
    runner: MinisignRunner = subprocess.run,
) -> ManifestSignatureVerificationReport:
    result = verify_minisign_signature(
        manifest_path,
        signature_path,
        pubkey_path,
        runner=runner,
    )
    return ManifestSignatureVerificationReport(
        ok=result.ok,
        manifest_path=str(manifest_path),
        signature_path=str(signature_path),
        pubkey_path=str(pubkey_path),
        command=result.command,
        failure=result.failure,
        message=(result.stderr or result.stdout or None),
    )


def dump_manifest_sign_report_json(report: ManifestSignReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_manifest_signature_verification_report_json(
    report: ManifestSignatureVerificationReport,
) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)

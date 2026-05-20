#!/usr/bin/env python3
"""Verify Capsem `.deb` package payloads without extracting them to disk."""

from __future__ import annotations

import argparse
import gzip
import lzma
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from typing import Optional


REQUIRED_PAYLOADS = (
    "usr/bin/capsem",
    "usr/bin/capsem-service",
    "usr/bin/capsem-process",
    "usr/bin/capsem-mcp",
    "usr/bin/capsem-mcp-aggregator",
    "usr/bin/capsem-mcp-builtin",
    "usr/bin/capsem-gateway",
    "usr/bin/capsem-tray",
    "usr/bin/capsem-admin",
    "usr/share/capsem/admin-python/capsem/admin/cli.py",
    "usr/share/capsem/assets/manifest.json",
    "usr/share/capsem/assets/manifest.json.minisig",
)


class VerificationError(RuntimeError):
    """A package failed verification."""


@dataclass(frozen=True)
class TarPayload:
    name: str
    data: bytes


def _normalize_tar_name(name: str) -> str:
    return name.removeprefix("./").lstrip("/")


def _read_ar_members(path: Path) -> dict[str, bytes]:
    data = path.read_bytes()
    if not data.startswith(b"!<arch>\n"):
        raise VerificationError(f"{path}: not an ar/deb archive")

    offset = 8
    members: dict[str, bytes] = {}
    while offset < len(data):
        header = data[offset:offset + 60]
        if len(header) != 60:
            raise VerificationError(f"{path}: truncated ar member header")
        if header[58:60] != b"`\n":
            raise VerificationError(f"{path}: invalid ar member header")

        raw_name = header[0:16].decode("utf-8", errors="replace").strip()
        name = raw_name.rstrip("/")
        size_text = header[48:58].decode("ascii", errors="replace").strip()
        try:
            size = int(size_text)
        except ValueError as exc:
            raise VerificationError(f"{path}: invalid ar member size {size_text!r}") from exc

        start = offset + 60
        end = start + size
        members[name] = data[start:end]
        offset = end + (size % 2)

    return members


def _decompress(name: str, payload: bytes) -> bytes:
    if name.endswith(".gz"):
        return gzip.decompress(payload)
    if name.endswith(".xz"):
        return lzma.decompress(payload)
    if name.endswith(".zst"):
        try:
            import zstandard  # type: ignore[import-not-found]
        except ModuleNotFoundError:
            if shutil.which("zstd") is None:
                raise VerificationError(
                    "zstd payload requires either the Python 'zstandard' package "
                    "or the 'zstd' command on PATH"
                )
            result = subprocess.run(
                ["zstd", "-dc"],
                input=payload,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                stderr = result.stderr.decode("utf-8", errors="replace")
                raise VerificationError(f"zstd failed to decompress {name}: {stderr}")
            return result.stdout
        with zstandard.ZstdDecompressor().stream_reader(BytesIO(payload)) as reader:
            return reader.read()
    return payload


def _find_tar(members: dict[str, bytes], prefix: str) -> TarPayload:
    for name, payload in members.items():
        if name.startswith(prefix + ".tar"):
            return TarPayload(name=name, data=_decompress(name, payload))
    raise VerificationError(f"missing {prefix}.tar.* member")


def _tar_names(payload: TarPayload) -> set[str]:
    with tarfile.open(fileobj=BytesIO(payload.data), mode="r:") as tar:
        return {_normalize_tar_name(member.name) for member in tar.getmembers()}


def _read_tar_file(payload: TarPayload, wanted: str) -> bytes:
    with tarfile.open(fileobj=BytesIO(payload.data), mode="r:") as tar:
        for member in tar.getmembers():
            if _normalize_tar_name(member.name) == wanted:
                extracted = tar.extractfile(member)
                if extracted is None:
                    raise VerificationError(f"{wanted} is not a regular file")
                return extracted.read()
    raise VerificationError(f"missing payload file {wanted}")


def _control_fields(payload: TarPayload) -> dict[str, str]:
    raw = _read_tar_file(payload, "control").decode("utf-8", errors="replace")
    fields: dict[str, str] = {}
    current: Optional[str] = None
    for line in raw.splitlines():
        if not line:
            continue
        if line[0].isspace() and current:
            fields[current] = fields[current] + "\n" + line.strip()
            continue
        key, sep, value = line.partition(":")
        if sep:
            current = key
            fields[key] = value.strip()
    return fields


def _verify_required_payloads(data_payload: TarPayload) -> None:
    names = _tar_names(data_payload)
    missing = [name for name in REQUIRED_PAYLOADS if name not in names]
    if missing:
        raise VerificationError("missing required payload(s): " + ", ".join(missing))


def _verify_minisign(data_payload: TarPayload, pubkey: Path) -> None:
    if shutil.which("minisign") is None:
        raise VerificationError("--minisign-pubkey was provided, but minisign is not on PATH")

    manifest = _read_tar_file(data_payload, "usr/share/capsem/assets/manifest.json")
    signature = _read_tar_file(data_payload, "usr/share/capsem/assets/manifest.json.minisig")

    with tempfile.TemporaryDirectory(prefix="capsem-deb-verify-") as tmp:
        tmp_path = Path(tmp)
        manifest_path = tmp_path / "manifest.json"
        sig_path = tmp_path / "manifest.json.minisig"
        manifest_path.write_bytes(manifest)
        sig_path.write_bytes(signature)
        result = subprocess.run(
            [
                "minisign",
                "-Vm",
                str(manifest_path),
                "-x",
                str(sig_path),
                "-p",
                str(pubkey),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            raise VerificationError(
                "manifest signature verification failed:\n"
                + result.stdout
                + result.stderr
            )


def verify_deb(
    deb: Path,
    *,
    expected_version: Optional[str],
    expected_architecture: Optional[str],
    minisign_pubkey: Optional[Path],
) -> None:
    members = _read_ar_members(deb)
    if members.get("debian-binary", b"").strip() != b"2.0":
        raise VerificationError(f"{deb}: missing or invalid debian-binary member")

    control_payload = _find_tar(members, "control")
    data_payload = _find_tar(members, "data")
    fields = _control_fields(control_payload)

    if fields.get("Package") != "capsem":
        raise VerificationError(f"{deb}: expected Package: capsem, got {fields.get('Package')!r}")
    if expected_version and fields.get("Version") != expected_version:
        raise VerificationError(
            f"{deb}: expected Version: {expected_version}, got {fields.get('Version')!r}"
        )
    if expected_architecture and fields.get("Architecture") != expected_architecture:
        raise VerificationError(
            f"{deb}: expected Architecture: {expected_architecture}, "
            f"got {fields.get('Architecture')!r}"
        )

    _verify_required_payloads(data_payload)
    if minisign_pubkey is not None:
        _verify_minisign(data_payload, minisign_pubkey)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("deb", nargs="+", type=Path, help=".deb package(s) to verify")
    parser.add_argument("--version", help="expected Debian package version")
    parser.add_argument("--architecture", help="expected Debian package architecture")
    parser.add_argument(
        "--minisign-pubkey",
        type=Path,
        help="verify usr/share/capsem/assets/manifest.json.minisig with this public key",
    )
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        for deb in args.deb:
            verify_deb(
                deb,
                expected_version=args.version,
                expected_architecture=args.architecture,
                minisign_pubkey=args.minisign_pubkey,
            )
            print(f"ok {deb}")
    except VerificationError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

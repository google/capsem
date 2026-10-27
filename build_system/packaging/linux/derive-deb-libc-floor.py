#!/usr/bin/env python3
"""Derive the packaged binaries' actual glibc floor as a Debian dependency.

`Capsem_0.6.0_arm64.deb` declared `libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3`
and no `libc6` at all, while every shipped binary needed GLIBC_2.39.  On Debian
bookworm (2.36) and Ubuntu 22.04 (2.35) `apt install` therefore *succeeded* --
each declared dependency was satisfiable -- and then every binary died with
"version `GLIBC_2.39' not found".  A package that installs cleanly and does
nothing is strictly worse than one apt refuses, so the floor is read back out
of the bytes being shipped rather than hand-written next to the GUI libraries.

Versioned symbol references name their version in the ELF's dynamic string
table, so the highest `GLIBC_x.y` a binary can require is recorded in the file
itself.  Scanning for it needs no toolchain, which keeps this usable from
inside the repack step on any builder image.
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import subprocess
import sys
import tempfile
from email.parser import Parser
from pathlib import Path

ELF_MAGIC = b"\x7fELF"

# Matches the version names glibc puts in .dynstr (GLIBC_2.39, GLIBC_2.38.1).
# Bounded on both sides by identifier characters, so a longer symbol that
# merely contains one of these cannot contribute a version the package does not
# actually require -- `GLIBC_2.991x` must yield nothing, not 2.991.
_GLIBC_VERSION = re.compile(rb"(?<![A-Za-z0-9_])GLIBC_(\d+)\.(\d+)(?:\.(\d+))?(?![0-9A-Za-z_])")


def elf_files(root: Path) -> list[Path]:
    """Every regular ELF file under ``root``, in a stable order."""
    found = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        try:
            with path.open("rb") as handle:
                if handle.read(4) == ELF_MAGIC:
                    found.append(path)
        except OSError as error:
            raise SystemExit(f"cannot read packaged file {path}: {error}") from error
    return found


def required_glibc(path: Path) -> tuple[int, ...] | None:
    """The highest GLIBC_x.y[.z] version ``path`` references, if any."""
    versions = [
        tuple(int(part) for part in match.groups() if part is not None)
        for match in _GLIBC_VERSION.finditer(path.read_bytes())
    ]
    return max(versions) if versions else None


def floor_for(root: Path) -> tuple[int, ...]:
    """The glibc version every packaged binary together requires."""
    binaries = elf_files(root)
    if not binaries:
        raise SystemExit(f"no ELF binaries found under {root}; nothing to derive a floor from")
    versions = [version for path in binaries if (version := required_glibc(path)) is not None]
    if not versions:
        raise SystemExit(
            f"none of the {len(binaries)} packaged ELF binaries under {root} reference glibc; "
            "a statically linked cohort must not silently drop the libc floor"
        )
    return max(versions)


def clause(version: tuple[int, ...], package: str) -> str:
    """One Debian dependency clause, e.g. ``libc6 (>= 2.39)``."""
    return f"{package} (>= {'.'.join(str(part) for part in version)})"


def write_preinstall_dependencies(root: Path) -> None:
    """Map candidate symbols and required installer tools before unpack."""
    repository = Path(__file__).resolve().parents[3]
    if importlib.util.find_spec("capsem_builder") is None:
        sys.path.insert(0, str(repository / "build_system/builder"))
        from bootstrap import mount_builder_package
        mount_builder_package(repository)
    from capsem_builder.release.tools import preinstall_resource

    subject = preinstall_resource.candidate(root, preinstall_resource.load_limits(repository))
    binary = root / "usr/bin/capsem-ledger"
    version = required_glibc(binary)
    if version is None:
        raise SystemExit("preinstall candidate has no proven glibc version")
    control = root / "DEBIAN/control"
    original = control.read_text()
    fields = Parser().parsestr(original)
    packages = [value.strip() for value in fields.get_all("Package", [])]
    if len(packages) != 1 or re.fullmatch(r"[a-z0-9][a-z0-9+.-]+", packages[0]) is None:
        raise SystemExit("preinstall candidate requires one valid package identity")
    if len(fields.get_all("Pre-Depends", [])) > 1:
        raise SystemExit("preinstall candidate has duplicate Pre-Depends fields")
    with tempfile.TemporaryDirectory(prefix="capsem-preinstall-shlibs-") as temporary:
        context = Path(temporary)
        (context / "debian").mkdir()
        (context / "debian/control").write_text(f"Source: {packages[0]}\n\n{original}")
        package_directory = context / "debian" / packages[0]
        package_directory.symlink_to(root.resolve(), target_is_directory=True)
        selected = package_directory / "usr/bin/capsem-ledger"
        try:
            result = subprocess.run(
                ["dpkg-shlibdeps", "-O", "-dPre-Depends", f"-S{package_directory}", f"-e{selected}"],
                cwd=context, env={**os.environ, "DEB_HOST_ARCH": subject.architecture},
                check=True, text=True, capture_output=True,
            )
        except (OSError, subprocess.CalledProcessError) as error:
            detail = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
            raise SystemExit(f"candidate target dependency mapping failed: {detail}") from error
    if result.stderr:
        print(result.stderr, file=sys.stderr, end="")
    lines = result.stdout.splitlines()
    prefix = "shlibs:Pre-Depends="
    if len(lines) != 1 or not lines[0].startswith(prefix):
        raise SystemExit("unproven preinstall candidate dependency mapping")
    dependencies = lines[0][len(prefix):].strip()
    clauses = [value.strip() for value in dependencies.split(",")]
    if clause(version, "libc6") not in clauses or any(not value for value in clauses):
        raise SystemExit("candidate glibc floor disagrees with actual target dependency mapping")
    from capsem_builder.gate.config import for_root

    clauses.extend(for_root(repository).install.package_preinstall_packages)
    dependencies = ", ".join(clauses)
    match = re.search(r"(?ms)^Pre-Depends:[ \t]*(.*?)(?=^[^ \t]|\Z)", original)
    if match:
        existing = match.group(1).replace("\n", " ").strip()
        retained = [value.strip() for value in existing.split(",")]
        additions = [value for value in clauses if value not in retained]
        if not additions:
            return
        dependencies = ", ".join(additions)
        replacement = f"Pre-Depends: {existing}, {dependencies}\n" if existing else f"Pre-Depends: {dependencies}\n"
        updated = original[:match.start()] + replacement + original[match.end():]
    else:
        updated = original + ("" if original.endswith("\n") else "\n") + f"Pre-Depends: {dependencies}\n"
    control.write_text(updated)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="the packaged tree to scan")
    parser.add_argument("--package", default="libc6", help="the C library package name")
    parser.add_argument("--preinstall-control", action="store_true", help="write candidate-only target Pre-Depends")
    args = parser.parse_args(argv)

    if not args.root.is_dir():
        raise SystemExit(f"packaged tree does not exist: {args.root}")
    if args.preinstall_control:
        write_preinstall_dependencies(args.root)
    else:
        print(clause(floor_for(args.root), args.package))
    return 0


if __name__ == "__main__":
    sys.exit(main())

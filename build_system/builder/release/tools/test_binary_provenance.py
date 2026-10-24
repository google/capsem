"""Read-only proof for a locally frozen, exact-source host binary cohort."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from capsem_builder import crypto

COHORT_SCHEMA = "capsem.local-test-cohort.v1"
COHORT_VARIABLE = "CAPSEM_LOCAL_TEST_BINARY_COHORT"


def _git(root: Path, *arguments: str) -> str:
    try:
        return subprocess.run(
            ("git", "-C", str(root), *arguments),
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError) as error:
        raise RuntimeError("frozen binary provenance cannot read source identity") from error


def verify_frozen_binary(binary: Path, manifest: Path, project_root: Path) -> None:
    """A directory selector alone is never artifact/source proof.

    The build/freeze owner supplies this manifest only after proving its build
    and copied bytes. This consumer independently checks that binding, current
    committed source and executable bytes, without rebuilding or writing into
    the frozen directory. It is not a full test/release qualification receipt.
    """
    try:
        payload = json.loads(manifest.read_text(encoding="utf-8"))
        if not isinstance(payload, dict) or payload.get("schema") != COHORT_SCHEMA:
            raise ValueError("unsupported cohort schema")
        commit, tree = payload["source_commit"], payload["source_tree"]
        for identity in (commit, tree):
            if (
                not isinstance(identity, str)
                or len(identity) != 40
                or any(value not in "0123456789abcdef" for value in identity)
            ):
                raise ValueError("invalid source identity")
        if Path(payload["bin_dir"]).resolve() != binary.parent.resolve():
            raise ValueError("binary is outside the frozen cohort")
        if _git(project_root, "rev-parse", f"{commit}^{{tree}}") != tree:
            raise ValueError("cohort commit/tree mismatch")
        if _git(project_root, "rev-parse", "HEAD^{tree}") != tree:
            raise ValueError("cohort does not match current source tree")
        if _git(project_root, "status", "--porcelain", "--untracked-files=all"):
            raise ValueError("current source is not a clean committed tree")
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError("frozen binary is missing or not executable")
        if payload["binaries"][binary.name] != crypto.digest_file(binary):
            raise ValueError("frozen binary digest mismatch")
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise RuntimeError(f"frozen binary provenance rejected: {error}") from error

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "verify-immutable-publication.py"
SPEC = importlib.util.spec_from_file_location("verify_immutable_publication", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VERIFY
SPEC.loader.exec_module(VERIFY)


def _publication(root: Path) -> None:
    root.mkdir()
    (root / "channel-source-nightly.json").write_bytes(b'{"channel":"nightly"}\n')
    (root / "x86_64-rootfs.erofs").write_bytes(b"rootfs")


def test_identical_immutable_publication_is_resumable(tmp_path: Path) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)

    VERIFY.verify_identical_publication(expected, actual)


@pytest.mark.parametrize("mutation", ("missing", "extra", "changed", "nested"))
def test_immutable_publication_rejects_any_file_set_or_byte_drift(
    tmp_path: Path,
    mutation: str,
) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)
    if mutation == "missing":
        (actual / "x86_64-rootfs.erofs").unlink()
    elif mutation == "extra":
        (actual / "unexpected").write_bytes(b"extra")
    elif mutation == "changed":
        (actual / "x86_64-rootfs.erofs").write_bytes(b"changed")
    else:
        (actual / "nested").mkdir()

    with pytest.raises(ValueError, match="publication"):
        VERIFY.verify_identical_publication(expected, actual)


def test_immutable_publication_rejects_symlinked_files(tmp_path: Path) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)
    target = actual / "x86_64-rootfs.erofs"
    target.unlink()
    target.symlink_to(expected / "x86_64-rootfs.erofs")

    with pytest.raises(ValueError, match="unsafe entry"):
        VERIFY.verify_identical_publication(expected, actual)

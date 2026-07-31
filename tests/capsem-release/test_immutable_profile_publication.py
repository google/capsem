from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "verify-immutable-publication.py"
PUBLISHER = ROOT / "scripts" / "publish-immutable-release-assets.sh"
SPEC = importlib.util.spec_from_file_location("verify_immutable_publication", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VERIFY
SPEC.loader.exec_module(VERIFY)


def _publication(root: Path) -> None:
    root.mkdir()
    (root / "channel-source-nightly.json").write_bytes(b'{"channel":"nightly"}\n')
    (root / "x86_64-rootfs.erofs").write_bytes(b"rootfs")


def _fake_gh(bin_dir: Path) -> Path:
    executable = bin_dir / "gh"
    executable.write_text(
        """#!/usr/bin/env python3
import os
from pathlib import Path
import shutil
import sys

args = sys.argv[1:]
state = Path(os.environ["FAKE_GH_STATE"])
marker = state / "release"
assets = state / "assets"
assets.mkdir(parents=True, exist_ok=True)
if args[:2] == ["release", "view"]:
    if not marker.exists():
        raise SystemExit(1)
    if "--json" in args:
        print(len(list(assets.iterdir())))
elif args[:2] == ["release", "create"]:
    marker.touch()
elif args[:2] == ["release", "download"]:
    destination = Path(args[args.index("--dir") + 1])
    for source in assets.iterdir():
        shutil.copy2(source, destination / source.name)
elif args[:2] == ["release", "upload"]:
    source = Path(args[3])
    destination = assets / source.name
    if destination.exists():
        print(f"refusing clobber: {destination}", file=sys.stderr)
        raise SystemExit(1)
    shutil.copy2(source, destination)
    upload_log = state / "uploads.log"
    with upload_log.open("a", encoding="utf-8") as output:
        output.write(f"{source.name}\\n")
    upload_count = len(upload_log.read_text(encoding="utf-8").splitlines())
    if upload_count == int(os.environ.get("FAKE_GH_FAIL_AFTER_UPLOAD", "0")):
        print("simulated interrupted publication", file=sys.stderr)
        raise SystemExit(1)
else:
    print(f"unsupported fake gh invocation: {args}", file=sys.stderr)
    raise SystemExit(2)
""",
        encoding="utf-8",
    )
    executable.chmod(0o755)
    return executable


def _publisher_env(tmp_path: Path) -> dict[str, str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _fake_gh(bin_dir)
    notes = tmp_path / "notes.md"
    notes.write_text("notes\n", encoding="utf-8")
    return {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "FAKE_GH_STATE": str(tmp_path / "remote"),
        "CAPSEM_RELEASE_CREATE_TITLE": "Capsem test",
        "CAPSEM_RELEASE_CREATE_NOTES_FILE": str(notes),
    }


def test_identical_immutable_publication_is_resumable(tmp_path: Path) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)

    VERIFY.verify_identical_publication(expected, actual)


def test_partial_owned_publication_reports_only_missing_files(tmp_path: Path) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    actual.mkdir()
    (actual / "channel-source-nightly.json").write_bytes(
        b'{"channel":"nightly"}\n'
    )
    (actual / "unrelated.pkg").write_bytes(b"package")

    assert VERIFY.verify_resumable_owned_publication(expected, actual) == [
        "x86_64-rootfs.erofs"
    ]


def test_complete_owned_publication_ignores_other_stage_files(tmp_path: Path) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)
    (actual / "unrelated.pkg").write_bytes(b"package")

    assert VERIFY.verify_resumable_owned_publication(expected, actual) == []


def test_resumable_owned_publication_rejects_existing_byte_drift(
    tmp_path: Path,
) -> None:
    expected = tmp_path / "expected"
    actual = tmp_path / "actual"
    _publication(expected)
    _publication(actual)
    (actual / "x86_64-rootfs.erofs").write_bytes(b"changed")

    with pytest.raises(ValueError, match="byte mismatch"):
        VERIFY.verify_resumable_owned_publication(expected, actual)


def test_publisher_creates_then_reuses_complete_immutable_release(
    tmp_path: Path,
) -> None:
    expected = tmp_path / "expected"
    _publication(expected)
    env = _publisher_env(tmp_path)

    first = subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    second = subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert first.returncode == 0, first.stderr
    assert second.returncode == 0, second.stderr
    assert sorted(path.name for path in (tmp_path / "remote" / "assets").iterdir()) == [
        "channel-source-nightly.json",
        "x86_64-rootfs.erofs",
    ]


def test_publisher_uploads_source_manifest_last_and_resumes_interruption(
    tmp_path: Path,
) -> None:
    expected = tmp_path / "expected"
    _publication(expected)
    env = {
        **_publisher_env(tmp_path),
        "FAKE_GH_FAIL_AFTER_UPLOAD": "1",
    }

    interrupted = subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    remote = tmp_path / "remote"
    assert interrupted.returncode != 0
    assert sorted(path.name for path in (remote / "assets").iterdir()) == [
        "x86_64-rootfs.erofs"
    ]

    env.pop("FAKE_GH_FAIL_AFTER_UPLOAD")
    resumed = subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert resumed.returncode == 0, resumed.stderr
    assert (remote / "uploads.log").read_text(encoding="utf-8").splitlines() == [
        "x86_64-rootfs.erofs",
        "channel-source-nightly.json",
    ]


def test_publisher_rejects_drift_without_clobbering_remote_bytes(
    tmp_path: Path,
) -> None:
    expected = tmp_path / "expected"
    _publication(expected)
    env = _publisher_env(tmp_path)
    subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    remote = tmp_path / "remote" / "assets" / "x86_64-rootfs.erofs"
    remote.write_bytes(b"remote-drift")

    result = subprocess.run(
        [PUBLISHER, "v1.6.test", expected],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode != 0
    assert "byte mismatch" in result.stderr
    assert remote.read_bytes() == b"remote-drift"


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

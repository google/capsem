"""Execute the host APT boundary with controlled package-manager processes."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


def _run(
    tmp_path: Path, mode: str, *, fresh_cache: bool = False
) -> subprocess.CompletedProcess[str]:
    """Remap only absolute filesystem/tool locations, keeping the shell intact."""
    root = tmp_path / "root"
    for relative in ("etc/apt/apt.conf.d", "var/lib/apt/lists", "var/cache/apt"):
        (root / relative).mkdir(parents=True)
    if not fresh_cache:
        (root / "var/cache/apt/archives").mkdir()
    commands = tmp_path / "bin"
    commands.mkdir()
    manager = commands / "apt-get"
    manager.write_text(
        f"#!{sys.executable}\n"
        "import os, sys\n"
        "from pathlib import Path\n"
        "root = Path(os.environ['FIXTURE_ROOT'])\n"
        "args = sys.argv[1:]\n"
        "with (root / 'commands').open('a') as log: log.write(repr(args) + '\\n')\n"
        "if 'update' in args: sys.exit(0)\n"
        "if '--print-uris' in args:\n"
        "    assert 'Acquire::ForceHash=SHA256' in args\n"
        "    if os.environ['MODE'] == 'planner-error': sys.exit(100)\n"
        "    for i in range(8):\n"
        "        digest = 'a' * 64 if os.environ['MODE'] != 'weak' else 'abcd'\n"
        "        base = 'https://snapshot.ubuntu.com/ubuntu/20260810T000000Z'\n"
        "        if os.environ['MODE'] == 'wrong-source': base = 'https://untrusted.invalid'\n"
        "        name = '../escape.deb' if os.environ['MODE'] == 'path' else f'p{i}.deb'\n"
        "        print(f\"'{base}/pool/p{i}.deb' {name} 10 SHA256:{digest}\")\n"
        "    sys.exit(0)\n"
        "assert '--no-download' in args, 'installation may not silently fetch serially'\n"
        "assert len(list((root / 'var/cache/apt/archives').glob('*.deb'))) == 8\n"
        "(root / 'installed').touch()\n"
    )
    manager.chmod(0o755)
    helper = commands / "apt-helper"
    helper.write_text(
        f"#!{sys.executable}\n"
        "import os, sys, time\n"
        "from pathlib import Path\n"
        "root = Path(os.environ['FIXTURE_ROOT'])\n"
        "args = sys.argv[1:]\n"
        "if os.environ['MODE'] == 'hang': time.sleep(30)\n"
        "at = args.index('download-file')\n"
        "uri, target, digest, size = args[at + 1:]\n"
        "assert digest == 'SHA256:' + 'a' * 64 and size == 'Checksum-FileSize:10'\n"
        "assert 'Acquire::Retries=3' in args and 'Acquire::https::Timeout=15' in args\n"
        "active = root / ('active-' + Path(target).name)\n"
        "active.touch()\n"
        "time.sleep(0.1)\n"
        "count = len(list(root.glob('active-*')))\n"
        "with (root / 'concurrency').open('a') as log: log.write(str(count) + '\\n')\n"
        "time.sleep(0.1)\n"
        "active.unlink()\n"
        "if os.environ['MODE'] == 'corrupt': sys.exit(100)\n"
        "Path(target).write_bytes(b'verified!!')\n"
    )
    helper.chmod(0o755)
    source = (ROOT / "build_system/docker/materialize-host-apt.sh").read_text()
    source = source.replace("/usr/lib/apt/apt-helper", str(helper))
    for prefix in ("/etc/apt", "/var/lib/apt", "/var/cache/apt"):
        source = source.replace(prefix, str(root) + prefix)
    script = tmp_path / "materializer.sh"
    script.write_text(source)
    return subprocess.run(
        ["bash", str(script)],
        env={
            **os.environ,
            "PATH": str(commands) + os.pathsep + os.environ["PATH"],
            "FIXTURE_ROOT": str(root),
            "MODE": mode,
            "WORKSPACE_APT_PACKAGES": "native-dev",
            "WORKSPACE_CROSS_APT_PACKAGES": "cross-dev",
            "APT_SNAPSHOT_BASE": "https://snapshot.ubuntu.com/ubuntu",
            "APT_SNAPSHOT_ID": "20260810T000000Z",
            "APT_UPDATE_TIMEOUT_SECONDS": "3",
            "APT_INSTALL_TIMEOUT_SECONDS": "1" if mode == "hang" else "10",
            "APT_REQUEST_TIMEOUT_SECONDS": "15",
            "APT_RETRIES": "3",
            "APT_DOWNLOAD_CONCURRENCY": "4",
        },
        text=True,
        capture_output=True,
        timeout=15,
        check=False,
    )


def test_archives_download_concurrently_then_install_without_network(tmp_path: Path) -> None:
    result = _run(tmp_path, "valid")
    assert result.returncode == 0, result.stderr
    counts = [int(value) for value in (tmp_path / "root/concurrency").read_text().split()]
    assert 1 < max(counts) <= 4
    assert (tmp_path / "root/installed").exists()


def test_fresh_builder_cache_creates_download_destinations_before_workers(tmp_path: Path) -> None:
    # BuildKit mounts the entire empty cache over /var/cache/apt. The actual
    # helper destination write must work without fixture-created ancestors.
    result = _run(tmp_path, "valid", fresh_cache=True)
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "root/var/cache/apt/archives/partial").is_dir()
    assert (tmp_path / "root/installed").is_file()


@pytest.mark.parametrize("mode", ["weak", "corrupt", "wrong-source", "path", "planner-error"])
def test_unverified_archive_never_reaches_install(tmp_path: Path, mode: str) -> None:
    result = _run(tmp_path, mode)
    assert result.returncode != 0
    assert not (tmp_path / "root/installed").exists()
    commands = (tmp_path / "root/commands").read_text()
    assert "--print-uris" in commands
    assert "--no-download" not in commands


def test_stalled_workers_share_one_finite_materialization_deadline(tmp_path: Path) -> None:
    result = _run(tmp_path, "hang")
    assert result.returncode == 124
    assert not (tmp_path / "root/installed").exists()

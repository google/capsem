"""Platform proof must work without Debian tools or shared host temp paths."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.release.tools import deb_preunpack_probe as probe
from capsem_builder.release.tools import preinstall_resource as resource
from capsem_builder.release.tools.finalize_binary_staging_fixtures import _write_synthetic_deb

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "build_system/packaging/linux/prove-deb-platform-support.py"


@pytest.mark.parametrize("libc,runs,expected", [("2.39", True, 0), ("2.35", False, 0), ("2.39", False, 1)])
def test_platform_proof_reads_exact_package_without_host_debian_tools(
    tmp_path: Path, libc: str, runs: bool, expected: int,
) -> None:
    tree = tmp_path / "tree"
    (tree / "DEBIAN").mkdir(parents=True)
    (tree / "DEBIAN/control").write_text(
        "Package: capsem\nArchitecture: arm64\nDepends: libc6 (>= 2.39)\n"
        "Pre-Depends: libc6 (>= 2.39), libgcc-s1 (>= 4.2)\n"
    )
    binary = tree / "usr/bin/capsem-admin"
    binary.parent.mkdir(parents=True)
    payload = b"exact packaged executable\x00\xff"
    binary.write_bytes(payload)
    binary.chmod(0o755)
    ledger = binary.with_name("capsem-ledger")
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    header[16:20] = b"\x03\x00\xb7\x00"
    ledger.write_bytes(bytes(header) + b"non-executable source double")
    ledger.chmod(0o755)
    (tree / "DEBIAN/preinst").write_bytes(probe.canonical_preinst(ROOT))
    resource.embed(tree, repository=ROOT, limits=resource.load_limits(ROOT))
    package = tmp_path / "candidate.deb"
    _write_synthetic_deb(tree, package)
    config = tmp_path / "gate.toml"
    config.write_text(
        '[platforms.linux]\nminimum_glibc="2.39"\n'
        '[[platforms.linux.distributions]]\nname="Test"\nversion="1"\n'
        f'libc="glibc {libc}"\nrepository="fixture"\ndigest="sha256:{"0" * 64}"\n'
    )
    commands = tmp_path / "commands"
    commands.mkdir()
    received = tmp_path / "received"
    calls = tmp_path / "calls"
    docker = commands / "docker"
    docker.write_text(
        f"#!{sys.executable}\nimport sys, pathlib, json\n"
        f"with pathlib.Path({str(calls)!r}).open('a') as log: log.write(json.dumps(sys.argv[1:])+'\\n')\n"
        "if sys.argv[1] == 'rm': sys.exit(0)\n"
        f"if 'ldd --version' in sys.argv[-1]: print('ldd (GNU libc) {libc}'); sys.exit(0)\n"
        "if 'platform-preunpack' in sys.argv:\n"
        f"    assert sys.stdin.buffer.read() == pathlib.Path({str(package)!r}).read_bytes()\n"
        "    print('CAPSEM_PLATFORM_PREUNPACK_' + ('OK' if sys.argv[-1] == 'supported' else 'REFUSED'))\n"
        "    sys.exit(0)\n"
        f"pathlib.Path({str(received)!r}).write_bytes(sys.stdin.buffer.read())\n"
        f"print({'version' if runs else 'GLIBC_2.39 not found'!r})\n"
        f"sys.exit({0 if runs else 1})\n"
    )
    docker.chmod(0o755)
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--package", str(package), "--config", str(config)],
        env={**os.environ, "PATH": str(commands)},
        capture_output=True, text=True, timeout=10,
    )
    assert result.returncode == expected, result.stdout + result.stderr
    assert received.read_bytes() == payload
    invocations = [json.loads(line) for line in calls.read_text().splitlines()]
    docker_runs = [call for call in invocations if call[0] == "run"]
    removals = [call for call in invocations if call[0] == "rm"]
    assert len(docker_runs) == (3 if expected == 0 else 2)
    assert len(removals) == len(docker_runs)
    names = [call[call.index("--name") + 1] for call in docker_runs]
    assert len(set(names)) == len(names)
    assert [call[-1] for call in removals] == names
    assert all(call[:4] == ["rm", "-f", "-v", name] for call, name in zip(removals, names, strict=True))
    assert all("-v" not in call and "--mount" not in call for call in docker_runs)
    assert all("--pull=never" in call and "--network=none" in call for call in docker_runs)
    assert "-i" in docker_runs[-1]

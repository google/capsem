"""Build runner contract tests."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_run_signed_reuses_only_verified_matching_entitlements(tmp_path: Path) -> None:
    package = tmp_path / "build_system" / "packaging" / "macos"
    package.mkdir(parents=True)
    shutil.copy(PROJECT_ROOT / "build_system/packaging/macos/run_signed.sh", package)
    entitlements = tmp_path / "build_system/packaging/macos/entitlements.plist"
    entitlements.write_text("entitlements-v1\n")
    binary = tmp_path / "program"
    binary.write_text("#!/bin/sh\necho launched\n")
    binary.chmod(0o755)
    commands = tmp_path / "commands"
    commands.mkdir()
    mocks = {
        "uname": "echo Darwin",
        "plutil": 'if [ "$5" = - ]; then cat; else cat "$5"; fi',
        "cp": """
if [ "${1:-}" = -c ]; then
  shift
  /bin/cp "$@" || exit 1
  if [ "${RACE_COPY:-0}" = 1 ] && [ ! -e "$SIGN_STATE/copied" ]; then
    touch "$SIGN_STATE/copied"
    printf '#!/bin/sh\\necho after-copy\\n' > "$SOURCE_BINARY.next"
    chmod 755 "$SOURCE_BINARY.next"
    mv "$SOURCE_BINARY.next" "$SOURCE_BINARY"
  fi
  exit 0
fi
exec /bin/cp "$@"
""",
        "codesign": """
case "$1" in
  --display) cat "$SIGN_STATE/$(stat -f '%d-%i' "$5").entitlements" ;;
  --verify)
    echo verified >> "$VERIFY_CALLS"
    [ "${FAIL_VERIFY:-0}" = 0 ] && cmp -s "$3" "$SIGN_STATE/$(stat -f '%d-%i' "$3").signed" ;;
  --sign)
    echo signed >> "$SIGN_CALLS"
    [ "${FAIL_SIGN:-0}" = 0 ] || exit 17
    sleep 0.1
    identity=$(stat -f '%d-%i' "$6")
    cp "$6" "$SIGN_STATE/$identity.signed"
    cp "$4" "$SIGN_STATE/$identity.entitlements"
    if [ "${REPLACE_SOURCE:-0}" = 1 ]; then
      printf '#!/bin/sh\\necho replaced\\n' > "$SOURCE_BINARY.next"
      chmod 755 "$SOURCE_BINARY.next"
      mv "$SOURCE_BINARY.next" "$SOURCE_BINARY"
    fi ;;
  *) exit 99 ;;
esac
""",
    }
    for name, body in mocks.items():
        path = commands / name
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(0o755)
    stat = commands / "stat"
    stat.write_text(
        f"#!{sys.executable}\n"
        + """
import os, sys
for path in sys.argv[3:]:
    s = os.stat(path)
    if sys.argv[2] == "%d-%i":
        print(f"{s.st_dev}-{s.st_ino}")
    else:
        print(f"{s.st_dev}:{s.st_ino}:{s.st_mode}:{s.st_size}:{s.st_mtime_ns}:{s.st_ctime_ns}")
"""
    )
    stat.chmod(0o755)
    calls = tmp_path / "sign-calls"
    verifies = tmp_path / "verify-calls"
    state = tmp_path / "sign-state"
    state.mkdir()
    env = {
        **os.environ,
        "PATH": f"{commands}:{os.environ['PATH']}",
        "SIGN_CALLS": str(calls),
        "VERIFY_CALLS": str(verifies),
        "SIGN_STATE": str(state),
        "SOURCE_BINARY": str(binary),
    }
    command = ["bash", str(package / "run_signed.sh"), str(binary)]

    # Simultaneous cold launches must publish one signature, then every warm
    # launch must leave the executable untouched (nextest runs one per test).
    children = [
        subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for _ in range(8)
    ]
    for child in children:
        stdout, stderr = child.communicate(timeout=15)
        assert child.returncode == 0, stderr
        assert stdout == b"launched\n"
    assert calls.read_text().splitlines() == ["signed"]
    before = binary.stat().st_mtime_ns
    verified = verifies.read_text()
    subprocess.run(command, env=env, check=True, capture_output=True)
    assert verifies.read_text() == verified
    assert binary.stat().st_mtime_ns == before
    assert calls.read_text().splitlines() == ["signed"]

    entitlements.write_text("entitlements-v2\n")
    subprocess.run(command, env=env, check=True, capture_output=True)
    binary.write_text("#!/bin/sh\necho rebuilt\n")
    os.utime(binary, ns=(before, before))  # restored mtime must not hide a write
    subprocess.run(command, env=env, check=True, capture_output=True)
    assert len(calls.read_text().splitlines()) == 3

    binary.write_text("#!/bin/sh\necho must-not-launch\n")
    failed = subprocess.run(command, env={**env, "FAIL_SIGN": "1"}, capture_output=True)
    assert failed.returncode == 1
    assert b"codesign failed" in failed.stderr
    assert b"must-not-launch" not in failed.stdout
    assert not (tmp_path / "cache/target/.run_signed_codesign.lock").exists()

    failed = subprocess.run(command, env={**env, "FAIL_VERIFY": "1"}, capture_output=True)
    assert failed.returncode == 1
    assert b"verification failed" in failed.stderr
    assert not failed.stdout
    # Failed verification cannot publish a reusable receipt.
    subprocess.run(command, env=env, check=True, capture_output=True)
    before = verifies.read_text()
    runner = package / "run_signed.sh"
    runner.write_text(runner.read_text() + "\n# policy changed\n")
    subprocess.run(command, env=env, check=True, capture_output=True)
    assert verifies.read_text() != before

    # Cargo can replace its public alias after signing starts. The execution
    # must retain the captured bytes, then select new bytes on the next call.
    binary.write_text("#!/bin/sh\necho captured\n")
    raced = subprocess.run(command, env={**env, "REPLACE_SOURCE": "1"}, capture_output=True)
    assert raced.returncode == 0, raced.stderr
    assert raced.stdout == b"captured\n"
    assert b"replaced" in binary.read_bytes()
    fresh = subprocess.run(command, env=env, check=True, capture_output=True)
    assert fresh.stdout == b"replaced\n"
    signed = next(
        path
        for path in tmp_path.glob(".run-signed-program-*")
        if path.read_bytes() == binary.read_bytes()
    )
    assert signed.parent == binary.parent
    before = signed.stat().st_mtime_ns
    signed.write_text("#!/bin/sh\necho tampered\n")
    os.utime(signed, ns=(before, before))
    repaired = subprocess.run(command, env=env, check=True, capture_output=True)
    assert repaired.stdout == b"replaced\n"

    binary.write_text("#!/bin/sh\necho before-copy\n")
    copied = subprocess.run(command, env={**env, "RACE_COPY": "1"}, capture_output=True)
    assert copied.returncode == 0, copied.stderr
    assert copied.stdout == b"after-copy\n"

    alias = tmp_path / "program-alias"
    os.link(binary, alias)
    alias_command = [*command[:-1], str(alias)]
    for invocation in (command, alias_command):
        subprocess.run(invocation, env=env, check=True, capture_output=True)
    verified = verifies.read_text()
    for invocation in (command, alias_command):
        subprocess.run(invocation, env=env, check=True, capture_output=True)
    assert verifies.read_text() == verified, "hardlinked names must retain independent receipts"

    binary.write_text(
        '#!/bin/sh\nif [ -e "/dev/fd/$PROBE_FD" ]; then echo inherited; else echo closed; fi\n'
    )
    read_fd, write_fd = os.pipe()
    try:
        for nextest, expected in (("", b"inherited\n"), ("1", b"closed\n")):
            inherited = subprocess.run(
                command,
                env={**env, "NEXTEST": nextest, "PROBE_FD": str(write_fd)},
                pass_fds=(write_fd,),
                capture_output=True,
                check=True,
            )
            assert inherited.stdout == expected
    finally:
        os.close(read_fd)
        os.close(write_fd)


def test_run_signed_serializes_codesign_without_flock() -> None:
    script = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "run_signed.sh").read_text()

    assert "SIGN_LOCK_DIR=" in script
    assert "acquire_sign_lock" in script
    assert "release_sign_lock" in script
    assert 'mkdir "$SIGN_LOCK_DIR"' in script
    assert "flock" not in script


@pytest.mark.parametrize("platform", ["Linux", "Darwin"])
def test_run_signed_materializes_its_cache_leaves(tmp_path: Path, platform: str) -> None:
    package_dir = tmp_path / "build_system" / "packaging" / "macos"
    package_dir.mkdir(parents=True)
    source = PROJECT_ROOT / "build_system" / "packaging" / "macos"
    shutil.copy(source / "run_signed.sh", package_dir)

    # Exercise both host branches without codesigning a real developer binary.
    # The copied runner deliberately has no entitlements beside it.
    result = subprocess.run(
        (
            "bash",
            "-c",
            'uname() { echo "$TEST_PLATFORM"; }; export -f uname; exec bash "$@"',
            "bash",
            str(package_dir / "run_signed.sh"),
            sys.executable,
        ),
        env={**os.environ, "TEST_PLATFORM": platform},
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 1
    expected = "codesign requires macOS" if platform == "Linux" else f"not found at {package_dir}/"
    assert expected in result.stderr
    assert (tmp_path / "cache" / "target").is_dir()
    assert expected in (tmp_path / "cache" / "containers" / "logs" / "build.log").read_text()

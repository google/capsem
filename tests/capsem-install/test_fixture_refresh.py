"""Regression tests for the simulated install fixture itself."""

from __future__ import annotations

from pathlib import Path

from . import conftest
from .conftest import (
    BINARIES,
    DEFAULT_BIN_SRC,
    HOST_CRATES,
    PROJECT_ROOT,
    _binary_is_current,
    _ensure_local_binaries_built,
    _installed_binaries_current,
    _should_build_default_bin_src,
)


def _write(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    path.chmod(0o755)


def test_binary_is_current_detects_stale_existing_binary(tmp_path):
    src = tmp_path / "src"
    install = tmp_path / "install"
    _write(src / "capsem", "new binary\n")
    _write(install / "capsem", "old binary\n")

    assert not _binary_is_current("capsem", src, install)


def test_installed_binaries_current_requires_content_match(tmp_path):
    src = tmp_path / "src"
    install = tmp_path / "install"
    for name in BINARIES:
        _write(src / name, f"{name} fresh\n")
        _write(install / name, f"{name} fresh\n")

    assert _installed_binaries_current(src, install)

    _write(src / "capsem-tray", "capsem-tray rebuilt\n")

    assert not _installed_binaries_current(src, install)


def test_default_bin_src_is_built_before_fixture_refresh(monkeypatch):
    calls = []

    def fake_run(cmd, **kwargs):
        calls.append((cmd, kwargs))
        return conftest.subprocess.CompletedProcess(cmd, 0, "built\n", "")

    monkeypatch.setattr(conftest, "_LOCAL_BUILD_DONE", False)
    monkeypatch.delenv("CAPSEM_INSTALL_SKIP_BUILD", raising=False)
    monkeypatch.setattr(conftest.subprocess, "run", fake_run)

    _ensure_local_binaries_built(DEFAULT_BIN_SRC)

    assert len(calls) == 1
    cmd, kwargs = calls[0]
    assert cmd[:2] == ["cargo", "build"]
    assert kwargs["cwd"] == PROJECT_ROOT
    for crate in HOST_CRATES:
        assert ["-p", crate] == cmd[cmd.index(crate) - 1 : cmd.index(crate) + 1]


def test_custom_bin_src_skips_auto_build(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_BIN_SRC", str(tmp_path / "prebuilt"))
    monkeypatch.delenv("CAPSEM_INSTALL_SKIP_BUILD", raising=False)

    assert not _should_build_default_bin_src(tmp_path / "prebuilt")


def test_clean_state_stops_systemd_unit_before_process_kill():
    """The deb harness unit uses Restart=always, so process kill alone races."""
    text = (PROJECT_ROOT / "tests" / "capsem-install" / "conftest.py").read_text()

    assert 'os.environ.get("CAPSEM_DEB_INSTALLED") == "1"' in text
    assert '["systemctl", "--user", "stop", "capsem"]' in text
    assert text.index('["systemctl", "--user", "stop", "capsem"]') < text.index(
        '["pkill", "-f", f"{install_prefix}{proc_name}"]'
    )

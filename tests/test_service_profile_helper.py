"""Contracts for the integration-test service helper."""

from pathlib import Path

import pytest

from tests.helpers import service as service_helper
from tests.helpers.constants import EXEC_READY_TIMEOUT


def test_exec_ready_timeout_covers_parallel_kvm_boot_pressure() -> None:
    assert EXEC_READY_TIMEOUT >= 60


def test_materialize_test_profiles_rejects_empty_generated_catalog(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    generated = tmp_path / "generated" / "profiles"
    generated.mkdir(parents=True)
    monkeypatch.setattr(service_helper, "PROFILES_DIR", generated)

    with pytest.raises(RuntimeError, match="contains no profile.toml"):
        service_helper.materialize_test_profiles(tmp_path / "run")


def test_materialize_test_profiles_copies_real_generated_profiles(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    generated = tmp_path / "generated" / "profiles"
    (generated / "code").mkdir(parents=True)
    (generated / "code" / "profile.toml").write_text('id = "code"\n')
    monkeypatch.setattr(service_helper, "PROFILES_DIR", generated)

    copied = service_helper.materialize_test_profiles(tmp_path / "run")

    assert (copied / "code" / "profile.toml").read_text() == 'id = "code"\n'


def test_service_instance_uses_private_production_shaped_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)

    service = service_helper.ServiceInstance()

    assert service.home_dir == home
    assert service.tmp_dir == home / "run"
    assert service.tmp_dir.is_dir()
    assert service.uds_path.parent == service.tmp_dir
    assert service.tmp_dir.parent / "sessions" == home / "sessions"


def test_service_instance_stop_removes_private_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    monkeypatch.setattr(service_helper, "preserve_tmp_dir_on_failure", lambda _path: None)
    service = service_helper.ServiceInstance()

    service.stop()

    assert not home.exists()


def test_service_instance_can_keep_shutdown_flushed_state_for_assertions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    monkeypatch.setattr(service_helper, "preserve_tmp_dir_on_failure", lambda _path: None)
    service = service_helper.ServiceInstance()
    state = service.home_dir / "sessions" / "main.db"
    state.parent.mkdir()
    state.write_bytes(b"flushed")

    service.stop(cleanup=False)
    assert state.read_bytes() == b"flushed"

    service.stop()
    assert not home.exists()

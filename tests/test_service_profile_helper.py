"""Contracts for the integration-test service helper."""

from pathlib import Path

import pytest

from tests.helpers import service as service_helper
from tests.helpers.constants import EXEC_READY_TIMEOUT


def test_exec_ready_timeout_covers_parallel_kvm_boot_pressure() -> None:
    assert EXEC_READY_TIMEOUT >= 60


def test_service_fixture_log_filter_suppresses_expected_notify_races() -> None:
    value = service_helper.test_rust_log_filter({})

    assert value.startswith("service=info,capsem=debug")
    assert "notify::poll::data=error" in value
    assert value.endswith("debug,notify::poll::data=error")


@pytest.mark.parametrize("variable", ["RUST_LOG", "CAPSEM_TEST_RUST_LOG"])
def test_service_fixture_log_filter_honors_diagnostic_override(variable: str) -> None:
    assert (
        service_helper.test_rust_log_filter({variable: "capsem=trace"})
        == "service=info,capsem=debug,capsem=trace"
    )


def test_service_fixture_log_filter_keeps_required_evidence_under_ambient_warn() -> None:
    assert (
        service_helper.test_rust_log_filter({"RUST_LOG": "warn"})
        == "service=info,capsem=debug,warn"
    )


def test_materialize_test_profiles_rejects_empty_generated_catalog(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    generated = tmp_path / "generated" / "profiles"
    generated.mkdir(parents=True)
    monkeypatch.setattr(service_helper, "PROFILES_DIR", generated)

    with pytest.raises(RuntimeError, match=r"contains no profile.toml"):
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


def test_service_instance_stop_and_read_log_reads_complete_rotated_stream(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    monkeypatch.setattr(service_helper, "preserve_tmp_dir_on_failure", lambda _path: None)
    service = service_helper.ServiceInstance()
    (service.tmp_dir / "service.log").write_text("first\n")
    (service.tmp_dir / "service.2026-08-30.log").write_text("second\n")

    assert service.stop_and_read_log() == "first\nsecond\n"
    assert home.exists()

    service.stop()
    assert not home.exists()


def test_service_instance_preserves_artifacts_during_exception_teardown(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A failure active inside ``finally`` must survive pre-report teardown."""
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    preserved: list[tuple[Path, bool]] = []

    def record_preserve(path: Path, *, force: bool = False) -> None:
        preserved.append((Path(path), force))

    monkeypatch.setattr(service_helper, "preserve_tmp_dir_on_failure", record_preserve)
    service = service_helper.ServiceInstance()

    with pytest.raises(RuntimeError, match="benchmark failed"):
        try:
            raise RuntimeError("benchmark failed")
        finally:
            service.stop()

    assert preserved == [(home, True)]
    assert not home.exists()

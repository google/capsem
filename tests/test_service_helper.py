"""Contracts for the integration-test service helpers."""

import json
import tomllib
from pathlib import Path

import pytest

from tests.helpers import service as service_helper
from tests.helpers.constants import EXEC_READY_TIMEOUT
from tests.helpers.persistent_registry import registry_entry, write_registry
from tests.helpers.settings_policy import write_settings_rule


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


def test_service_fixture_log_filter_keeps_required_evidence_under_ambient_warn() -> (
    None
):
    assert (
        service_helper.test_rust_log_filter({"RUST_LOG": "warn"})
        == "service=info,capsem=debug,warn"
    )


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
    monkeypatch.setattr(
        service_helper, "preserve_tmp_dir_on_failure", lambda _path: None
    )
    service = service_helper.ServiceInstance()

    service.stop()

    assert not home.exists()


def test_service_instance_can_keep_shutdown_flushed_state_for_assertions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    monkeypatch.setattr(
        service_helper, "preserve_tmp_dir_on_failure", lambda _path: None
    )
    service = service_helper.ServiceInstance()
    state = service.home_dir / "sessions" / "host.db"
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
    monkeypatch.setattr(
        service_helper, "preserve_tmp_dir_on_failure", lambda _path: None
    )
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


class _Client:
    def __init__(self, response=None, error=None):
        self._response = response
        self._error = error

    def post(self, *args, **kwargs):
        if self._error is not None:
            raise self._error
        return self._response


@pytest.mark.parametrize(
    "client",
    [_Client(error=TimeoutError()), _Client(None), _Client({"error": "VM is booting"})],
)
def test_exec_ready_is_false_when_the_vm_cannot_answer(client) -> None:
    assert service_helper.wait_exec_ready(client, "vm", timeout=1) is False


def test_exec_ready_reports_an_unreadable_answer_instead_of_a_dead_vm() -> None:
    """A binary transition probe boots the public release, whose exec answers
    stdout as a plain string; swallowing the decode error reported a VM that
    had just run the command as one that never became ready."""
    with pytest.raises(TypeError):
        service_helper.wait_exec_ready(_Client({"stdout": "ready\n"}), "vm", timeout=1)


def test_installed_exec_output_reads_both_published_wire_shapes() -> None:
    read = service_helper.installed_exec_output_text
    assert read({"stdout": "ready\n"}) == "ready\n"
    assert read({"stdout": {"encoding": "utf8", "data": "ready\n"}}) == "ready\n"
    with pytest.raises(AssertionError):
        read({"stdout": {"encoding": "base64", "data": "cmVhZHkK"}})
    assert service_helper.wait_exec_ready(
        _Client({"stdout": "ready\n"}), "vm", timeout=1, read=read
    )


def test_service_instance_names_no_profile_catalog(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The service reads no profiles; the helper must not hand it one."""
    home = tmp_path / "capsem-home"
    home.mkdir()
    monkeypatch.setattr(service_helper, "make_capsem_tmp_dir", lambda _prefix: home)
    service = service_helper.ServiceInstance()
    assert not hasattr(service, "profiles_dir")
    assert not hasattr(service_helper, "materialize_test_profiles")
    assert not hasattr(service_helper, "wait_profile_assets_settled")


class _StatusClient:
    def __init__(self, answers: list[dict]) -> None:
        self.answers = answers
        self.paths: list[str] = []

    def get(self, path: str) -> dict:
        self.paths.append(path)
        return self.answers.pop(0)


def test_wait_assets_settled_polls_the_asset_status_route() -> None:
    client = _StatusClient([{"downloading": True}, {"downloading": False, "ready": True}])
    assert service_helper.wait_assets_settled(client, timeout=5) == {
        "downloading": False,
        "ready": True,
    }
    assert client.paths == ["/assets/status", "/assets/status"]


def test_settings_rule_replaces_only_its_own_table(tmp_path: Path) -> None:
    grant = '[images]\nsources = ["127.0.0.1:5000"]\nadmit = ["127.0.0.1:5000"]\n'
    (tmp_path / "settings.toml").write_text(grant)
    write_settings_rule(tmp_path, "block_example", action="ask", match='http.host == "example.com"')
    write_settings_rule(tmp_path, "block_example", action="block", match='http.host == "example.com"')
    write_settings_rule(tmp_path, "other", action="allow", match="true")

    text = (tmp_path / "settings.toml").read_text()
    assert text.startswith(grant)
    assert text.count("[profiles.rules.block_example]") == 1
    settings = tomllib.loads(text)
    assert settings["images"]["sources"] == ["127.0.0.1:5000"]
    assert settings["profiles"]["rules"] == {
        "block_example": {
            "name": "block_example",
            "action": "block",
            "match": 'http.host == "example.com"',
        },
        "other": {"name": "other", "action": "allow", "match": "true"},
    }


def test_registry_entry_has_the_current_shape_and_manifest_pins(tmp_path: Path) -> None:
    assets = tmp_path / "assets"
    assets.mkdir()
    arch = "arm64" if __import__("platform").machine().lower() in ("arm64", "aarch64") else "x86_64"
    (assets / "manifest.json").write_text(
        json.dumps(
            {
                "assets": {
                    "current": "1",
                    "releases": {
                        "1": {
                            "arches": {
                                arch: {
                                    "vmlinuz": {"hash": "a" * 64},
                                    "initrd.img": {"hash": "blake3:" + "b" * 64},
                                    "rootfs.erofs": {"hash": "c" * 64},
                                }
                            }
                        }
                    },
                }
            }
        )
    )
    run = tmp_path / "run"
    entry = registry_entry(run, "vm-id", "vm-name", assets_dir=assets)
    write_registry(run, [entry])

    assert not {key for key in entry if key.startswith("profile")}
    assert entry["asset_pins"] == {
        "kernel": {"name": "vmlinuz", "hash": "blake3:" + "a" * 64},
        "initrd": {"name": "initrd.img", "hash": "blake3:" + "b" * 64},
        "rootfs": {"name": "rootfs.erofs", "hash": "blake3:" + "c" * 64},
    }
    overlay = Path(entry["session_dir"]) / "system" / "rootfs.img"
    assert overlay.stat().st_size == 1024 * 1024 * 1024
    registry = json.loads((run / "persistent_registry.json").read_text())
    assert registry == {"vms": {"vm-name": entry}}

    legacy = registry_entry(run, "old-id", "old-name", assets_dir=assets, overlay=False, profile_id="code")
    assert legacy["profile_id"] == "code"
    assert not (Path(legacy["session_dir"]) / "system").exists()

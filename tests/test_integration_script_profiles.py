import importlib.util
import os
import subprocess
from pathlib import Path


def load_integration_script():
    script_path = Path(__file__).resolve().parents[1] / "scripts" / "integration_test.py"
    spec = importlib.util.spec_from_file_location("capsem_integration_test", script_path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_integration_script_uses_materialized_profiles_dir():
    module = load_integration_script()

    assert module.default_materialized_profiles_dir().endswith("target/config/profiles")
    assert module._profile_env()["CAPSEM_PROFILES_DIR"] == module.default_materialized_profiles_dir()


def test_integration_script_service_paths_use_process_scoped_isolated_home():
    module = load_integration_script()

    assert module.INTEGRATION_HOME == (
        module.PROJECT_ROOT / "target" / f"integration-capsem-home-{os.getpid()}"
    )
    assert module.CAPSEM_HOME == module.INTEGRATION_HOME
    assert module.INTEGRATION_RUNTIME_ROOT.name == f"capsem-integration-{os.getuid()}-{os.getpid()}"
    assert module.INTEGRATION_RUN_DIR == module.INTEGRATION_RUNTIME_ROOT / "run"
    assert module.SERVICE_SOCKET == module.INTEGRATION_RUN_DIR / "service.sock"
    assert module.SESSIONS_DIR == module.INTEGRATION_RUN_DIR / "sessions"
    assert module.MAIN_DB == module.INTEGRATION_RUNTIME_ROOT / "sessions" / "main.db"
    assert len(os.fsencode(module.SERVICE_SOCKET)) < 108


def test_integration_script_honors_explicit_home_override(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_INTEGRATION_HOME", str(tmp_path / "integration-home"))
    monkeypatch.setenv("CAPSEM_INTEGRATION_RUNTIME_ROOT", str(tmp_path / "runtime-root"))

    module = load_integration_script()

    assert module.INTEGRATION_HOME == tmp_path / "integration-home"
    assert module.INTEGRATION_RUNTIME_ROOT == tmp_path / "runtime-root"
    assert module.SERVICE_SOCKET == module.INTEGRATION_RUN_DIR / "service.sock"


def test_integration_script_uses_isolated_credential_broker_store():
    module = load_integration_script()

    env = module._test_isolation_env()

    assert env["CAPSEM_CREDENTIAL_STORE_PATH"] == str(
        module.INTEGRATION_HOME / "run" / "credential-store.json"
    )


def test_integration_script_discovers_profile_scoped_session_names(tmp_path):
    module = load_integration_script()

    sessions = tmp_path / "run" / "sessions"
    sessions.mkdir(parents=True)
    (sessions / "already-there").mkdir()
    code = sessions / "code-1"
    cowork = sessions / "co-work-1"
    code.mkdir()
    cowork.mkdir()
    older = 1_700_000_000
    newer = older + 10
    os.utime(code, (older, older))
    os.utime(cowork, (newer, newer))

    discovered = module._new_session_dirs(sessions, {"already-there"})

    assert [p.name for p in discovered] == ["co-work-1", "code-1"]


def test_integration_script_does_not_require_legacy_tmp_suffix(tmp_path):
    module = load_integration_script()

    sessions = tmp_path / "run" / "sessions"
    sessions.mkdir(parents=True)
    (sessions / "code-1").mkdir()

    assert [p.name for p in module._new_session_dirs(sessions, set())] == ["code-1"]


def test_integration_model_fixture_command_is_bounded_and_asserts_output_file():
    module = load_integration_script()

    command = module._vm_command("http://127.0.0.1:3713")

    assert "/v1/chat/completions" in command
    assert "--connect-timeout 5 -m 30" in command
    assert "test -s /root/model_fixture.json" in command
    assert " && " in command
    assert "|| true" in command


def test_service_ready_wait_accepts_zero_exit_peer_startup(tmp_path):
    module = load_integration_script()

    sock = tmp_path / "service.sock"
    sock.write_text("")
    attempts = {"count": 0}

    class Proc:
        returncode = 0

        def poll(self):
            return 0

    def fake_run(*_args, **_kwargs):
        attempts["count"] += 1
        return subprocess.CompletedProcess([], 0 if attempts["count"] == 2 else 7)

    now = {"value": 0.0}

    def fake_now():
        return now["value"]

    def fake_sleep(seconds):
        now["value"] += seconds

    module._wait_for_service_ready(
        Proc(),
        service_socket=sock,
        log_path=tmp_path / "service.log",
        timeout_secs=1,
        poll_interval=0.1,
        run_cmd=fake_run,
        sleep=fake_sleep,
        monotonic=fake_now,
    )

    assert attempts["count"] == 2


def test_start_service_creates_run_dir_before_pidfile(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_INTEGRATION_HOME", str(tmp_path / "integration-home"))
    module = load_integration_script()

    class FakeProc:
        pid = 424242

    captured = {}

    def fake_popen(args, **kwargs):
        captured["args"] = args
        captured["env"] = kwargs["env"]
        return FakeProc()

    monkeypatch.setattr(module.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(module, "_wait_for_service_ready", lambda *_args, **_kwargs: None)

    module._start_service_with_test_config(
        "assets",
        "tests/fixtures/config/integration/settings.toml",
        "tests/fixtures/config/integration/corp.toml",
    )

    assert module.SERVICE_PIDFILE.read_text() == "424242"
    assert captured["env"]["CAPSEM_HOME"] == str(module.INTEGRATION_HOME)
    assert captured["env"]["CAPSEM_RUN_DIR"] == str(module.INTEGRATION_RUN_DIR)
    assert captured["env"]["RUST_LOG"] == "info"
    assert "--uds-path" in captured["args"]
    assert captured["args"][captured["args"].index("--uds-path") + 1] == str(module.SERVICE_SOCKET)

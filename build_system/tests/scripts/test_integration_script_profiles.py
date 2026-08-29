import importlib.util
import os
import re
import subprocess
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def load_integration_script():
    script_path = PROJECT_ROOT / "scripts" / "integration_test.py"
    spec = importlib.util.spec_from_file_location("capsem_integration_test", script_path)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def _just_python_entrypoints() -> list[Path]:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    referenced = {
        PROJECT_ROOT / relative
        for relative in re.findall(r"scripts/[A-Za-z0-9_.-]+\.py", justfile)
    }
    referenced.add(
        PROJECT_ROOT / "build_system" / "scripts" / "doctor" / "doctor_session_test.py"
    )
    return sorted(
        path
        for path in referenced
        if any(
            marker in path.read_text(encoding="utf-8")
            for marker in ("ArgumentParser", "capsem_builder.gate.tools.doctor")
        )
    )


@pytest.mark.parametrize(
    "script_path",
    _just_python_entrypoints(),
    ids=lambda path: path.name,
)
def test_just_python_entrypoints_load_under_the_host_python(script_path):
    result = subprocess.run(
        ["python3", str(script_path), "--help"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 0, result.stderr
    assert "usage:" in result.stdout.lower()


def test_integration_script_uses_materialized_profiles_dir(monkeypatch):
    monkeypatch.delenv("CAPSEM_PROFILES_DIR", raising=False)
    module = load_integration_script()

    assert module.default_materialized_profiles_dir().endswith("target/config/profiles")
    assert module._profile_env()["CAPSEM_PROFILES_DIR"] == module.default_materialized_profiles_dir()


def test_integration_script_honors_selected_profiles_dir(monkeypatch):
    monkeypatch.setenv("CAPSEM_PROFILES_DIR", "/verified/profile/catalog")
    module = load_integration_script()

    assert module.default_materialized_profiles_dir() == "/verified/profile/catalog"


def test_integration_script_pins_every_cli_run_to_the_selected_profile():
    module = load_integration_script()

    assert module._profile_run_prefix(
        "target/debug/capsem", "experimental", timeout=300
    ) == [
        "target/debug/capsem",
        "run",
        "--timeout",
        "300",
        "--profile",
        "experimental",
    ]
    assert module._profile_run_prefix("target/debug/capsem", "co-work") == [
        "target/debug/capsem",
        "run",
        "--profile",
        "co-work",
    ]


def test_integration_telemetry_uses_a_retained_named_session(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_INTEGRATION_HOME", str(tmp_path / "integration-home"))
    monkeypatch.setenv(
        "CAPSEM_INTEGRATION_RUNTIME_ROOT", str(tmp_path / "runtime-root")
    )
    module = load_integration_script()

    persistent_dir = module.INTEGRATION_RUN_DIR / "persistent"
    persistent_dir.mkdir(parents=True)
    calls = []
    stopped = {"value": False}

    class FakeService:
        def send_signal(self, _signal):
            pass

        def wait(self, timeout):
            assert timeout == 10

        def kill(self):
            raise AssertionError("clean fake service must not be killed")

    monkeypatch.setattr(module, "_kill_dev_service", lambda: None)
    monkeypatch.setattr(
        module,
        "_start_service_with_test_config",
        lambda *_args: FakeService(),
    )
    monkeypatch.setattr(
        module,
        "start_mock_server",
        lambda: (object(), {"base_url": "http://127.0.0.1:3713"}),
    )
    monkeypatch.setattr(module, "stop_process", lambda _process: None)

    def fake_run(args, **_kwargs):
        args = [str(arg) for arg in args]
        calls.append(args)
        if len(args) > 1 and args[1] == "create":
            (persistent_dir / "vm-123").mkdir()
            return subprocess.CompletedProcess(args, 0, "vm-123 (persistent)\n", "")
        if len(args) > 1 and args[1] == "exec":
            return subprocess.CompletedProcess(args, 0, "CAPSEM_INTEGRATION_DONE\n", "")
        if args[0] == "curl" and args[-1].endswith("/vms/vm-123/stop"):
            stopped["value"] = True
            return subprocess.CompletedProcess(args, 0, '{"success":true}', "")
        if len(args) > 1 and args[1] == "delete":
            assert stopped["value"]
            return subprocess.CompletedProcess(args, 0, "Session deleted.\n", "")
        raise AssertionError(f"unexpected command: {args}")

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    verified = {}

    def fake_verify(session_id, session_dir):
        assert stopped["value"], "the retained session must be stopped before DB inspection"
        verified["session_id"] = session_id
        verified["session_dir"] = session_dir
        return True

    monkeypatch.setattr(module, "verify_session", fake_verify)

    telemetry_ok, exit_code = module.run_vm("capsem", "assets", "code")

    assert telemetry_ok
    assert exit_code == 0
    assert verified == {
        "session_id": "vm-123",
        "session_dir": persistent_dir / "vm-123",
    }
    assert [args[1] if args[0] != "curl" else "stop" for args in calls] == [
        "create",
        "exec",
        "stop",
        "delete",
    ]
    assert all(len(args) < 2 or args[1] != "run" for args in calls)


def test_integration_script_service_paths_use_process_scoped_isolated_home():
    module = load_integration_script()

    assert (
        module.PROJECT_ROOT / "target" / f"integration-capsem-home-{os.getpid()}"
    ) == module.INTEGRATION_HOME
    assert module.CAPSEM_HOME == module.INTEGRATION_HOME
    assert module.INTEGRATION_RUNTIME_ROOT.name == f"capsem-integration-{os.getuid()}-{os.getpid()}"
    assert module.INTEGRATION_RUN_DIR == module.INTEGRATION_RUNTIME_ROOT / "run"
    assert module.SERVICE_SOCKET == module.INTEGRATION_RUN_DIR / "service.sock"
    assert module.PERSISTENT_DIR == module.INTEGRATION_RUN_DIR / "persistent"
    assert module.MAIN_DB == module.INTEGRATION_RUNTIME_ROOT / "sessions" / "main.db"
    assert len(os.fsencode(module.SERVICE_SOCKET)) < 108


def test_integration_script_honors_explicit_home_override(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_INTEGRATION_HOME", str(tmp_path / "integration-home"))
    monkeypatch.setenv("CAPSEM_INTEGRATION_RUNTIME_ROOT", str(tmp_path / "runtime-root"))

    module = load_integration_script()

    assert tmp_path / "integration-home" == module.INTEGRATION_HOME
    assert tmp_path / "runtime-root" == module.INTEGRATION_RUNTIME_ROOT
    assert module.SERVICE_SOCKET == module.INTEGRATION_RUN_DIR / "service.sock"


def test_integration_script_uses_isolated_credential_broker_store():
    module = load_integration_script()

    env = module._test_isolation_env()

    assert env["CAPSEM_CREDENTIAL_STORE_PATH"] == str(
        module.INTEGRATION_HOME / "run" / "credential-store.json"
    )


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


@pytest.mark.parametrize("host_arch", ["arm64", "x86_64"])
def test_start_service_uses_current_host_assets_from_dual_arch_tree(
    tmp_path, monkeypatch, host_arch
):
    monkeypatch.setenv("CAPSEM_INTEGRATION_HOME", str(tmp_path / "integration-home"))
    module = load_integration_script()

    assets = tmp_path / "assets"
    for arch in ("arm64", "x86_64"):
        (assets / arch).mkdir(parents=True)
    (assets / "current").symlink_to(host_arch)

    class FakeProc:
        pid = 424242

    captured = {}

    def fake_popen(args, **_kwargs):
        captured["args"] = args
        return FakeProc()

    monkeypatch.setattr(module.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(module, "_wait_for_service_ready", lambda *_args, **_kwargs: None)

    module._start_service_with_test_config(
        str(assets),
        "tests/fixtures/config/integration/settings.toml",
        "tests/fixtures/config/integration/corp.toml",
    )

    selected = Path(captured["args"][captured["args"].index("--assets-dir") + 1])
    assert selected == assets / "current"
    assert selected.resolve() == (assets / host_arch).resolve()


def test_service_assets_dir_preserves_direct_architecture_root(tmp_path):
    module = load_integration_script()
    assets = tmp_path / "assets-x86_64"
    assets.mkdir()
    (assets / "vmlinuz").write_bytes(b"kernel")

    assert module._service_assets_dir(str(assets)) == str(assets)


def test_vm_failure_diagnostics_are_bounded_and_include_process_log(tmp_path, capsys):
    module = load_integration_script()
    session_dir = tmp_path / "sessions" / "code-1-failed"
    session_dir.mkdir(parents=True)
    process_log = session_dir / "process.log"
    process_log.write_text(
        "\n".join(
            [f"old process line {number}" for number in range(100)]
            + [
                "p" * 20_000,
                "kernel architecture mismatch: ARM64 image on x86_64 host",
            ]
        )
    )
    proc = subprocess.CompletedProcess(
        ["capsem", "run"],
        1,
        stdout="",
        stderr="\n".join(
            [f"old stderr line {number}" for number in range(100)]
            + ["e" * 20_000, "VM assets rejected"]
        ),
    )

    module._print_vm_failure_diagnostics(proc, session_dir)

    output = capsys.readouterr().out
    assert "VM assets rejected" in output
    assert "kernel architecture mismatch: ARM64 image on x86_64 host" in output
    assert str(process_log) in output
    assert "old stderr line 0" not in output
    assert "old process line 0" not in output
    assert len(output) < 2 * module._FAILURE_DIAGNOSTIC_MAX_CHARS + 1_000


def test_vm_failure_diagnostics_are_quiet_for_success(tmp_path, capsys):
    module = load_integration_script()
    proc = subprocess.CompletedProcess(["capsem", "run"], 0, stdout="ok", stderr="")

    module._print_vm_failure_diagnostics(proc, tmp_path)

    assert capsys.readouterr().out == ""

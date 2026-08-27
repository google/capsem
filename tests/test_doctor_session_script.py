import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def load_doctor_script():
    script_path = PROJECT_ROOT / "scripts" / "doctor_session_test.py"
    spec = importlib.util.spec_from_file_location("capsem_doctor_session_test", script_path)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_doctor_ledger_session_is_stopped_then_verified_then_deleted(tmp_path, monkeypatch):
    monkeypatch.setenv("CAPSEM_HOME", str(tmp_path / "home"))
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(tmp_path / "run"))
    module = load_doctor_script()

    module.PERSISTENT_DIR.mkdir(parents=True)
    calls = []
    stopped = {"value": False}
    verified = {"value": False}

    def fake_run(args, **_kwargs):
        args = [str(arg) for arg in args]
        calls.append(args)
        if len(args) > 1 and args[1] == "create":
            (module.PERSISTENT_DIR / "vm-456").mkdir()
            return subprocess.CompletedProcess(args, 0, "vm-456 (persistent)\n", "")
        if len(args) > 1 and args[1] == "exec":
            return subprocess.CompletedProcess(args, 0, "RESULT: PASS\n", "")
        if args[0] == "curl" and args[-1].endswith("/vms/vm-456/stop"):
            stopped["value"] = True
            return subprocess.CompletedProcess(args, 0, '{"success":true}', "")
        if len(args) > 1 and args[1] == "delete":
            assert verified["value"], "doctor evidence must be verified before deletion"
            return subprocess.CompletedProcess(args, 0, "Session deleted.\n", "")
        raise AssertionError(f"unexpected command: {args}")

    def fake_verify(session_id, session_dir):
        assert stopped["value"], "doctor session must be stopped before DB inspection"
        assert session_id == "vm-456"
        assert session_dir == module.PERSISTENT_DIR / "vm-456"
        verified["value"] = True
        return True

    monkeypatch.setattr(module.subprocess, "run", fake_run)
    monkeypatch.setattr(module, "verify_session", fake_verify)
    monkeypatch.setattr(
        module,
        "start_mock_server",
        lambda: (object(), {"base_url": "http://127.0.0.1:3713"}),
    )
    monkeypatch.setattr(module, "stop_process", lambda _process: None)
    monkeypatch.setattr(module.time, "sleep", lambda _seconds: None)
    monkeypatch.setattr(
        sys,
        "argv",
        ["doctor_session_test.py", "--binary", "capsem", "--assets", "assets"],
    )

    with pytest.raises(SystemExit) as exit_info:
        module.main()

    assert exit_info.value.code == 0
    assert [args[1] if args[0] != "curl" else "stop" for args in calls] == [
        "create",
        "exec",
        "stop",
        "delete",
    ]

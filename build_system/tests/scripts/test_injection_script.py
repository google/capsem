import importlib.util
import subprocess
from pathlib import Path


def load_injection_script():
    script_path = (
        Path(__file__).resolve().parents[1] / "helpers" / "injection_test.py"
    )
    spec = importlib.util.spec_from_file_location("capsem_injection_test", script_path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_injection_scenario_runs_in_an_isolated_home(monkeypatch, tmp_path):
    module = load_injection_script()
    captured = {}
    shared_run_dir = tmp_path / "gate-run"
    shared_run_dir.mkdir()
    (shared_run_dir / "service.explicitly-stopped").write_text("stopped\n")
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(shared_run_dir))

    def fake_run(args, env, capture_output, text, timeout):
        captured["args"] = args
        captured["env"] = env
        captured["capture_output"] = capture_output
        captured["text"] = text
        captured["timeout"] = timeout
        return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    results = module.Results()
    module.run_scenario(
        "cache/target/cargo/debug/capsem",
        "assets",
        {
            "name": "proof",
            "description": "proof",
            "settings_toml": "[settings]\n",
            "corp_toml": None,
        },
        results,
    )

    assert results.success
    assert captured["env"]["CAPSEM_ASSETS_DIR"] == "assets"
    assert captured["env"]["CAPSEM_HOME"].startswith("/tmp/capsem-injection-proof-home-")
    assert captured["env"]["CAPSEM_RUN_DIR"] == str(
        Path(captured["env"]["CAPSEM_HOME"]) / "run"
    )
    assert captured["env"]["CAPSEM_RUN_DIR"] != str(shared_run_dir)
    assert captured["args"] == [
        "cache/target/cargo/debug/capsem",
        "run",
        "capsem-doctor -k injection",
    ]

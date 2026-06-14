import importlib.util
import subprocess
from pathlib import Path


def load_injection_script():
    script_path = Path(__file__).resolve().parents[1] / "scripts" / "injection_test.py"
    spec = importlib.util.spec_from_file_location("capsem_injection_test", script_path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def test_injection_scenario_uses_materialized_profiles_dir(monkeypatch, tmp_path):
    module = load_injection_script()
    captured = {}

    def fake_run(args, env, capture_output, text, timeout):
        captured["args"] = args
        captured["env"] = env
        captured["capture_output"] = capture_output
        captured["text"] = text
        captured["timeout"] = timeout
        return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    results = module.Results()
    profiles_dir = tmp_path / "target" / "config" / "profiles"
    module.run_scenario(
        "target/debug/capsem",
        "assets",
        str(profiles_dir),
        {
            "name": "proof",
            "description": "proof",
            "settings_toml": "[settings]\n",
            "corp_toml": None,
        },
        results,
    )

    assert results.success
    assert captured["env"]["CAPSEM_PROFILES_DIR"] == str(profiles_dir)
    assert captured["env"]["CAPSEM_HOME"] != str(profiles_dir)
    assert captured["env"]["CAPSEM_HOME"].startswith("/tmp/capsem-injection-proof-home-")
    assert captured["args"] == ["target/debug/capsem", "run", "capsem-doctor -k injection"]


def test_default_materialized_profiles_dir_points_at_target_config_profiles():
    module = load_injection_script()

    assert module.default_materialized_profiles_dir().endswith("target/config/profiles")

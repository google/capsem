from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace

import pytest
from helpers.workflow_contract import emitted_assignment_names, workflow_step

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "replay-release-lane.py"
SPEC = importlib.util.spec_from_file_location("replay_release_lane", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
REPLAY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REPLAY)


def _cohort(tmp_path: Path) -> dict[str, str]:
    content = (tmp_path / "content").resolve()
    return {
        "package": str(tmp_path / "Capsem_0.6.0_amd64.deb"),
        "inputs": str(tmp_path / "inputs"),
        "content_root": str(content),
        "before_manifest": str(tmp_path / "before" / "manifest.json"),
        "manifest": str(tmp_path / "after" / "manifest.json"),
        "before_profile_inputs": str(tmp_path / "before"),
    }


def test_binary_replay_uses_fabricated_content_and_exact_workflow_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CAPSEM_RELEASE_PROFILE", "stale-profile")
    monkeypatch.setenv("CAPSEM_RELEASE_BEFORE_PACKAGE", "/stale/package.deb")
    cohort = _cohort(tmp_path)
    args = SimpleNamespace(lane="binaries", channel="stable", activation_ready="false")

    command, environment = REPLAY.qualification_command(args, cohort)

    assert command == ["just", "qualify-binaries", cohort["content_root"]]
    assert environment["CAPSEM_RELEASE_BEFORE_PACKAGE"] == ""
    assert "CAPSEM_RELEASE_PROFILE" not in environment
    assert environment["CAPSEM_TEST_ASSETS_DIR"] == f"{cohort['content_root']}/assets"
    assert environment["CAPSEM_TEST_CONFIG_ROOT"] == f"{cohort['content_root']}/target/config"

    activation = workflow_step(
        ROOT / ".github" / "workflows" / "release.yaml",
        "test-binary-pairing",
        "Activate exact candidate package binaries for functional tests",
    )
    workflow_environment = emitted_assignment_names(
        str(activation["run"]), origin="release.yaml:test-binary-pairing:activate"
    )
    replay_environment = {
        name
        for name in environment
        if name.startswith("CAPSEM_RELEASE_")
        or name in REPLAY._TEST_SELECTION_ENV
    }
    assert replay_environment == workflow_environment


def test_asset_replay_refuses_to_invent_an_activation_ready_pairing(tmp_path: Path) -> None:
    args = SimpleNamespace(
        lane="assets", channel="nightly", profile="code", activation_ready="true"
    )

    with pytest.raises(SystemExit, match="real public-before package cohort"):
        REPLAY.qualification_command(args, _cohort(tmp_path))


def test_cold_asset_replay_uses_fabricated_content_root(tmp_path: Path) -> None:
    cohort = _cohort(tmp_path)
    args = SimpleNamespace(
        lane="assets", channel="stable", profile="code", activation_ready="false"
    )

    command, environment = REPLAY.qualification_command(args, cohort)

    assert command == [
        "just",
        "qualify-assets",
        cohort["inputs"],
        "code",
        cohort["content_root"],
        "false",
    ]
    assert not any(name.startswith("CAPSEM_RELEASE_") for name in environment)

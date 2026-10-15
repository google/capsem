"""The candidate is read from its own layout; its manifest can only add."""

import hashlib
import json

import pytest

from tests.qualification import candidate


def _layout(root, config):
    blobs = root / "blobs" / "sha256"
    blobs.mkdir(parents=True)

    def put(data):
        digest = hashlib.sha256(data).hexdigest()
        (blobs / digest).write_bytes(data)
        return "sha256:" + digest

    image = put(json.dumps({"os": "linux", "architecture": "arm64", "config": config}).encode())
    manifest = put(json.dumps({"config": {"digest": image}, "layers": []}).encode())
    (root / "index.json").write_text(json.dumps({"manifests": [{"digest": manifest}]}))
    return manifest


def test_the_candidate_is_what_its_config_says(tmp_path):
    digest = _layout(
        tmp_path,
        {"User": "capsem", "WorkingDir": "/workspace", "Entrypoint": ["/init"], "Cmd": ["bash"]},
    )
    read = candidate.read("dev", tmp_path)
    assert read.digest == digest
    assert (read.user, read.working_dir, read.command) == ("capsem", "/workspace", ["/init", "bash"])


def test_an_image_with_no_user_or_directory_runs_as_root_in_root(tmp_path):
    _layout(tmp_path, {"Cmd": ["sleep", "infinity"]})
    read = candidate.read("unlisted-image", tmp_path)
    assert (read.user, read.working_dir) == ("0", "/")
    assert read.capabilities == frozenset()


def test_half_a_candidate_is_refused(monkeypatch):
    monkeypatch.setenv(candidate.NAME_ENV, "dev")
    monkeypatch.delenv(candidate.LAYOUT_ENV, raising=False)
    with pytest.raises(RuntimeError, match="both"):
        candidate.resolve()


def test_capability_selection_does_not_require_built_image_bytes(tmp_path, monkeypatch):
    image = tmp_path / "images" / "agent"
    image.mkdir(parents=True)
    (image / "qualify.toml").write_text('capabilities = ["agent", "ollama"]\n')
    monkeypatch.setattr(candidate, "ROOT", tmp_path)
    monkeypatch.setenv(candidate.NAME_ENV, "agent")
    monkeypatch.setenv(candidate.LAYOUT_ENV, str(tmp_path / "missing-layout"))
    assert candidate.selected_capabilities() == frozenset({"agent", "ollama"})
    with pytest.raises(FileNotFoundError):
        candidate.resolve()


def test_half_a_candidate_is_also_refused_during_capability_selection(monkeypatch):
    monkeypatch.setenv(candidate.NAME_ENV, "dev")
    monkeypatch.delenv(candidate.LAYOUT_ENV, raising=False)
    with pytest.raises(RuntimeError, match="both"):
        candidate.selected_capabilities()


def test_a_manifest_may_only_declare_capabilities_and_expectations(tmp_path, monkeypatch):
    image = tmp_path / "images" / "agent"
    image.mkdir(parents=True)
    (image / "qualify.toml").write_text('capabilities = ["agent"]\nentrypoint = ["sh"]\n')
    monkeypatch.setattr(candidate, "ROOT", tmp_path)
    with pytest.raises(ValueError, match="entrypoint"):
        candidate.declared_capabilities("agent")
    (image / "qualify.toml").write_text('capabilities = ["agent", "ollama"]\n')
    assert candidate.declared_capabilities("agent") == frozenset({"agent", "ollama"})

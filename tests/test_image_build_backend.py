"""Contract tests for the private capsem-admin image build backend."""

from __future__ import annotations

from pathlib import Path

from capsem.builder import image_build_backend


def test_private_backend_loads_guest_config_and_delegates_to_build_image(
    monkeypatch,
    tmp_path: Path,
) -> None:
    guest_dir = tmp_path / "guest"
    output_dir = tmp_path / "out"
    guest_dir.mkdir()
    output_dir.mkdir()
    loaded_config = object()
    calls: dict[str, object] = {}
    repo_root = tmp_path / "repo"
    repo_root.mkdir()

    monkeypatch.chdir(repo_root)
    def fake_load_guest_config(path: Path) -> object:
        calls["loaded_path"] = path
        return loaded_config

    monkeypatch.setattr(image_build_backend, "load_guest_config", fake_load_guest_config)

    def fake_build_image(config, arch, *, template, output_dir, repo_root):
        calls["config"] = config
        calls["arch"] = arch
        calls["template"] = template
        calls["output_dir"] = output_dir
        calls["repo_root"] = repo_root

    monkeypatch.setattr(image_build_backend, "build_image", fake_build_image)
    monkeypatch.setattr(
        "sys.argv",
        [
            "python -m capsem.builder.image_build_backend",
            str(guest_dir),
            "--arch",
            "arm64",
            "--template",
            "rootfs",
            "--output",
            str(output_dir),
        ],
    )

    image_build_backend.main()

    assert calls == {
        "loaded_path": guest_dir,
        "config": loaded_config,
        "arch": "arm64",
        "template": "rootfs",
        "output_dir": output_dir,
        "repo_root": repo_root,
    }

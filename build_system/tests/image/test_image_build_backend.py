"""Contract tests for the private capsem-admin image build backend."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.image import image_build_backend
from capsem_builder.image.assetdependencies import AssetDependencyImage


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
            "python -m capsem_builder.image.image_build_backend",
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


def test_private_backend_materializes_dependencies_and_prints_exact_image(
    monkeypatch,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    guest_dir = tmp_path / "guest"
    guest_dir.mkdir()
    loaded_config = object()
    calls: dict[str, object] = {}
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(
        image_build_backend,
        "load_guest_config",
        lambda path: loaded_config,
    )

    def fake_materialize(config, arch, *, template, repo_root) -> AssetDependencyImage:
        calls.update(
            config=config,
            arch=arch,
            template=template,
            repo_root=repo_root,
        )
        return AssetDependencyImage(
            reference="capsem-kernel-dependencies-arm64:fixture",
            image_id="sha256:materialized",
        )

    monkeypatch.setattr(
        image_build_backend,
        "materialize_asset_dependencies",
        fake_materialize,
    )
    monkeypatch.setattr(
        "sys.argv",
        [
            "python -m capsem_builder.image.image_build_backend",
            str(guest_dir),
            "--arch",
            "arm64",
            "--template",
            "kernel",
            "--materialize-dependencies",
        ],
    )

    image_build_backend.main()

    assert calls == {
        "config": loaded_config,
        "arch": "arm64",
        "template": "kernel",
        "repo_root": tmp_path,
    }
    assert capsys.readouterr().out == "sha256:materialized\n"


def test_private_backend_requires_dependencies_through_detected_runtime(
    monkeypatch,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    guest_dir = tmp_path / "guest"
    guest_dir.mkdir()
    loaded_config = object()
    calls: dict[str, object] = {}
    monkeypatch.setattr(
        image_build_backend,
        "load_guest_config",
        lambda path: loaded_config,
    )
    monkeypatch.setattr(image_build_backend, "detect_runtime", lambda: "docker")

    def fake_require(runtime, config, arch, template) -> AssetDependencyImage:
        calls.update(
            runtime=runtime,
            config=config,
            arch=arch,
            template=template,
        )
        return AssetDependencyImage(
            reference="capsem-rootfs-dependencies-x86_64:fixture",
            image_id="sha256:required",
        )

    monkeypatch.setattr(
        image_build_backend,
        "require_asset_dependencies",
        fake_require,
    )
    monkeypatch.setattr(
        "sys.argv",
        [
            "python -m capsem_builder.image.image_build_backend",
            str(guest_dir),
            "--arch",
            "x86_64",
            "--template",
            "rootfs",
            "--require-dependencies",
        ],
    )

    image_build_backend.main()

    assert calls == {
        "runtime": "docker",
        "config": loaded_config,
        "arch": "x86_64",
        "template": "rootfs",
    }
    assert capsys.readouterr().out == "sha256:required\n"


def test_private_backend_refuses_ambiguous_dependency_operation(
    monkeypatch,
    tmp_path: Path,
) -> None:
    guest_dir = tmp_path / "guest"
    guest_dir.mkdir()
    monkeypatch.setattr(image_build_backend, "load_guest_config", lambda path: object())
    monkeypatch.setattr(
        "sys.argv",
        [
            "python -m capsem_builder.image.image_build_backend",
            str(guest_dir),
            "--arch",
            "x86_64",
            "--template",
            "rootfs",
            "--materialize-dependencies",
            "--require-dependencies",
        ],
    )

    with pytest.raises(SystemExit, match="2"):
        image_build_backend.main()

"""Exact private builder-image registry behavior."""

from __future__ import annotations

import json
import subprocess
from argparse import Namespace
from dataclasses import replace
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import privateimages
from capsem_builder.gate.docker import Docker
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.privateimagecache import (
    SOURCE_LABEL,
    PrivateImage,
    publish,
    try_import,
    try_import_previous,
)
from helpers.gate import RECORDED_IMAGE_ID, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
HOST_PLATFORM = CONFIG.host_arch().docker_platform
HOST_PLATFORM_TAG = HOST_PLATFORM.replace("/", "-")
HOST_PACKAGE = f"google/capsem-ci/capsem-host-builder-{HOST_PLATFORM_TAG}"
HOST_REMOTE = f"ghcr.io/{HOST_PACKAGE}:input-key"
IMAGE = PrivateImage(
    local="capsem-host-builder:latest",
    platform=HOST_PLATFORM,
    generation="input-key",
    identity_label="org.capsem.host-builder.input-key",
    identity="input-key",
)


def _runner(*, failures: tuple[str, ...] = ()) -> RecordingRunner:
    return RecordingRunner(
        PROJECT_ROOT,
        replies={
            IMAGE.identity_label: IMAGE.identity,
            SOURCE_LABEL: CONFIG.private_images.source,
        },
        failures=failures,
    )


def test_private_reference_is_platform_scoped_for_three_generation_retention() -> None:
    assert IMAGE.remote(CONFIG.private_images.registry) == HOST_REMOTE
    assert IMAGE.package(CONFIG.private_images.registry) == HOST_PACKAGE


def test_failed_private_push_keeps_the_registry_diagnostic(tmp_path, monkeypatch, capfd) -> None:
    from capsem_builder.gate.proc import Runner

    docker = tmp_path / "docker"
    docker.write_text(
        "#!/bin/sh\nprintf '%s\\n' 'denied: package publication permission missing' >&2\nexit 1\n"
    )
    docker.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path))
    assert not Docker(Runner(PROJECT_ROOT)).try_push("ghcr.io/example/private:exact-inputs")
    assert "denied: package publication permission missing" in capfd.readouterr().err


def test_every_private_helper_dockerfile_links_the_owning_repository() -> None:
    paths = (
        "build_system/docker/Dockerfile.host-builder",
        "build_system/docker/Dockerfile.linux-rust-base",
        "build_system/docker/Dockerfile.install-builder",
        "build_system/docker/Dockerfile.package-builder",
        "build_system/docker/Dockerfile.guest-rust-builder",
        "build_system/docker/Dockerfile.asset-tools",
        "config/docker/Dockerfile.rootfs-dependencies.j2",
        "config/docker/Dockerfile.kernel-dependencies.j2",
    )
    label = f'LABEL {SOURCE_LABEL}="{CONFIG.private_images.source}"'

    for relative in paths:
        assert label in (PROJECT_ROOT / relative).read_text(encoding="utf-8"), relative


def test_private_inventory_covers_every_network_open_helper() -> None:
    runner = _runner()
    images = privateimages.inventory(CONFIG, Docker(runner))

    assert len(images) == 10
    repositories = {image.local.rsplit(":", 1)[0] for image in images}
    assert repositories == {
        "capsem-host-builder",
        "capsem-linux-rust-base",
        "capsem-install-builder",
        "capsem-package-builder-arm64",
        "capsem-package-builder-x86_64",
        "capsem-guest-rust-arm64",
        "capsem-guest-rust-x86_64",
        f"capsem-asset-tools-{CONFIG.host_arch().name}",
        f"capsem-kernel-dependencies-{CONFIG.host_arch().name}",
        f"capsem-rootfs-dependencies-{CONFIG.host_arch().name}",
    }


def test_both_hosts_use_exactly_the_twenty_approved_packages(monkeypatch) -> None:
    packages: set[str] = set()
    for host in ("arm64", "x86_64"):
        monkeypatch.setattr("platform.machine", lambda host=host: host)
        packages.update(
            image.package(CONFIG.private_images.registry)
            for image in privateimages.inventory(CONFIG, Docker(_runner()))
        )

    approved = CONFIG.private_images.approved_packages
    assert len(approved) == len(set(approved)) == 20
    assert packages == set(approved)


def test_private_image_plan_publishes_only_after_every_materializer() -> None:
    command = privateimages.PrivateImagesCommand(
        _runner(),
        Namespace(publish=True, sandbox=None),
    )
    plan = command.plan()
    plan.validate(CONFIG)

    assert plan.after_of("private.publish") == {
        "private.linux-rust-base",
        "private.product-helpers",
        "private.asset-dependencies",
    }


def test_private_pull_is_the_normal_path_and_falls_back_without_a_generation() -> None:
    missing = _runner(failures=("docker pull",))
    assert not try_import(missing, CONFIG, IMAGE)
    assert missing.rendered == [
        f"docker pull --platform {HOST_PLATFORM} {HOST_REMOTE}"
    ]


def test_private_pull_retags_and_validates_both_identity_labels() -> None:
    runner = _runner()

    assert try_import(runner, CONFIG, IMAGE)
    assert runner.index_of(rf"docker pull --platform {HOST_PLATFORM}") < runner.index_of(
        r"docker tag .* capsem-host-builder:latest"
    )
    assert any(IMAGE.identity_label in command for command in runner.rendered)
    assert any(SOURCE_LABEL in command for command in runner.rendered)


def test_prior_manifest_generation_is_imported_as_a_validated_build_cache(
    tmp_path: Path,
) -> None:
    manifest = tmp_path / "private-images.json"
    repository = "capsem-package-builder-arm64"
    generation = "previous"
    local = f"{repository}:{generation}"
    image = PrivateImage(
        local=local,
        platform=HOST_PLATFORM,
        generation=generation,
        identity_label="org.capsem.package-builder.input-key",
        identity=local,
    )
    manifest.write_text(
        json.dumps(
            {
                "images": [
                    {
                        "generation": generation,
                        "package": image.package(CONFIG.private_images.registry),
                        "platform": image.platform,
                        "reference": image.remote(CONFIG.private_images.registry),
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    config = CONFIG.model_copy(
        update={
            "private_images": CONFIG.private_images.model_copy(update={"manifest": str(manifest)})
        }
    )
    runner = RecordingRunner(
        PROJECT_ROOT,
        replies={SOURCE_LABEL: CONFIG.private_images.source},
    )

    found = try_import_previous(
        runner,
        config,
        repository=repository,
        platform=image.platform,
        identity_label=image.identity_label,
        current_generation="current",
    )

    assert found == local
    assert any(
        image.remote(CONFIG.private_images.registry) in command for command in runner.rendered
    )
    assert any(image.identity_label in command for command in runner.rendered)


def test_publish_is_append_only_and_writes_retention_manifest(tmp_path: Path) -> None:
    runner = _runner(failures=("docker manifest inspect",))
    manifest = tmp_path / "private-images.json"

    publish(runner, CONFIG, (IMAGE,), manifest)

    runner.assert_order(
        r"docker manifest inspect .*:input-key",
        r"docker tag capsem-host-builder:latest .*:input-key",
        r"docker push .*:input-key",
    )
    assert json.loads(manifest.read_text()) == {
        "keep_generations": 3,
        "images": [
            {
                "generation": "input-key",
                "package": HOST_PACKAGE,
                "platform": HOST_PLATFORM,
                "reference": HOST_REMOTE,
            }
        ],
    }
    assert RECORDED_IMAGE_ID in runner.capture(
        ["docker", "image", "inspect", "--format", "{{.Id}}", IMAGE.local]
    )


@pytest.mark.parametrize(
    "unexpected",
    [
        replace(IMAGE, local="capsem-another-helper:input-key"),
        replace(IMAGE, platform="linux/riscv64"),
    ],
)
def test_unapproved_package_aborts_entire_batch_before_registry_access(
    tmp_path: Path,
    unexpected: PrivateImage,
) -> None:
    runner = _runner(failures=("docker manifest inspect",))
    manifest = tmp_path / "private-images.json"
    manifest.write_text("previous manifest\n")

    with pytest.raises(GateError, match="unapproved private image package"):
        publish(runner, CONFIG, (IMAGE, unexpected), manifest)

    assert runner.rendered == []
    assert manifest.read_text() == "previous manifest\n"


def test_publish_retries_a_transient_final_manifest_denial(tmp_path: Path) -> None:
    class TransientPushRunner(RecordingRunner):
        pushes = 0

        def execute(self, command):
            if command.argv[:2] == ("docker", "push"):
                self.commands.append(command)
                self.pushes += 1
                return subprocess.CompletedProcess(
                    args=list(command.argv),
                    returncode=1 if self.pushes == 1 else 0,
                    stdout="",
                    stderr="denied",
                )
            return super().execute(command)

    runner = TransientPushRunner(
        PROJECT_ROOT,
        replies={
            IMAGE.identity_label: IMAGE.identity,
            SOURCE_LABEL: CONFIG.private_images.source,
        },
        failures=("docker manifest inspect",),
    )

    publish(runner, CONFIG, (IMAGE,), tmp_path / "private-images.json")

    assert runner.pushes == 2
    assert runner.notes == [
        "private image manifest publication failed (attempt 1/3): "
        f"{HOST_REMOTE}"
    ]

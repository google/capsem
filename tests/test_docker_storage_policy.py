"""Contracts for the release-gate storage and failure-evidence policy."""

from __future__ import annotations

import json
import importlib.util
from pathlib import Path
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parent.parent
POLICY_PATH = ROOT / "config" / "storage-policy.toml"
POLICY_SCRIPT = ROOT / "scripts" / "docker-storage-policy.py"


def load_policy_module():
    spec = importlib.util.spec_from_file_location("docker_storage_policy", POLICY_SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_policy() -> dict:
    with POLICY_PATH.open("rb") as stream:
        return tomllib.load(stream)


def test_policy_has_one_warm_cache_and_capacity_model() -> None:
    policy = load_policy()

    assert policy["version"] == 1
    assert policy["docker"]["minimum_disk_gib"] == 96
    assert policy["docker"]["recommended_disk_gib"] == 128
    assert policy["docker"]["buildkit_keep_gib"] == 24
    assert policy["docker"]["minimum_free_gib"] == 24
    assert set(policy["rails"]) == {
        "default",
        "assets",
        "package",
        "install-preflight",
        "install",
    }
    for rail in policy["rails"].values():
        assert rail["minimum_free_gib"] >= 24
        assert rail["buildkit_keep_gib"] >= 24
        assert rail["linked_keep_gib"] >= 4


def test_policy_declares_last_consumers_before_release_boundaries() -> None:
    policy = load_policy()
    resources = policy["resources"]

    assert resources["capsem-host-builder"]["last_consumer"] == "package-x86_64"
    assert resources["capsem-host-builder"]["release_boundary"] == "after-packages"
    assert resources["capsem-linux-rust-target"]["last_consumer"] == "linux-rust"
    assert resources["capsem-linux-rust-target"]["release_boundary"] == "after-linux-rust"
    assert resources["capsem-agent-target-arm64"]["last_consumer"] == "assets"
    assert resources["capsem-agent-target-arm64"]["release_boundary"] == "after-assets"
    assert resources["capsem-host-target-arm64"]["release_boundary"] == "after-package-arm64"
    assert resources["capsem-host-target-x86_64"]["release_boundary"] == "after-package-x86_64"
    assert resources["capsem-install-target"]["maximum_gib"] == 25
    assert resources["capsem-install-target"]["retention"] == "working"
    assert resources["capsem-cargo-registry"]["retention"] == "cache"
    assert resources["capsem-install-frontend-node-modules"]["retention"] == "cache"
    assert resources["capsem-linux-python-venv"]["retention"] == "obsolete"


def test_policy_cli_reports_resolved_rail_without_docker() -> None:
    result = subprocess.run(
        [
            sys.executable,
            str(POLICY_SCRIPT),
            "show",
            "--rail",
            "assets",
            "--offline",
            "--json",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    report = json.loads(result.stdout)

    assert report["rail"] == "assets"
    assert report["limits"]["buildkit_keep_gib"] == 24
    assert report["limits"]["minimum_free_gib"] == 24
    assert report["docker"]["minimum_disk_gib"] == 96
    assert report["docker"]["recommended_disk_gib"] == 128
    assert report["resources"]["capsem-host-builder"]["last_consumer"] == "package-x86_64"


def test_justfile_uses_named_rails_and_keeps_builder_until_packages_finish() -> None:
    justfile = (ROOT / "justfile").read_text()

    assert "CAPSEM_DOCKER_CACHE_KEEP_GB=" not in justfile
    assert 'scripts/ensure-docker-space.sh" assets' in justfile
    assert 'scripts/ensure-docker-space.sh" package' in justfile
    assert 'scripts/ensure-docker-space.sh" install-preflight' in justfile
    assert 'scripts/ensure-docker-space.sh" install' in justfile

    arm64 = justfile.index("just _cross-compile arm64")
    x86_64 = justfile.index("just _cross-compile x86_64")
    release = justfile.index("just _release-completed-buildkit-graph", arm64)
    assert arm64 < x86_64 < release

    assert "docker buildx prune --all --force --reserved-space 2GB" not in justfile
    assert "docker image rm rust:slim-bookworm" not in justfile
    assert '[ "$VOLUME_GB" -gt 25 ]' not in justfile
    assert "resource --name capsem-install-target --field maximum_gib" not in justfile
    assert "docker builder prune" not in justfile
    assert "docker volume rm" not in justfile
    assert "docker-storage-policy.py gc" in justfile
    assert "--boundary after-package-arm64" in justfile
    assert "--boundary after-package-x86_64" in justfile


def test_shell_space_guard_is_only_a_python_controller_entrypoint() -> None:
    guard = (ROOT / "scripts" / "ensure-docker-space.sh").read_text()

    assert "docker " not in guard
    assert 'docker-storage-policy.py" enforce' in guard


def test_size_parser_and_system_df_are_byte_exact() -> None:
    policy_module = load_policy_module()
    rows = policy_module.parse_system_df(
        "\n".join(
            [
                '{"Type":"Images","TotalCount":"8","Active":"0","Size":"13.43GB","Reclaimable":"7.385GB (54%)"}',
                '{"Type":"Build Cache","TotalCount":"91","Active":"0","Size":"9.906GB","Reclaimable":"6.877GB"}',
            ]
        )
    )

    assert rows["images"]["size_bytes"] == 13_430_000_000
    assert rows["images"]["reclaimable_bytes"] == 7_385_000_000
    assert rows["build_cache"]["size_bytes"] == 9_906_000_000
    assert rows["build_cache"]["reclaimable_bytes"] == 6_877_000_000


def test_offline_snapshot_reports_every_managed_resource_and_decision() -> None:
    result = subprocess.run(
        [
            sys.executable,
            str(POLICY_SCRIPT),
            "snapshot",
            "--rail",
            "default",
            "--label",
            "contract",
            "--offline",
            "--json",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    report = json.loads(result.stdout)

    assert report["event"] == "snapshot"
    assert report["label"] == "contract"
    assert report["runtime"]["available"] is False
    resources = report["resources"]
    assert resources["capsem-host-target-arm64"]["decision"] == ("release-after-package-arm64")
    assert resources["capsem-cargo-registry"]["decision"] == "retain-cache"
    assert resources["capsem-linux-python-venv"]["decision"] == "delete-obsolete"


def test_candidate_failure_captures_storage_and_asset_logs_before_next_cleanup() -> None:
    justfile = (ROOT / "justfile").read_text()
    test_recipe = justfile[justfile.index("test:\n") : justfile.index("\n_test-candidate:")]

    assert "capture-failure" in test_recipe
    assert test_recipe.index("trap ") < test_recipe.index("scripts/with-gate-colima.sh")


def test_failure_capture_has_a_side_effect_free_offline_mode(tmp_path: Path) -> None:
    policy_text = POLICY_PATH.read_text().replace(
        'root = "test-artifacts"', f'root = "{tmp_path.as_posix()}"'
    )
    policy_path = tmp_path / "policy.toml"
    policy_path.write_text(policy_text)

    result = subprocess.run(
        [
            sys.executable,
            str(POLICY_SCRIPT),
            "--policy",
            str(policy_path),
            "capture-failure",
            "--rail",
            "assets",
            "--label",
            "dry-run",
            "--offline",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    capture_dir = next(tmp_path.glob("*-storage-dry-run"))

    assert "preserved release-gate storage evidence" in result.stdout
    assert json.loads((capture_dir / "policy.json").read_text())["rail"] == "assets"
    assert "offline capture" in (capture_dir / "docker-system-df.txt").read_text()


def test_debug_artifact_retention_is_bounded_but_keeps_recent_failures() -> None:
    debug = load_policy()["debug_artifacts"]

    assert debug["minimum_runs"] >= 5
    assert debug["maximum_runs"] >= debug["minimum_runs"]
    assert debug["maximum_age_days"] >= 14
    assert debug["maximum_total_gib"] >= 8
    assert debug["maximum_file_mib"] <= 25
    assert "rootfs.img" in debug["skip_names"]


def test_bootstrap_and_doctor_share_the_recommended_disk_policy() -> None:
    bootstrap = (ROOT / "bootstrap.sh").read_text()
    doctor = (ROOT / "scripts" / "doctor-macos.sh").read_text()

    assert "config/storage-policy.toml" in bootstrap
    assert '--disk "$DOCKER_DISK_GIB"' in bootstrap
    assert "recommended_docker_disk_gib" in doctor
    assert "minimum_docker_disk_gib" in doctor
    assert "Colima Docker disk:" in doctor
    assert "--disk ${recommended_disk_gib}" in doctor

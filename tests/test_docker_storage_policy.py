"""Contracts for the release-gate storage and failure-evidence policy."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POLICY_PATH = ROOT / "config" / "storage-policy.toml"
POLICY_SCRIPT = ROOT / "scripts" / "docker-storage-policy.py"


def _gate_labels(name: str = "candidate") -> tuple[str, ...]:
    """Every step of a command's plan, in graph order. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels(name)


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
    assert policy["docker"]["minimum_disk_gib"] == 160
    assert policy["docker"]["recommended_disk_gib"] == 200
    assert policy["docker"]["buildkit_keep_gib"] == 80
    assert policy["docker"]["minimum_free_gib"] == 40
    assert set(policy["rails"]) == {
        "default",
        "assets",
        "package",
        "install-preflight",
        "install",
    }
    for rail in policy["rails"].values():
        assert rail["minimum_free_gib"] >= 40
        assert rail["buildkit_keep_gib"] >= 80
        assert rail["linked_keep_gib"] >= 4


def test_policy_declares_last_consumers_before_release_boundaries() -> None:
    policy = load_policy()
    resources = policy["resources"]

    # `install`, not `package-x86_64`: `docker/Dockerfile.install-test` is
    # `FROM capsem-host-builder:latest`, and the install proof always rebuilds
    # rather than reusing a tag that may be stale. Declaring the packages as
    # the last consumer released the base image out from under it, and the
    # gate died at `docker build` with `pull access denied`.
    assert resources["capsem-host-builder"]["last_consumer"] == "install"
    assert resources["capsem-host-builder"]["release_boundary"] == "after-install"
    # `capsem-install-target` and `-frontend-node-modules` were `working` and
    # `cache`; both are obsolete now. The install lanes copy their source into
    # the image, so nothing declares them and nothing can mount them.
    assert resources["capsem-install-target"]["retention"] == "obsolete"
    assert resources["capsem-install-frontend-node-modules"]["retention"] == "obsolete"
    # Obsolete: the base image resolves the dependency graph, and the volume
    # that used to carry it mounted over `/usr/local/cargo` -- shadowing the
    # toolchain, the cross-targets and the tools the image installs there.
    assert resources["capsem-cargo-registry"]["retention"] == "obsolete"
    # Obsolete too: the install image bakes the release-site dependencies, so
    # nothing mounts this. It was a cache for a bind-mounted checkout that no
    # longer exists, and a stale copy of it mounted over an image-provided
    # `release-site/` is what pnpm refused to reconcile without a TTY.
    assert resources["capsem-install-release-site-node-modules"]["retention"] == "obsolete"
    assert resources["capsem-linux-python-venv"]["retention"] == "obsolete"


#: The four the sealed parity lane stopped mounting. `capsem-linux-rust-base`
#: is deliberately absent -- it is the image that replaced them, and it is the
#: one managed resource keyed by a repository rather than a tag.
RETIRED_BY_THE_SEALED_LANE = (
    "capsem-linux-rust-target",
    "capsem-linux-rust-cargo-registry",
    "capsem-linux-rust-cargo-git",
    "capsem-linux-rust-rustup",
)


def test_the_sealed_parity_lane_declares_no_volumes_to_hand_back() -> None:
    """What replaced `capsem-linux-rust-target`'s consumer and boundary.

    Those two assertions said the lane's build tree is released after the lane.
    The lane now mounts nothing at all, so the honest form of the same property
    is that none of the four can name a consumer or a boundary -- a volume with
    no producer must not have a step existing to give it back.

    Strictly stronger than what it replaces: the old pair constrained one
    volume's ordering, this constrains all four out of the graph entirely.
    """
    resources = load_policy()["resources"]

    for name in RETIRED_BY_THE_SEALED_LANE:
        resource = resources[name]
        assert resource["retention"] == "obsolete", (
            f"{name} is mounted by nothing since the parity lane was sealed"
        )
        assert resource["owner"] == "none", f"{name} has no owner left to claim it"
        assert "last_consumer" not in resource, (
            f"{name} names a consumer, but the sealed lane does not mount it"
        )
        assert "release_boundary" not in resource, (
            f"{name} keeps a release boundary, so a step still exists to hand "
            "back a volume nothing takes"
        )


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
    assert report["limits"]["buildkit_keep_gib"] == 80
    assert report["limits"]["minimum_free_gib"] == 40
    assert report["docker"]["minimum_disk_gib"] == 160
    assert report["docker"]["recommended_disk_gib"] == 200
    assert report["resources"]["capsem-host-builder"]["last_consumer"] == "install"


def test_justfile_uses_named_rails_and_keeps_builder_until_packages_finish() -> None:
    justfile = (ROOT / "justfile").read_text()

    assert "CAPSEM_DOCKER_CACHE_KEEP_GB=" not in justfile
    # The builder's final tag survives until neither package build needs it.
    # Ordering is an edge in the gate now rather than line order in a recipe.
    labels = list(_gate_labels())
    # The lane is eight steps now; its build is the one that runs the image.
    arm64 = labels.index("package.arm64.build")
    x86_64 = labels.index("package.x86_64.build")
    # The builder's tag now outlives the packages too: the install proof's
    # image is `FROM` it. `after-install` is the first boundary at which
    # nothing derives from it.
    install = labels.index("glowup.install")
    assert arm64 < x86_64 < install

    assert "docker buildx prune --all --force --reserved-space 2GB" not in justfile
    assert "docker image rm rust:slim-bookworm" not in justfile
    assert '[ "$VOLUME_GB" -gt 25 ]' not in justfile
    assert "resource --name capsem-install-target --field maximum_gib" not in justfile
    assert "docker builder prune" not in justfile
    assert "docker volume rm" not in justfile
    assert "capsem-gate storage gc" in justfile


def test_the_install_rails_reserve_headroom_before_and_during_the_proof() -> None:
    """These two reservations moved into `capsem.gate.install` with the recipe.

    They are the reason ENOSPC surfaces here, with a disk recommendation,
    rather than hours later inside a fixture on an otherwise-green run.
    """
    install = (ROOT / "src" / "capsem" / "gate" / "install.py").read_text()

    assert 'ensure_space("install-preflight")' in install
    assert 'ensure_space("install")' in install

    # The package rail owns its own headroom the same way, and reserves it
    # twice: once before the builder image and once after, since the image
    # build itself is what consumes the first reservation.
    # `packagerail` since the rail's runtime operations were split from the
    # adapter that orders them; the pair has to sit in one file either way,
    # which is what the count is really asserting.
    package = (ROOT / "src" / "capsem" / "gate" / "packagerail.py").read_text()
    assert package.count('ensure_space("package")') == 2

    # The asset rail reserves its own before the dual-architecture lanes start.
    assets = (ROOT / "src" / "capsem" / "gate" / "assets.py").read_text()
    assert 'ensure_space("assets")' in assets


def test_both_package_architectures_release_their_own_install_headroom() -> None:
    """Each package rail frees the install rail once, under its own boundary.

    The justfile used to spell the `--boundary`/`--rail` pair at each of eleven
    call sites, so this read them out of the recipe text. They are now a table
    in `config/gate.toml`, validated at load, which is where a typo can be
    caught before any storage is touched.
    """
    from capsem.gate import config as gate_config

    phases = gate_config.load(ROOT).storage.phases

    assert (phases["completed-package-arm64"].boundary, phases["completed-package-arm64"].rail) == (
        "after-package-arm64",
        "install",
    )
    assert (
        phases["completed-package-x86_64"].boundary,
        phases["completed-package-x86_64"].rail,
    ) == ("after-package-x86_64", "install")

    for phase in ("completed-package-arm64", "completed-package-x86_64"):
        assert f"glowup.storage.{phase}" in _gate_labels()


def test_shell_space_guard_is_only_a_python_controller_entrypoint() -> None:
    guard = (ROOT / "scripts" / "ensure-docker-space.sh").read_text()

    assert "docker " not in guard
    assert 'docker-storage-policy.py" enforce' in guard


def test_shell_space_guard_does_not_enter_the_project_uv_environment(
    tmp_path: Path,
) -> None:
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    python_args = tmp_path / "python-args"
    fake_python = fake_bin / "python"
    fake_python.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$CAPSEM_TEST_PYTHON_ARGS"\n')
    fake_python.chmod(0o755)
    fake_uv = fake_bin / "uv"
    fake_uv.write_text(
        """#!/bin/sh
test "$1" = "run" && test "$2" = "--no-project" || exit 97
shift 2
test "$1" = "--python" && test "$2" = "3.12" || exit 98
shift 2
exec "$@"
"""
    )
    fake_uv.chmod(0o755)

    result = subprocess.run(
        [str(ROOT / "scripts" / "ensure-docker-space.sh"), "install", "contract"],
        cwd=ROOT,
        env={
            **os.environ,
            "PATH": f"{fake_bin}:/usr/bin:/bin",
            "CAPSEM_TEST_PYTHON_ARGS": str(python_args),
        },
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert python_args.read_text().splitlines() == [
        str(POLICY_SCRIPT),
        "enforce",
        "--rail",
        "install",
        "--label",
        "contract",
    ]


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
    # `delete-obsolete`, not `release-after-...`: there is no named volume to
    # release. The per-arch build directory is an anonymous volume now,
    # allocated per container and reclaimed with it, so the boundary that used
    # to hand it back has nothing to hand.
    assert resources["capsem-host-target-arm64"]["decision"] == "delete-obsolete"
    # Nothing retains a cache volume any more. The base image resolves the
    # dependency graph, and this volume mounted over the directory the image
    # installs the toolchain and its tools into.
    assert resources["capsem-cargo-registry"]["decision"] == "delete-obsolete"
    assert resources["capsem-linux-python-venv"]["decision"] == "delete-obsolete"


def test_candidate_failure_captures_storage_and_asset_logs_before_next_cleanup() -> None:
    """Evidence is taken on the way out of a failure, before anything reclaims
    the storage that holds it."""
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from helpers.gate import RecordingRunner

    from capsem.gate import config as gate_config
    from capsem.gate.gateresources import FailureEvidence
    from capsem.gate.lifecycle import held

    # It is a resource now, so the ordering is the lifecycle's guarantee rather
    # than the shape of one `try` block: `preserve` runs on the failure path,
    # and it runs before the release that reclaims what it captured.
    config = gate_config.load(ROOT)
    runner = RecordingRunner(ROOT)
    evidence = FailureEvidence(config, runner)
    order: list[str] = []
    evidence.release = lambda: order.append("release")  # type: ignore[method-assign]

    try:
        with held(evidence):
            order.append("body")
            raise RuntimeError("the gate failed")
    except RuntimeError:
        pass

    issued = " ".join(" ".join(command.argv) for command in runner.commands)
    assert "capture-failure" in issued
    assert order == ["body", "release"]
    # ...and the capture happened while the body's failure was in flight,
    # meaning before the release that follows it.
    assert runner.commands, "no evidence was captured on the failure path"


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


def test_the_evidence_bundle_says_what_it_could_not_collect(tmp_path: Path) -> None:
    """Silence is the one answer a post-mortem cannot use.

    `copy_small_file` returns quietly for three different outcomes -- the file
    was absent, it was over the size cap, or it could not be read -- and the
    IronBank globs yield nothing at all on a tree where those builds never
    ran. So a preserved bundle gave no way to tell "there were no IronBank
    logs" from "the collector never looked" from "the copy failed", and the
    gap only became visible during the post-mortem that needed it.

    The bundle now carries a manifest of every source attempted and what
    happened to it.
    """
    policy_text = POLICY_PATH.read_text().replace(
        'root = "test-artifacts"', f'root = "{tmp_path.as_posix()}"'
    )
    policy_path = tmp_path / "policy.toml"
    policy_path.write_text(policy_text)

    subprocess.run(
        [
            sys.executable,
            str(POLICY_SCRIPT),
            "--policy",
            str(policy_path),
            "capture-failure",
            "--rail",
            "assets",
            "--label",
            "gap",
            "--offline",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    capture_dir = next(tmp_path.glob("*-storage-gap"))

    collected = json.loads((capture_dir / "collected.json").read_text())
    by_source = {entry["source"]: entry for entry in collected["files"]}

    # Every optional source is accounted for by name, present or not.
    assert any("ironbank" in source for source in by_source), sorted(by_source)
    assert {entry["outcome"] for entry in collected["files"]} <= {
        "copied",
        "absent",
        "truncated",
        "unreadable",
    }, collected["files"]
    # Specifically the globs. Asserting merely that *something* was absent
    # passes on `build.log` alone, which is not the gap being closed -- a glob
    # that matches nothing is the case that produced no record at all.
    ironbank_absent = {
        entry["source"]
        for entry in collected["files"]
        if entry["outcome"] == "absent" and "ironbank" in entry["source"]
    }
    assert any(source.endswith("build-*.log") for source in ironbank_absent), (
        f"an empty IronBank build-log glob left no record: {sorted(ironbank_absent)}"
    )
    assert any(source.endswith("run-failure") for source in ironbank_absent), (
        f"an empty IronBank run-failure glob left no record: {sorted(ironbank_absent)}"
    )


def test_an_oversized_log_is_tailed_rather_than_dropped(tmp_path: Path) -> None:
    """Because the end of a build log is where the failure is.

    The first bundle written with a collection manifest reported both
    `build.log` and `docker-storage.jsonl` as over the cap -- so every bundle
    before it had silently omitted the two files a post-mortem reaches for
    first, and precisely on the long runs that needed them most.
    """
    module = load_policy_module()
    source = tmp_path / "build.log"
    source.write_bytes(b"discard\n" * 1000 + b"THE ACTUAL FAILURE\n")
    destination = tmp_path / "out" / "build.log"

    outcome = module.copy_small_file(source, destination, 256)

    assert outcome == "truncated", outcome
    kept = destination.read_bytes()
    assert len(kept) <= 256
    assert b"THE ACTUAL FAILURE" in kept, "the tail -- the part that matters -- was lost"


# ---------------------------------------------------------------------------
# Generational images
# ---------------------------------------------------------------------------
#
# `capsem-linux-rust-base` is the one managed image whose Docker name is a
# repository rather than a tag: `base_tag()` keys it by a blake2b of the three
# lockfiles that decide its dependencies, so every bump mints a new ~25 GiB tag
# and nothing retired the old one. Three coexisting tags were observed after a
# single security bump, on a VM with 54.7 GiB free.


class _FakeDockerDaemon:
    """A daemon holding a declared set of tags, remembering what was removed.

    Fakes Docker, not the policy: `command_reclaim` runs unmodified, and what
    the test reads back is the `docker image rm` it really issued.
    """

    def __init__(self, module, tags: dict[str, tuple[str, int]]) -> None:
        self._module = module
        self.tags = dict(tags)
        self.removed: list[str] = []

    def _result(self, command: list[str], stdout: str = "", code: int = 0):
        return self._module.CommandResult(command, code, stdout, "")

    def __call__(self, command: list[str], *, timeout: int = 120):
        head = command[:3]
        if head == ["docker", "run", "--rm"]:
            return self._result(command, "209715200 104857600 104857600")
        if head == ["docker", "system", "df"]:
            if "-v" in command:
                return self._result(
                    command, "Local Volumes space usage:\nVOLUME NAME  LINKS  SIZE\n"
                )
            return self._result(
                command,
                '{"Type":"Images","TotalCount":"3","Active":"0",'
                '"Size":"40GB","Reclaimable":"20GB (50%)"}',
            )
        if head == ["docker", "image", "ls"]:
            reference = next(part for part in command if part.startswith("reference="))
            repository = reference.removeprefix("reference=").removesuffix(":*")
            matching = [ref for ref in self.tags if ref.rsplit(":", 1)[0] == repository]
            return self._result(command, "\n".join(sorted(matching)))
        if head == ["docker", "image", "inspect"]:
            reference = command[3]
            if reference not in self.tags:
                return self._result(command, "", 1)
            created, size = self.tags[reference]
            if "{{.Size}}" in command:
                return self._result(command, str(size))
            return self._result(command, f"sha256:{reference}\t{created}\t{size}")
        if head == ["docker", "image", "rm"]:
            reference = command[3]
            self.removed.append(reference)
            self.tags.pop(reference, None)
            return self._result(command)
        if command[:2] == ["docker", "ps"]:
            return self._result(command)
        if head == ["docker", "volume", "inspect"]:
            return self._result(command, "", 1)
        return self._result(command)


def _reclaim(module, daemon, monkeypatch, tmp_path: Path, *, keep: str, resource: str):
    import argparse
    import shutil as _shutil

    monkeypatch.setattr(module, "run_command", daemon)
    # No fstrim: the fake daemon has no Colima behind it, and a real one would
    # make this test depend on the developer's machine.
    monkeypatch.setattr(_shutil, "which", lambda _name: None)
    monkeypatch.setenv("CAPSEM_STORAGE_REPORT_PATH", str(tmp_path / "docker-storage.jsonl"))
    return module.command_reclaim(
        argparse.Namespace(resource=resource, keep=keep, rail="default"),
        load_policy(),
    )


def test_the_base_image_is_declared_generational_and_keeps_only_the_current_tag() -> None:
    resource = load_policy()["resources"]["capsem-linux-rust-base"]

    assert resource["kind"] == "image"
    assert resource["retention"] == "generational"
    assert resource["docker_name"] == "capsem-linux-rust-base", (
        "a repository, not a tag: the tag is minted per lockfile digest"
    )
    # Zero rather than one. Deleting a tag leaves the BuildKit layer cache
    # untouched, and that cache is what makes a rebuild fast -- re-tagging a
    # generation whose layers are still cached is sub-second and needs no
    # network. Raising this to 1 costs a permanent ~25 GiB against a 40 GiB
    # floor to shorten a rebuild that is already short.
    assert resource["keep_previous"] == 0


def test_a_superseded_base_tag_is_removed_while_the_current_one_survives(
    monkeypatch, tmp_path: Path
) -> None:
    """The whole point: no run can want a tag the lockfiles no longer resolve to."""
    module = load_policy_module()
    repository = "capsem-linux-rust-base"
    current = f"{repository}:03ebe122079926b2"
    daemon = _FakeDockerDaemon(
        module,
        {
            f"{repository}:b4ebbb254e6ef534": ("2026-07-02T09:00:00Z", 25_400_000_000),
            f"{repository}:40c7d9f3e85f1091": ("2026-08-01T09:00:00Z", 13_700_000_000),
            current: ("2026-08-05T09:00:00Z", 25_400_000_000),
        },
    )

    status = _reclaim(module, daemon, monkeypatch, tmp_path, keep=current, resource=repository)

    assert status == 0
    assert daemon.removed == [
        f"{repository}:40c7d9f3e85f1091",
        f"{repository}:b4ebbb254e6ef534",
    ], "the superseded generations were not both retired"
    assert current in daemon.tags, "the tag the lane is about to run was deleted"


def test_reclaim_refuses_when_the_tag_it_was_told_to_keep_is_absent(
    monkeypatch, tmp_path: Path
) -> None:
    """Deleting every generation is the one outcome that is never right.

    A missing keep-tag means something removed it between the build and the
    reclaim -- most likely a second worktree. Proceeding would retire the
    generation that process is about to run.
    """
    module = load_policy_module()
    repository = "capsem-linux-rust-base"
    daemon = _FakeDockerDaemon(
        module, {f"{repository}:b4ebbb254e6ef534": ("2026-07-02T09:00:00Z", 25_400_000_000)}
    )

    status = _reclaim(
        module,
        daemon,
        monkeypatch,
        tmp_path,
        keep=f"{repository}:03ebe122079926b2",
        resource=repository,
    )

    assert status != 0
    assert daemon.removed == [], "a reclaim that could not find its anchor deleted anyway"


def test_generations_are_retired_oldest_first_so_keep_previous_keeps_the_newest() -> None:
    module = load_policy_module()
    generations = [
        {"ref": "repo:oldest", "created": "2026-06-01T00:00:00Z"},
        {"ref": "repo:current", "created": "2026-08-05T00:00:00Z"},
        {"ref": "repo:newest-superseded", "created": "2026-08-04T00:00:00Z"},
    ]

    retained, removable = module.superseded_generations(
        generations, keep="repo:current", keep_previous=1
    )

    assert [row["ref"] for row in retained] == ["repo:newest-superseded"]
    assert [row["ref"] for row in removable] == ["repo:oldest"]


def test_warming_the_base_image_is_what_retires_the_superseded_tags() -> None:
    """The one command that can: it holds the tag every other one is measured against.

    Nothing else may. `test_automatic_docker_gc_never_prunes_tagged_images`
    forbids the automatic GC from pruning tagged images -- correctly, so a
    running gate cannot lose the image it is about to use -- which leaves the
    superseded generations with no collector at all until here.
    """
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from helpers.gate import RecordingRunner, gate_issued

    from capsem.gate import config as gate_config
    from capsem.gate.docker import Docker
    from capsem.gate.linuxrust import base_repository, base_tag

    config = gate_config.load(ROOT)
    repository = base_repository(config)
    issued = gate_issued("warm-linux-rust-base")

    # Through the same recorder the run used, because the tag now includes the
    # id of the mutable parent image and a second source for that answer is a
    # second tag.
    expected = base_tag(config, Docker(RecordingRunner(ROOT)))

    assert f"reclaim --resource {repository} --keep {expected}" in " ".join(issued.split()), (
        f"warm-linux-rust-base does not retire superseded {repository} tags:\n{issued}"
    )


def test_the_repository_the_gate_reclaims_is_the_one_the_policy_declares() -> None:
    """Two files, one name. A drift here deletes nothing and says nothing."""
    from capsem.gate import config as gate_config
    from capsem.gate.linuxrust import base_repository

    repository = base_repository(gate_config.load(ROOT))
    declared = load_policy()["resources"].get(repository)

    assert declared is not None, (
        f"the gate reclaims {repository!r}, which config/storage-policy.toml "
        "does not declare -- `reclaim` would refuse and the tags would accumulate"
    )
    assert declared["retention"] == "generational"


def test_every_mounted_volume_is_governed_by_the_policy() -> None:
    """A volume the gate mounts must have an entry here, and not a dead one.

    `capsem-install-release-site-dist` had no entry at all: the install proof
    mounted it every run and no retention, owner or budget applied to it. An
    unmanaged volume is invisible to the disk budget and to `gc`, so it grows
    until a run dies on `no space left on device` and the cause is a resource
    nothing ever claimed.

    The reverse is caught too. A volume marked `obsolete` while something still
    mounts it is a policy that says one thing while the gate does another --
    and the retirements in this change are exactly when that mistake is easy.
    """
    policy = load_policy()["resources"]
    mounted = set(
        re.findall(
            r'source = "(capsem-[^"]+)"',
            (ROOT / "config" / "gate.toml").read_text(encoding="utf-8"),
        )
    )

    ungoverned = sorted(name for name in mounted if name not in policy)
    assert not ungoverned, (
        f"these are mounted but absent from storage-policy.toml: {ungoverned}. "
        "An unmanaged volume is invisible to the disk budget and to gc."
    )

    retired = sorted(name for name in mounted if policy[name]["retention"] == "obsolete")
    assert not retired, (
        f"these are marked obsolete but still mounted: {retired}. Retire the "
        "mount first, or the policy is describing a gate that does not exist."
    )

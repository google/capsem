"""The warm asset shortcut may change work, never the candidate graph."""

from __future__ import annotations

import json
from pathlib import Path

import blake3
import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import imagebuild
from capsem_builder.gate.actions import Action
from capsem_builder.gate.assetcondition import AssetRecovery
from capsem_builder.gate.context import Context
from capsem_builder.gate.plan import Plan
from helpers.gate import RecordingJournal, RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


class _Probe(Action, name="asset-recovery-probe"):
    def __init__(self, calls: list[str]) -> None:
        self._calls = calls

    def render(self) -> str:
        return "recover host assets"

    def perform(self, context: Context) -> None:
        del context
        self._calls.append("performed")


def _obom_payload(arch: str) -> bytes:
    return json.dumps(
        {
            "bomFormat": "CycloneDX",
            "metadata": {
                "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                "component": {
                    "type": "operating-system",
                    "name": f"capsem-rootfs-{arch}",
                    "version": "guest-rootfs",
                    "properties": [
                        {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                        {"name": "capsem:guest:architecture", "value": arch},
                    ],
                },
            },
            "components": [{"purl": "pkg:deb/debian/base-files@1"}],
        }
    ).encode()


def _seed_assets(config) -> Path:
    tree = config.path(config.imagebuild.output) / config.host_arch().name
    tree.mkdir(parents=True)
    required = (*config.artifacts.bootable, *config.assets.evidence_artifacts)
    entries = {}
    for name in required:
        payload = (
            _obom_payload(config.host_arch().name)
            if name == config.assets.obom_artifact
            else b"qualified"
        )
        (tree / name).write_bytes(payload)
        entries[name] = {
            "hash": blake3.blake3(payload).hexdigest(),
            "sha256": "0" * 64,
            "size": len(payload),
        }
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "assets": {
                    "current": "test",
                    "releases": {"test": {"arches": {config.host_arch().name: entries}}},
                },
            }
        ),
        encoding="utf-8",
    )
    return manifest


def _recovery_plan(config, monkeypatch) -> Plan:
    monkeypatch.setattr(imagebuild, "profiles", lambda _config: ["code"])
    plan = Plan("asset-recovery")
    imagebuild.check_assets(plan, config)
    return plan


def test_warm_and_cold_checkouts_describe_the_same_recovery_graph(
    monkeypatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr(imagebuild, "missing", lambda *_args: [])
    warm_plan = _recovery_plan(config, monkeypatch)

    monkeypatch.setattr(imagebuild, "missing", lambda *_args: ["initrd.img"])
    cold_plan = _recovery_plan(config, monkeypatch)

    assert warm_plan.labels == cold_plan.labels
    assert warm_plan.edges == cold_plan.edges
    assert "assets.asset-tools" in warm_plan.labels


def test_describing_recovery_never_reads_asset_presence(monkeypatch) -> None:
    config = gate_config.load(PROJECT_ROOT)

    def refuse_plan_time_read(*_args, **_kwargs):
        raise AssertionError("asset presence was read while describing the plan")

    monkeypatch.setattr("capsem_builder.gate.assetcondition.missing", refuse_plan_time_read)

    plan = _recovery_plan(config, monkeypatch)

    assert "assets.asset-tools" in plan.labels


def test_recovery_action_skips_warm_assets_and_runs_for_cold_assets(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    calls: list[str] = []
    recovery = AssetRecovery(config, arch)
    action = recovery.when(_Probe(calls))
    context = Context(
        RecordingRunner(tmp_path),
        config,
        journal=RecordingJournal(),
    )
    _seed_assets(config)

    action.perform(context)
    assert calls == []

    missing_asset = config.path(config.imagebuild.output) / arch.name / config.artifacts.bootable[0]
    missing_asset.unlink()
    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]
    assert action.render() == "when host assets are missing: recover host assets"


def test_nonempty_partial_asset_output_cannot_satisfy_recovery(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    tree = config.path(config.imagebuild.output) / arch.name
    tree.mkdir(parents=True)
    for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
        (tree / name).write_bytes(b"partial producer output")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_manifest_digest_must_match_every_boot_and_evidence_artifact(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    _seed_assets(config)
    changed = config.path(config.imagebuild.output) / arch.name / config.assets.obom_artifact
    changed.write_bytes(b"nonempty but not the completed producer output")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_rehashed_raw_scanner_output_is_not_a_completed_asset_build(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    manifest = _seed_assets(config)
    obom = config.path(config.imagebuild.output) / arch.name / config.assets.obom_artifact
    raw = json.dumps(
        {
            "bomFormat": "CycloneDX",
            "metadata": {
                "timestamp": "2026-08-12T00:00:00Z",
                "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                "component": {"type": "container", "name": "rootfs"},
            },
            "components": [{"purl": "pkg:deb/debian/base-files@1"}],
        }
    ).encode()
    obom.write_bytes(raw)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    entry = document["assets"]["releases"]["test"]["arches"][arch.name][obom.name]
    entry["hash"] = blake3.blake3(raw).hexdigest()
    entry["size"] = len(raw)
    manifest.write_text(json.dumps(document), encoding="utf-8")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_one_cold_decision_covers_every_action_in_the_recovery(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    recovery = AssetRecovery(config, config.host_arch())
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    recovery.when(_Probe(calls)).perform(context)
    _seed_assets(config)
    recovery.when(_Probe(calls)).perform(context)

    assert calls == ["performed", "performed"]


def test_profile_recovery_builds_are_ordered_and_invalidate_completion_first(
    monkeypatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr(imagebuild, "profiles", lambda _config: ["code", "co-work"])

    plan = Plan("asset-recovery-order")
    images = imagebuild.check_assets(plan, config)

    first, second = images
    assert (first.label, second.label) in plan.edges
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    assert first.actions[0].render() == f"when host assets are missing: rm -rf {manifest}"
    assert "capsem-admin" in first.actions[1].render()


# ---------------------------------------------------------------------------
# What the shortcut may skip.
#
# The recovery fragment is conditioned on "this host's assets are missing",
# which is right for building assets and wrong for building the *tool* that
# builds them. `assets.guest-builders` materialises the locked guest Rust
# builder image, and `initrd.guest-agents` -- which is not conditioned on
# anything -- runs `capsem-builder agent` and needs that image.
#
# So on a machine with warm assets and a stale builder, the gate skipped
# materialising the builder and then failed four steps later with
#
#     locked guest Rust builder is missing: capsem-guest-rust-x86_64:97972c5e...
#
# The builder goes stale on its own schedule: it is keyed on `Cargo.lock`, so
# adding one dependency invalidates it while every asset on disk stays valid.
# That is precisely the case the condition cannot see.
# ---------------------------------------------------------------------------


def _candidate_steps() -> dict:
    from capsem_builder.gate import candidateplan
    from capsem_builder.gate.qualification import LocalQualification

    config = gate_config.load(PROJECT_ROOT)
    plan = Plan("candidate")
    candidateplan.compose(
        plan,
        config,
        qualification=LocalQualification(bin_dir=config.modules.default_bin_dir),
    )
    return {step.label: step for step in plan.steps}


def _is_conditional(step) -> bool:
    return any(action.name == "when-assets-missing" for action in step.actions)


def test_the_guest_rust_builder_is_materialised_whatever_the_assets_look_like() -> None:
    """A later step needs it, and that step asks no questions.

    Making it unconditional costs nothing on a warm machine:
    `materialize_rust_builders` checks `image_exists` and notes that the image
    is already present. What it buys is that "my assets are warm" stops
    meaning "my toolchain is current".
    """
    steps = _candidate_steps()
    builders = steps.get("assets.guest-builders")
    assert builders is not None, sorted(steps)
    assert not _is_conditional(builders), (
        "the guest Rust builder is built only when host assets are missing, "
        "but `initrd.guest-agents` needs it unconditionally -- so a warm "
        "checkout with a stale builder fails four steps later"
    )


def test_the_step_that_needs_the_builder_still_asks_no_questions() -> None:
    """If this ever becomes conditional, the guard above stops being needed.

    Pinning it means the two cannot quietly diverge again in the other
    direction.
    """
    steps = _candidate_steps()
    agents = steps.get("initrd.guest-agents")
    assert agents is not None, sorted(steps)
    assert not _is_conditional(agents)


def test_building_assets_is_still_skipped_on_a_warm_checkout() -> None:
    """The shortcut is the point; only the tool leaves it."""
    steps = _candidate_steps()
    arch = gate_config.load(PROJECT_ROOT).host_arch().name
    for label in (f"assets.image.code.all.{arch}", "assets.recovery-dependencies"):
        subject = steps.get(label)
        assert subject is not None, sorted(steps)
        assert _is_conditional(subject), f"{label} lost the warm-asset shortcut"


# ---------------------------------------------------------------------------
# Rebuilding assets nothing changed.
#
# `AssetLanes._build` shells into `capsem-admin image build` for every profile
# and every stage, every run, with no check of any kind. Four consecutive
# qualifications of one release spent about 25 minutes each rebuilding both
# architectures' assets from sources none of them had touched -- the last
# three changed only test files and a shell function.
#
# The build cache used to carry only `assets/`, which this lane does not read;
# `_when_missing` recovery answers a different question. The lane's own
# `target/ironbank-assets` tree now travels between prefixes with a receipt, and
# preflight must preserve those isolated roots long enough to validate them.
# ---------------------------------------------------------------------------


def test_the_lane_identity_covers_everything_an_asset_is_built_from() -> None:
    """Over-hashing costs a rebuild; under-hashing ships a stale asset.

    So the list is declared, reviewable, and deliberately wider than it
    strictly needs to be. A path that can change what lands in a VM and is
    absent here is the bug this guard exists to catch.
    """
    from capsem_builder.gate import assetidentity

    covered = set(assetidentity.roots(gate_config.load(PROJECT_ROOT)))
    for required in (
        "guest",                  # guest scripts and files the images contain
        "config",                 # profiles, packages, VM values, and templates
        "build_system/builder/image",     # the image build implementation
        "crates/capsem-admin",    # the profile-owned public build rail
        "crates/capsem-core",     # profile/config semantics used by admin
        "crates/capsem-logger",   # capsem-core's local dependency closure
        "crates/capsem-agent",    # the guest agent binary
        "crates/capsem-proto",    # the agent/core shared protocol
        "crates/capsem-bench",    # the guest benchmark binary in rootfs
        "Cargo.toml",             # workspace versions and dependency features
        "Cargo.lock",             # exact Rust dependency graph
        "build_system/pyproject.toml",  # Python builder dependency declaration
        "build_system/uv.lock",         # exact Python dependency graph
        ".cargo",                 # cross-target compiler/linker configuration
        "rust-toolchain.toml",    # exact Rust compiler and components
    ):
        assert any(root == required or root.startswith(f"{required}/") for root in covered), (
            f"{required} can change what a VM boots and is not part of the lane "
            f"identity, so a change there would reuse a stale asset: {sorted(covered)}"
        )


def test_a_lane_whose_inputs_are_unchanged_is_reused(tmp_path: Path) -> None:
    from capsem_builder.gate import assetidentity

    (tmp_path / "guest").mkdir()
    (tmp_path / "guest" / "init").write_text("one", encoding="utf-8")
    first = assetidentity.digest_of(tmp_path, ("guest",))
    assert assetidentity.digest_of(tmp_path, ("guest",)) == first


def test_a_lane_whose_inputs_moved_is_rebuilt(tmp_path: Path) -> None:
    from capsem_builder.gate import assetidentity

    (tmp_path / "guest").mkdir()
    (tmp_path / "guest" / "init").write_text("one", encoding="utf-8")
    first = assetidentity.digest_of(tmp_path, ("guest",))
    (tmp_path / "guest" / "init").write_text("two", encoding="utf-8")
    assert assetidentity.digest_of(tmp_path, ("guest",)) != first


def test_a_new_file_changes_the_identity(tmp_path: Path) -> None:
    """Addition is a change. Hashing contents alone would miss it."""
    from capsem_builder.gate import assetidentity

    (tmp_path / "guest").mkdir()
    (tmp_path / "guest" / "init").write_text("one", encoding="utf-8")
    first = assetidentity.digest_of(tmp_path, ("guest",))
    (tmp_path / "guest" / "extra").write_text("", encoding="utf-8")
    assert assetidentity.digest_of(tmp_path, ("guest",)) != first


def test_an_executable_mode_change_changes_the_identity(tmp_path: Path) -> None:
    """Docker COPY preserves mode, so equal bytes can still build new output."""
    from capsem_builder.gate import assetidentity

    script = tmp_path / "guest" / "build.sh"
    script.parent.mkdir()
    script.write_text("#!/bin/sh\n", encoding="utf-8")
    script.chmod(0o644)
    first = assetidentity.digest_of(tmp_path, ("guest",))

    script.chmod(0o755)

    assert assetidentity.digest_of(tmp_path, ("guest",)) != first


def test_a_symlink_target_change_changes_the_identity(tmp_path: Path) -> None:
    """The path a build context exposes is an input even when bytes match."""
    from capsem_builder.gate import assetidentity

    guest = tmp_path / "guest"
    guest.mkdir()
    (guest / "one").write_text("same", encoding="utf-8")
    (guest / "two").write_text("same", encoding="utf-8")
    selected = guest / "selected"
    selected.symlink_to("one")
    first = assetidentity.digest_of(tmp_path, ("guest",))

    selected.unlink()
    selected.symlink_to("two")

    assert assetidentity.digest_of(tmp_path, ("guest",)) != first


def test_an_absent_root_is_refused_rather_than_hashed_as_empty(tmp_path: Path) -> None:
    """A typo in the declared list would otherwise read as "nothing here",
    silently shrinking the identity to the roots that happen to exist."""
    from capsem_builder.gate import assetidentity
    from capsem_builder.gate.errors import GateError

    with pytest.raises(GateError, match="ghost"):
        assetidentity.digest_of(tmp_path, ("ghost",))


def _seed_lane_receipt(tmp_path: Path, *, identity: str = "a" * 64):
    from capsem_builder.gate import assetreceipt

    base = gate_config.load(PROJECT_ROOT)
    prefix = base.prefix.model_copy(update={"parent": str(tmp_path / "prefixes")})
    config = base.model_copy(update={"root": tmp_path, "prefix": prefix})
    arch = config.arch("x86_64")
    output = config.path(config.assets.test_root) / "code" / f"build-{arch.name}"
    produced = output / arch.name
    produced.mkdir(parents=True)
    for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
        payload = _obom_payload(arch.name) if name == config.assets.obom_artifact else b"bytes"
        (produced / name).write_bytes(payload)
    assetreceipt.record(
        config,
        output,
        identity,
        profile="code",
        arch=arch,
        stage="packed",
    )
    return config, arch, output, produced


def test_the_lane_skips_a_build_only_when_its_receipt_and_bytes_match(tmp_path: Path) -> None:
    """The whole point: unchanged inputs cost nothing.

    Recorded beside the output rather than in a side table, so a reused tree
    and the identity that justifies reusing it cannot be separated -- a stamp
    that outlives its assets is how a cache starts lying.
    """
    from capsem_builder.gate import assetreceipt

    config, arch, output, _produced = _seed_lane_receipt(tmp_path)

    assert assetreceipt.validates(config, output, "a" * 64, profile="code", arch=arch)
    assert not assetreceipt.validates(config, output, "b" * 64, profile="code", arch=arch)
    assert not assetreceipt.validates(config, output, "a" * 64, profile="co-work", arch=arch)


@pytest.mark.parametrize("mutation", ["change", "delete", "add"])
def test_a_receipt_never_accepts_mutated_or_partial_output(
    tmp_path: Path, mutation: str
) -> None:
    """Existence plus a matching input stamp is not reusable authority."""
    from capsem_builder.gate import assetreceipt

    config, arch, output, produced = _seed_lane_receipt(tmp_path)
    target = produced / config.artifacts.rootfs
    if mutation == "change":
        target.write_bytes(b"different")
    elif mutation == "delete":
        target.unlink()
    else:
        (produced / "unrecorded").write_bytes(b"extra")

    assert not assetreceipt.validates(config, output, "a" * 64, profile="code", arch=arch)


def test_preflight_keeps_only_reusable_lane_roots(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The cache is not real if preflight deletes it before the hit check."""
    from capsem_builder.gate import assetlanes
    from capsem_builder.gate.assetlanes import Profile

    config, arch, output, _produced = _seed_lane_receipt(tmp_path)
    profile_root = output.parent
    (profile_root / config.assets.merged_assets_dir).mkdir()
    (profile_root / config.assets.merged_config_dir).mkdir()
    obsolete = config.path(config.assets.test_root) / "deleted-profile"
    obsolete.mkdir()
    log = config.path(config.assets.test_root) / f"build-{arch.name}.log"
    log.write_text("old", encoding="utf-8")
    monkeypatch.setattr("capsem_builder.gate.assetidentity.lane_identity", lambda _config: "a" * 64)

    assetlanes.prepare_workspace(
        config,
        [Profile(name="code", manifest=tmp_path / "config/profiles/code/profile.toml")],
    )

    assert output.is_dir()
    assert not (profile_root / config.assets.merged_assets_dir).exists()
    assert not (profile_root / config.assets.merged_config_dir).exists()
    assert not obsolete.exists()
    assert not log.exists()


def test_asset_plan_seals_receipts_before_packing_can_be_carried() -> None:
    """A journal may call packing complete only after its exact receipt exists."""
    from capsem_builder.gate.assetplan import fragment

    config = gate_config.load(PROJECT_ROOT)
    plan = Plan("asset-receipts")
    fragment(plan, config)

    for arch in config.architectures:
        lane = plan.step_named(f"assets.build.{arch}")
        assert [check.name for check in lane.carry_checks] == ["require-asset-lane-receipts"]
    packed = plan.step_named("assets.pack-initrds")
    assert packed.actions[-1].name == "seal-packed-asset-lane-receipts"
    assert [check.name for check in packed.carry_checks] == ["require-asset-lane-receipts"]

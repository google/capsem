"""Actual pinned Cargo no-run protocol, explicitly rebased into source fixtures.

The original fixture was compiled only; its executable was never invoked.
The copied documents below keep the original capture bytes and provenance.
"""

import copy
import json
from pathlib import Path

import pytest

from benchmarks.helpers import criterion_artifact as artifact

FIXTURE = Path(__file__).parents[1] / "fixtures/criterion_cargo_1_97_1"
PROFILE = {
    "opt_level": "3",
    "debuginfo": 0,
    "debug_assertions": False,
    "overflow_checks": False,
    "test": True,
}


@pytest.fixture
def protocol(tmp_path):
    metadata = json.loads((FIXTURE / "metadata.json").read_text())
    events = [
        json.loads(line) for line in (FIXTURE / "cargo-bench-no-run.jsonl").read_text().splitlines()
    ]
    provenance = json.loads((FIXTURE / "provenance.json").read_text())
    source = tmp_path / "source"
    stage = tmp_path / "shared-cargo"
    original = source / "bench.rs"
    source.mkdir()
    original.write_text(provenance["fixture_source"])
    (source / "Cargo.toml").write_text(provenance["fixture_manifest"])
    executable = stage / "release/deps/criterion_protocol-source-double"
    executable.parent.mkdir(parents=True)
    executable.write_bytes(b"source double only, never executed")
    executable.chmod(0o755)

    # Path substitution is a labelled source-double transformation, not another capture.
    def rebase(value):
        return json.loads(
            json.dumps(value)
            .replace(provenance["source_root"], str(source))
            .replace(provenance["target_directory"], str(stage))
            .replace("criterion_protocol-c8057a5dd1c0b0d2", executable.name)
        )

    metadata, events = rebase(metadata), rebase(events)
    return metadata, events, source, stage, executable


def select(protocol):
    metadata, events, source, stage, _ = protocol
    selected = artifact.select_target(
        metadata, source, stage, "capsem-criterion-protocol-fixture", "criterion_protocol"
    )
    return artifact.select_executable(events, selected, stage, PROFILE, profile_directory="release")


def test_actual_positive_no_run_artifact_has_exact_local_identity(protocol):
    assert select(protocol).path == protocol[-1]
    assert json.loads((FIXTURE / "provenance.json").read_text())["executed"] is False


@pytest.mark.parametrize(
    "change",
    [
        "missing",
        "failed",
        "duplicate",
        "after_finish",
        "no_finish",
        "foreign_package",
        "manifest",
        "source",
        "target",
        "profile",
        "null",
        "filenames",
        "foreign_stage",
        "debug_stage",
        "linked",
        "nonexecutable",
        "special",
        "unknown_reason",
    ],
)
def test_wrong_or_partial_artifacts_are_not_selected(protocol, change, tmp_path):
    _metadata, events, source, stage, executable = protocol
    row = events[0]
    if change == "missing":
        events.pop(0)
    elif change == "failed":
        events[-1]["success"] = False
    elif change == "duplicate":
        events.insert(0, copy.deepcopy(row))
    elif change == "after_finish":
        events.append(copy.deepcopy(row))
    elif change == "no_finish":
        events.pop()
    elif change == "foreign_package":
        row["package_id"] = "foreign"
    elif change == "manifest":
        row["manifest_path"] = str(source / "other.toml")
    elif change == "source":
        row["target"]["src_path"] = str(source / "foreign.rs")
    elif change == "target":
        row["target"]["kind"] = ["lib"]
    elif change == "profile":
        row["profile"]["opt_level"] = "0"
    elif change == "null":
        row["executable"] = None
    elif change == "filenames":
        row["filenames"] = []
    elif change in ("foreign_stage", "debug_stage"):
        other = tmp_path / "outside" if change == "foreign_stage" else stage / "debug/deps/other"
        other.parent.mkdir(parents=True, exist_ok=True)
        other.write_bytes(b"other")
        other.chmod(0o755)
        row["executable"] = str(other)
        row["filenames"] = [str(other)]
    elif change == "linked":
        executable.unlink()
        executable.symlink_to(source / "bench.rs")
    elif change == "nonexecutable":
        executable.chmod(0o644)
    elif change == "special":
        executable.unlink()
        executable.mkdir()
    else:
        row["reason"] = "new-unknown-protocol"
    with pytest.raises(ValueError):
        select(protocol)


@pytest.mark.parametrize(
    "change", ["foreign_stage", "nonlocal", "duplicate", "source", "nonmember", "foreign_manifest"]
)
def test_metadata_must_declare_exact_local_workspace_target(protocol, change):
    metadata, _, _source, _stage, _ = protocol
    package = metadata["packages"][0]
    if change == "foreign_stage":
        metadata["target_directory"] = "/foreign"
    elif change == "nonlocal":
        package["source"] = "registry+foreign"
    elif change == "duplicate":
        metadata["packages"].append(copy.deepcopy(package))
    elif change == "source":
        package["targets"][0]["src_path"] = "/foreign.rs"
    elif change == "nonmember":
        metadata["workspace_members"] = []
    else:
        package["manifest_path"] = "/foreign/Cargo.toml"
    with pytest.raises(ValueError):
        select(protocol)


def test_successful_warm_artifact_still_requires_exact_original_executable(protocol):
    protocol[1][0]["fresh"] = True
    stamp = select(protocol)
    stamp.verify()
    stamp.path.write_bytes(b"replacement")
    with pytest.raises(ValueError, match="replaced"):
        stamp.verify()


def test_profile_scalar_types_and_profile_directory_are_not_laundered(protocol, tmp_path):
    protocol[1][0]["profile"]["debuginfo"] = False
    with pytest.raises(ValueError, match="profile"):
        select(protocol)
    protocol[1][0]["profile"]["debuginfo"] = 0
    stage = protocol[3]
    external = tmp_path / "external"
    external.mkdir()
    (stage / "foreign-profile").symlink_to(external, target_is_directory=True)
    metadata, events, root, stage, _ = protocol
    target = artifact.select_target(
        metadata, root, stage, "capsem-criterion-protocol-fixture", "criterion_protocol"
    )
    with pytest.raises(ValueError, match="escapes"):
        artifact.select_executable(
            events, target, stage, PROFILE, profile_directory="foreign-profile"
        )

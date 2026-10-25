"""Only the image-owned scratch alias may reuse the existing workspace mount."""

import os
import runpy
from pathlib import Path

import pytest

from tests.fixtures.oci.test_launch_config import SECURITY, launcher, unpacked
from tests.fixtures.oci.test_launch_config import image as image_metadata

__all__ = ["launcher"]

SOURCE = (
    Path(__file__).resolve().parents[3]
    / "guest/artifacts/container/runtime_workspace.py"
)


def policy():
    return {
        "config": {
            "Labels": {"org.capsem.workspace.policy": "antigravity-scratch-v1"},
            "Volumes": {"/home/capsem": {}},
        }
    }


def module():
    return runpy.run_path(str(SOURCE))


def test_alias_uses_existing_workspace_backing_and_masks_stage_at_both_paths():
    mounts = module()["alias_mounts"](
        policy(), "/workspace", "/existing/workspace", ".capsem-image"
    )

    assert len(mounts) == 2
    assert mounts[0]["destination"] == "/home/capsem/.gemini/antigravity/scratch"
    assert mounts[0]["source"] == "/existing/workspace"
    assert mounts[1]["destination"].endswith("/scratch/.capsem-image")
    assert mounts[1]["options"] == [
        "ro",
        "nosuid",
        "nodev",
        "noexec",
        "mode=000",
        "size=4k",
    ]
    assert (
        module()["alias_mounts"](
            {"config": {}}, "/workspace", "unused", ".capsem-image"
        )
        == []
    )


def test_real_launcher_applies_only_the_recipe_owned_alias_after_home_volume(launcher):
    metadata = image_metadata()
    metadata["config"].update(policy()["config"])
    options = {**SECURITY, "args": [], "env": {}, "workspace": "/workspace"}
    config = launcher.configure(unpacked(), metadata, options)
    destinations = [mount["destination"] for mount in config["mounts"]]
    scratch = "/home/capsem/.gemini/antigravity/scratch"
    assert destinations.index(scratch) > destinations.index("/home")
    assert destinations.index(scratch + "/.capsem-image") > destinations.index(scratch)
    alias = next(mount for mount in config["mounts"] if mount["destination"] == scratch)
    assert alias["source"] == str(launcher.WORKSPACE_VIEW)


def test_scratch_import_preserves_originals_conflicts_and_credential_siblings(tmp_path):
    home = tmp_path / "home"
    scratch = home / ".gemini/antigravity/scratch"
    scratch.mkdir(parents=True)
    (scratch / "poem.md").write_bytes(b"original poem\n")
    (home / ".gemini/credentials.json").write_bytes(b"must not copy")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "poem.md").write_bytes(b"existing workspace\n")
    receipts = tmp_path / "private"
    receipts.mkdir()
    migrate = module()["import_scratch"]
    migrate(home, workspace, receipts)
    assert (scratch / "poem.md").read_bytes() == b"original poem\n"
    assert (workspace / "poem.md").read_bytes() == b"existing workspace\n"
    assert (workspace / "poem.md.imported-1").read_bytes() == b"original poem\n"
    assert not (workspace / "credentials.json").exists()
    before = sorted(path.name for path in workspace.iterdir())
    migrate(home, workspace, receipts)
    assert sorted(path.name for path in workspace.iterdir()) == before


def test_copy_uses_current_user_without_changing_destination_ownership(tmp_path):
    source = tmp_path / "source"
    source.mkdir()
    (source / "poem").write_bytes(b"poem")
    destination = tmp_path / "destination"
    destination.mkdir()
    with module()["directory"](None, source) as src, module()["directory"](None, destination) as dst:
        module()["copy_as_owner"](src, dst, 100, (os.geteuid(), os.getegid()))
    assert (destination / "poem").read_bytes() == b"poem"
    assert (destination / "poem").stat().st_uid == os.geteuid()


def test_import_refuses_a_source_growing_after_the_size_check(tmp_path, monkeypatch):
    source = tmp_path / "source"
    source.write_bytes(b"x")
    target = tmp_path / "target"
    target.mkdir()
    owner = module()
    with source.open("rb") as handle, owner["directory"](None, target) as destination:
        actual_read = os.read
        reads = iter([b"xx", b""])
        monkeypatch.setattr(os, "read", lambda fd, count: next(reads) if fd == handle.fileno() else actual_read(fd, count))
        with pytest.raises(ValueError, match="changed"):
            owner["copy_file"](handle.fileno(), destination, "poem", owner["Budget"](100), None)
    assert source.read_bytes() == b"x"


@pytest.mark.parametrize(
    "attack", ["source_parent", "source_leaf", "destination_leaf", "receipt"]
)
def test_import_never_follows_guest_links(tmp_path, attack):
    home = tmp_path / "home"
    scratch = home / ".gemini/antigravity/scratch"
    scratch.mkdir(parents=True)
    outside = tmp_path / "outside"
    outside.mkdir()
    sentinel = outside / "secret"
    sentinel.write_text("untouched")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    receipts = tmp_path / "private"
    receipts.mkdir()
    if attack == "source_parent":
        scratch.rmdir()
        scratch.symlink_to(outside, target_is_directory=True)
    elif attack == "source_leaf":
        (scratch / "secret").symlink_to(sentinel)
    elif attack == "receipt":
        (receipts / "antigravity-scratch-v1").symlink_to(sentinel)
    else:
        (scratch / "secret").write_text("new file")
        (workspace / "secret").symlink_to(sentinel)
    if attack == "destination_leaf":
        module()["import_scratch"](home, workspace, receipts)
        assert (workspace / "secret.imported-1").read_text() == "new file"
    else:
        with pytest.raises((OSError, ValueError)):
            module()["import_scratch"](home, workspace, receipts)
    assert sentinel.read_text() == "untouched"


def test_import_has_finite_byte_and_entry_budgets(tmp_path):
    home = tmp_path / "home"
    scratch = home / ".gemini/antigravity/scratch"
    scratch.mkdir(parents=True)
    (scratch / "large").write_bytes(b"12345")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    receipts = tmp_path / "private"
    receipts.mkdir()
    with pytest.raises(ValueError, match="budget"):
        module()["import_scratch"](home, workspace, receipts, max_bytes=4)
    assert (scratch / "large").read_bytes() == b"12345"
    assert not (receipts / "antigravity-scratch-v1").exists()


def test_import_refuses_special_files_and_the_reserved_launcher_stage(tmp_path):
    home = tmp_path / "home"
    scratch = home / ".gemini/antigravity/scratch"
    scratch.mkdir(parents=True)
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    receipts = tmp_path / "private"
    receipts.mkdir()
    os.mkfifo(scratch / "pipe")
    with pytest.raises(ValueError, match="non-regular"):
        module()["import_scratch"](home, workspace, receipts)
    (scratch / "pipe").unlink()
    (scratch / ".capsem-image").mkdir()
    with pytest.raises(ValueError, match="stage"):
        module()["import_scratch"](home, workspace, receipts)
    assert list(workspace.iterdir()) == []


def test_policy_cannot_choose_an_arbitrary_alias_or_home_volume():
    for image, workspace in [
        ({"config": {"Labels": {"org.capsem.workspace.policy": "/etc"}}}, "/workspace"),
        (policy(), "/other"),
        ({"config": {"Labels": policy()["config"]["Labels"]}}, "/workspace"),
    ]:
        with pytest.raises(ValueError, match="unsupported"):
            module()["alias_mounts"](image, workspace, "fixed", ".capsem-image")


def test_directory_enumeration_stops_at_the_remaining_entry_budget(monkeypatch):
    import contextlib
    from types import SimpleNamespace

    owner = module()
    visits = []

    @contextlib.contextmanager
    def entries(_fd):
        def generate():
            for index in range(1000000):
                visits.append(index)
                yield SimpleNamespace(name=str(index))

        yield generate()

    monkeypatch.setattr(os, "scandir", entries)
    budget = owner["Budget"](100)
    budget.entries = 2
    with pytest.raises(ValueError, match="entry budget"):
        owner["bounded_names"](42, budget)
    assert len(visits) == 3


def test_alias_preserves_backend_owned_workspace_metadata_mapping():
    user = {"uid": 1000, "gid": 1000}
    choose = module()["workspace_user"]
    # Apple's daemon reflects caller-namespace ids; KVM reports stable root.
    # Rotating caller-relative uid1000 makes it workload uid999.
    assert choose(policy(), "/workspace", None, user) is None
    assert choose(policy(), "/workspace", "image_user", user) == user
    assert choose({"config": {}}, "/workspace", None, user) is None
    assert choose({"config": {}}, "/workspace", "image_user", user) == user
    with pytest.raises(ValueError, match="owner policy"):
        choose(policy(), "/workspace", "root", user)

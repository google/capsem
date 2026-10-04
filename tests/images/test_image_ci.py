"""The official image publication scripts under images/ci.

These compute what the publication workflow decides on: whether a build is
already published (the input key) and what a channel's catalog says (the
merge). Both are pure, so they are proved here rather than discovered on the
hosted runner.
"""

from __future__ import annotations

import copy
import importlib.util
import sys
from pathlib import Path
from types import ModuleType

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "images" / "ci"


def _load(name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(f"image_ci_{name}", SCRIPTS / f"{name}.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


inputkey = _load("inputkey")
catalog = _load("catalog")

REGISTRY = "ghcr.io/google/capsem"
ARM = "sha256:" + "1" * 64
AMD = "sha256:" + "2" * 64
INDEX = "sha256:" + "3" * 64


def _tree(root: Path, files: dict[str, str]) -> Path:
    for name, text in files.items():
        path = root / "images" / "dev" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    return root / "images" / "dev"


def _key(root: Path, *extra: str) -> str:
    return inputkey.input_key([root / "images" / "dev"], root=root, extra=extra)


# --- input key -------------------------------------------------------------


def test_the_key_is_stable_for_the_same_inputs(tmp_path: Path) -> None:
    _tree(tmp_path / "a", {"Dockerfile": "FROM x\n", "lib/tool.sh": "echo\n"})
    _tree(tmp_path / "b", {"lib/tool.sh": "echo\n", "Dockerfile": "FROM x\n"})
    assert _key(tmp_path / "a") == _key(tmp_path / "b")


@pytest.mark.parametrize(
    "change",
    ["content", "rename", "new file", "executable bit", "base digest"],
)
def test_the_key_changes_with_every_input(tmp_path: Path, change: str) -> None:
    image = _tree(tmp_path, {"Dockerfile": "FROM x\n", "tool.sh": "echo\n"})
    before = _key(tmp_path, ARM)
    extra = ARM
    if change == "content":
        (image / "Dockerfile").write_text("FROM y\n")
    elif change == "rename":
        (image / "tool.sh").rename(image / "tool2.sh")
    elif change == "new file":
        (image / "README").write_text("")
    elif change == "executable bit":
        (image / "tool.sh").chmod(0o755)
    else:
        extra = AMD
    assert _key(tmp_path, extra) != before


def test_moving_bytes_between_files_changes_the_key(tmp_path: Path) -> None:
    """Length-prefixed fields: `ab`+`c` must not collide with `a`+`bc`."""
    _tree(tmp_path / "a", {"1": "ab", "2": "c"})
    _tree(tmp_path / "b", {"1": "a", "2": "bc"})
    assert _key(tmp_path / "a") != _key(tmp_path / "b")


def test_an_empty_directory_cannot_key_a_build(tmp_path: Path) -> None:
    (tmp_path / "images" / "dev").mkdir(parents=True)
    with pytest.raises(ValueError, match="no files"):
        _key(tmp_path)


# --- catalog ---------------------------------------------------------------


def _records(*, built: bool = True, key: str = "k1") -> list[dict]:
    legs = []
    for arch, digest in (("arm64", ARM), ("amd64", AMD)):
        leg = {"kind": "platform", "image": "dev", "arch": arch, "key": key, "built": built}
        if built:
            leg |= {"digest": digest, "obom": digest, "erofs": digest}
        legs.append(leg)
    index = {"kind": "index", "image": "dev", "key": key, "built": built}
    if built:
        index["digest"] = INDEX
    return [*legs, index]


def _merge(previous: dict | None, new: dict) -> tuple[dict, bool]:
    return catalog.merge(
        previous,
        channel="nightly",
        generated_at="2026-10-03T00:00:00Z",
        described={"dev": "A dev image."},
        new=new,
    )


def test_a_rebuilt_image_becomes_one_version() -> None:
    assert catalog.versions(_records(), REGISTRY) == {
        "dev": {
            "image": f"{REGISTRY}/dev@{INDEX}",
            "platforms": ["linux/amd64", "linux/arm64"],
            "contract": 1,
            "obom": {"arm64": ARM, "amd64": AMD},
            "erofs": {"arm64": ARM, "amd64": AMD},
        }
    }


def test_a_skipped_image_adds_no_version() -> None:
    assert catalog.versions(_records(built=False), REGISTRY) == {}


@pytest.mark.parametrize(
    "defect", ["missing obom", "bad erofs", "mixed built", "mixed key", "orphan"]
)
def test_records_that_do_not_describe_one_build_are_refused(defect: str) -> None:
    records = _records()
    if defect == "missing obom":
        del records[0]["obom"]
    elif defect == "bad erofs":
        records[1]["erofs"] = "sha256:nothex"
    elif defect == "mixed built":
        records[1] = {k: v for k, v in records[1].items() if k in {"kind", "image", "arch", "key"}}
        records[1]["built"] = False
    elif defect == "mixed key":
        records[1]["key"] = "k2"
    else:
        records.pop()
    with pytest.raises(catalog.CatalogError):
        catalog.versions(records, REGISTRY)


def test_the_first_catalog_holds_the_new_version() -> None:
    new = catalog.versions(_records(), REGISTRY)
    document, changed = _merge(None, new)
    assert changed
    assert document["schema_version"] == 1
    assert document["channel"] == "nightly"
    assert document["entries"]["dev"] == {"description": "A dev image.", "versions": [new["dev"]]}


def test_a_new_version_is_appended_and_no_old_one_is_dropped() -> None:
    first, _ = _merge(None, catalog.versions(_records(), REGISTRY))
    first["entries"]["retired"] = {"description": "Gone.", "versions": [{"image": "old"}]}
    snapshot = copy.deepcopy(first)
    rebuilt = copy.deepcopy(catalog.versions(_records(), REGISTRY))
    rebuilt["dev"]["image"] = f"{REGISTRY}/dev@sha256:{'4' * 64}"

    second, changed = _merge(first, rebuilt)

    assert changed
    assert first == snapshot, "the merge must not mutate the previous catalog"
    versions = [version["image"] for version in second["entries"]["dev"]["versions"]]
    assert versions == [f"{REGISTRY}/dev@{INDEX}", f"{REGISTRY}/dev@sha256:{'4' * 64}"]
    assert second["entries"]["retired"] == first["entries"]["retired"]


def test_republishing_a_listed_digest_changes_nothing() -> None:
    new = catalog.versions(_records(), REGISTRY)
    first, _ = _merge(None, new)
    second, changed = _merge(first, new)
    assert not changed
    assert second["entries"] == first["entries"]


@pytest.mark.parametrize(
    ("field", "value", "message"),
    [("schema_version", 2, "schema"), ("channel", "stable", "channel")],
)
def test_a_previous_catalog_from_elsewhere_is_refused(
    field: str, value: object, message: str
) -> None:
    previous, _ = _merge(None, {})
    previous[field] = value
    with pytest.raises(catalog.CatalogError, match=message):
        _merge(previous, {})


def test_an_image_without_a_description_is_refused() -> None:
    new = catalog.versions(_records(), REGISTRY)
    with pytest.raises(catalog.CatalogError, match="description"):
        catalog.merge(None, channel="nightly", generated_at="t", described={}, new=new)


# --- one list of images ----------------------------------------------------


def test_descriptions_directories_and_workflow_name_the_same_images() -> None:
    """A new image directory that the workflow never builds, or a catalog entry
    with no image behind it, is a publication that silently does not happen."""
    directories = {path.parent.name for path in (ROOT / "images").glob("*/Dockerfile")} - {
        "base",
        _debug_image_directory(),
    }
    workflow = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    for job in ("image-arch", "image"):
        assert set(workflow["jobs"][job]["strategy"]["matrix"]["image"]) == directories, job
    assert set(catalog.descriptions()) == directories


def _debug_image_directory() -> str:
    from capsem_builder.gate import config as gate_config

    return Path(gate_config.load(ROOT).functional.debug_image.context).name


DEBUG_IMAGE_RATIONALE = """\
capsem-debug is a test fixture, not an official image.

It carries test runners, network tools, package managers and agent CLIs so the
VM runtime does not. Sessions admit it only because a test grants its local
registry; listed in images/catalog.toml it would be admitted by every default
policy, since admission is the catalog's digests (crates/capsem-core
container::admission), and built by images.yaml it would be published beside
the official images under an input key no test pinned. Its one pin is
config/gate.toml [functional.debug_image].
"""


def test_the_debug_image_is_neither_catalogued_nor_published_by_the_image_workflow() -> None:
    name = _debug_image_directory()
    assert (ROOT / "images" / name / "Dockerfile").is_file()
    assert name not in catalog.descriptions(), DEBUG_IMAGE_RATIONALE
    workflow = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    for job in ("image-arch", "image"):
        assert name not in workflow["jobs"][job]["strategy"]["matrix"]["image"], DEBUG_IMAGE_RATIONALE
    assert name not in (ROOT / ".github/workflows/images.yaml").read_text(), DEBUG_IMAGE_RATIONALE


def test_oci_architectures_resolve_to_the_guest_config() -> None:
    """The rootfs tools run under the guest config's name for each platform."""
    rootfs = _load("rootfs")
    build = rootfs.build_config()
    assert rootfs.builder_architecture(build, "arm64") == "arm64"
    assert rootfs.builder_architecture(build, "amd64") == "x86_64"

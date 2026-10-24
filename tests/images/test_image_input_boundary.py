"""Producer keys cannot include their own mutable functional digest pins."""

import shutil
import tomllib
from pathlib import Path

import pytest
import tomli_w
from capsem_builder.gate.errors import GateError

from images.ci import inputkey
from tests.fixtures.oci.native_inputs import workflow_key

ROOT = Path(__file__).resolve().parents[2]
BASE = "sha256:" + "a" * 64


@pytest.fixture
def source(tmp_path: Path) -> Path:
    # Independent real checked-in source fixture; no fake producer/config model.
    for name in ("images", "config"):
        shutil.copytree(ROOT / name, tmp_path / name, ignore=shutil.ignore_patterns("__pycache__"))
    for directory in ("build_system/builder", "build_system/builder/image", "build_system/builder/gate"):
        target = tmp_path / directory
        target.mkdir(parents=True, exist_ok=True)
        for path in (ROOT / directory).glob("*.py"):
            shutil.copy2(path, target / path.name)
    for name in (
        ".github/workflows/images.yaml", "build_system/builder/release/obom.py",
        "build_system/builder/release/package_inventory.py", "build_system/builder/release/releasechannel.py",
        "build_system/builder/policy/dockerpolicy.py", "build_system/docker/Dockerfile.asset-tools",
        "build_system/pyproject.toml", "build_system/uv.lock",
    ):
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, path)
    return tmp_path


def key(source: Path) -> str:
    return workflow_key(source, "dev", BASE)


@pytest.mark.parametrize("change", ["platform", "functional-pin", "qualification", "branding"])
def test_nonproducer_edits_do_not_invalidate_original_image(source: Path, change: str) -> None:
    before = key(source)
    if change == "platform":
        path = source / "build_system/builder/gate/platformproof.py"
        path.write_text(path.read_text() + "\n# package qualification changed\n")
    elif change == "functional-pin":
        path = source / "config/gate.toml"
        value = path.read_text()
        start = value.index("[functional.reference_image]")
        position = value.index("sha256:", start) + len("sha256:")
        path.write_text(value[:position] + "f" * 64 + value[position + 64:])
    elif change == "qualification":
        path = source / "images/dev/qualify.toml"
        path.write_text(path.read_text() + "\n# different test policy\n")
    else:
        path = source / "images/catalog.toml"
        path.write_text(path.read_text().replace("An agentless environment", "A useful agentless environment"))
    assert key(source) == before


@pytest.mark.parametrize("path", [
    "images/dev/Dockerfile", "images/ci/rootfs.py", "build_system/builder/image/docker.py",
    "build_system/builder/image/assettools.py", "build_system/builder/crypto.py",
    "build_system/builder/release/package_inventory.py", "build_system/builder/release/obom.py",
    "build_system/builder/policy/dockerpolicy.py", "build_system/docker/Dockerfile.asset-tools",
    "build_system/builder/gate/config.py", "build_system/builder/gate/auditfs.py",
    "build_system/uv.lock",
])
def test_real_producer_code_and_tool_inputs_invalidate(source: Path, path: str) -> None:
    before = key(source)
    target = source / path
    target.write_bytes(target.read_bytes() + b"\n# producer input changed\n")
    assert key(source) != before


@pytest.mark.parametrize("change", ["execute", "rename", "add", "base"])
def test_context_metadata_and_original_base_remain_inputs(source: Path, change: str) -> None:
    before = key(source)
    path = source / "images/dev/Dockerfile"
    if change == "execute":
        path.chmod(path.stat().st_mode ^ 0o100)
    elif change == "rename":
        (source / "images/dev/new-input").write_bytes(b"same bytes")
        before = key(source)
        (source / "images/dev/new-input").rename(source / "images/dev/renamed-input")
    elif change == "add":
        (source / "images/dev/new-input").write_bytes(b"new actual context bytes")
    else:
        assert workflow_key(source, "dev", "sha256:" + "b" * 64) != before
        return
    assert key(source) != before


@pytest.mark.parametrize("old,new", [
    ('compression_level = 12', 'compression_level = 11'),
    ('cluster_size = 65536', 'cluster_size = 32768'),
    ('debian_snapshot_id = "20260810T000000Z"', 'debian_snapshot_id = "20260811T000000Z"'),
    ('cdxgen-linux-arm64', 'cdxgen-linux-arm64-new'),
    ('cdxgen-linux-amd64', 'cdxgen-linux-amd64-new'),
    ('cdx-validate-linux-amd64', 'cdx-validate-linux-amd64-new'),
])
def test_selected_configuration_and_both_arch_tools_invalidate(source: Path, old: str, new: str) -> None:
    before = key(source)
    path = source / "config/docker/image/build.toml"
    value = path.read_text()
    assert old in value
    path.write_text(value.replace(old, new, 1))
    assert key(source) != before


@pytest.mark.parametrize("change", ["missing-code", "bad-config", "selector-escape", "selector-symlink"])
def test_invalid_admission_fails_before_a_cache_key(source: Path, change: str, tmp_path: Path) -> None:
    if change == "missing-code":
        (source / "build_system/builder/release/obom.py").unlink()
    elif change == "bad-config":
        path = source / "config/gate.toml"
        path.write_text(path.read_text().replace('minimum_glibc = "2.39"', 'minimum_glibc = false'))
    elif change == "selector-escape":
        path = source / "config/gate.toml"
        path.write_text(path.read_text().replace('source_config = "config/docker/image"', 'source_config = "../foreign"'))
    else:
        path = source / "config/docker/image"
        foreign = source / "held-config"
        path.rename(foreign)
        path.symlink_to(foreign, target_is_directory=True)
    with pytest.raises((OSError, ValueError, GateError), match=r"missing|configuration|invalid|source|escape|symlink|producer"):
        key(source)


@pytest.mark.parametrize("section,field", [
    (("architectures", "arm64"), "base_image"),
    (("architectures", "x86_64"), "base_image"),
    (("architectures", "arm64"), "rust_builder_base_image"),
    (("architectures", "x86_64"), "rust_builder_base_image"),
    (("asset_tools", "erofs_source"), "sha256"),
    (("asset_tools", "architectures", "arm64", "cdxgen"), "sha256"),
    (("asset_tools", "architectures", "x86_64", "cdxgen"), "sha256"),
    (("asset_tools", "architectures", "arm64", "cdx_validate"), "sha256"),
    (("asset_tools", "architectures", "x86_64", "cdx_validate"), "sha256"),
])
def test_every_selected_arch_and_scanner_pin_is_bound(source: Path, section: tuple[str, ...], field: str) -> None:
    before = key(source)
    path = source / "config/docker/image/build.toml"
    data = tomllib.loads(path.read_text())
    selected = data["build"]
    for component in section:
        selected = selected[component]
    value = selected[field]
    selected[field] = value.split("sha256:")[0] + "sha256:" + "f" * 64 if "sha256:" in value else "f" * 64
    path.write_text(tomli_w.dumps(data))
    assert key(source) != before


def test_config_formatting_is_not_artifact_identity_but_selector_is(source: Path) -> None:
    before = key(source)
    path = source / "config/docker/image/build.toml"
    path.write_text(tomli_w.dumps(tomllib.loads(path.read_text())))
    assert key(source) == before
    alternative = source / "config/docker/alternative"
    shutil.copytree(path.parent, alternative)
    gate = source / "config/gate.toml"
    gate.write_text(gate.read_text().replace('source_config = "config/docker/image"', 'source_config = "config/docker/alternative"'))
    assert key(source) != before


@pytest.mark.parametrize("change", ["missing-tool-pin", "duplicate-arch", "disabled-erofs", "tool-path-escape", "context-symlink", "context-directory-symlink"])
def test_missing_or_unsafe_producer_prerequisites_refuse(source: Path, change: str) -> None:
    path = source / "config/docker/image/build.toml"
    data = tomllib.loads(path.read_text())
    if change == "missing-tool-pin":
        del data["build"]["asset_tools"]["architectures"]["x86_64"]["cdxgen"]
    elif change == "duplicate-arch":
        data["build"]["architectures"]["x86_64"]["docker_platform"] = "linux/arm64"
    elif change == "disabled-erofs":
        data["build"]["erofs"]["enabled"] = False
    elif change == "tool-path-escape":
        data["build"]["asset_tools"]["dockerfile"] = "../outside"
    elif change == "context-symlink":
        (source / "images/dev/foreign-link").symlink_to(source / "config/gate.toml")
    else:
        (source / "images/dev/foreign-link").symlink_to(source / "config", target_is_directory=True)
    path.write_text(tomli_w.dumps(data))
    with pytest.raises(ValueError):
        key(source)


@pytest.mark.parametrize("replacement", [
    '--derived foreign --with "$BASE_DIGEST"',
    '--derived "$IMAGE" --with "$BASE_DIGEST" images/dev',
    '--derived "$IMAGE" --with "$OTHER_DIGEST"',
    '"images/$IMAGE" --with "$BASE_DIGEST"',
])
def test_native_rejects_changed_workflow_owner_arguments(source: Path, replacement: str) -> None:
    path = source / ".github/workflows/images.yaml"
    value = path.read_text()
    original = '--derived "$IMAGE" --with "$BASE_DIGEST"'
    assert original in value
    path.write_text(value.replace(original, replacement))
    with pytest.raises(ValueError, match="canonical derived owner"):
        key(source)


def test_base_v1_domain_and_exact_old_key_are_unchanged(source: Path) -> None:
    assert inputkey.VERSION == b"capsem-official-image-input-key-v1"
    assert workflow_key(source, "base") == "caa05a2af87b7c162a9a1a48c9407f7ee05bedea8f29929a0b25041fe1727cb7"
    assert inputkey.DERIVED_DOMAIN == "capsem-derived-image-producer-v2"


@pytest.mark.parametrize("arguments", [
    ["--derived", "dev"], ["--derived", "dev", "--with", BASE, "images/dev"],
    ["--derived", "dev", "--with", BASE, "--with", BASE], [],
])
def test_cli_cannot_override_derived_selection(arguments: list[str]) -> None:
    with pytest.raises(SystemExit) as error:
        inputkey.main(arguments)
    assert error.value.code == 2


@pytest.mark.parametrize("name", [
    "build_system/builder/image/docker.py", "build_system/builder/image/config.py",
    "build_system/builder/image/assettools.py", "build_system/builder/crypto.py",
    "build_system/builder/gate/configschema.py", "build_system/builder/policy/dockerpolicy.py",
    "build_system/docker/Dockerfile.asset-tools",
])
def test_missing_essential_producer_is_not_an_incomplete_glob_key(source: Path, name: str) -> None:
    (source / name).unlink()
    with pytest.raises(ValueError, match="missing"):
        key(source)


def test_cli_and_native_share_one_exact_derived_owner(
    source: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(inputkey, "ROOT", source)
    inputkey.main(["--derived", "dev", "--with", BASE])
    assert capsys.readouterr().out.strip() == key(source)


@pytest.mark.parametrize("name,base", [("../dev", BASE), ("foreign", BASE), ("dev", "latest"), ("dev", "sha256:" + "f" * 63)])
def test_unknown_recipe_or_nonimmutable_base_is_refused(source: Path, name: str, base: str) -> None:
    with pytest.raises(ValueError):
        inputkey.derived_key(name, base, root=source)


@pytest.mark.parametrize("section,field,value", [
    (("reference_image",), "repository", "ghcr.io/foreign/dev"),
    (("reference_image",), "context", "images/agy"),
    (("reference_image",), "base_context", "images/foreign"),
    (("native_images", "base"), "context", "images/foreign-base"),
    (("native_images", "base"), "repository", "ghcr.io/foreign/base"),
])
def test_native_declaration_cannot_diverge_from_original_workflow_producer(
    source: Path, section: tuple[str, ...], field: str, value: str,
) -> None:
    path = source / "config/gate.toml"
    data = tomllib.loads(path.read_text())
    selected = data["functional"]
    for component in section:
        selected = selected[component]
    selected[field] = value
    path.write_text(tomli_w.dumps(data))
    with pytest.raises((ValueError, GateError), match=r"producer|fixed original owner"):
        key(source)


def test_unused_valid_gate_and_guest_runtime_data_are_not_artifact_identity(source: Path) -> None:
    before = key(source)
    path = source / "config/gate.toml"
    data = tomllib.loads(path.read_text())
    data["package"]["preinstall_resource"]["timeout_seconds"] = 14
    path.write_text(tomli_w.dumps(data))
    assert key(source) == before
    path = source / "config/docker/image/build.toml"
    data = tomllib.loads(path.read_text())
    data["build"]["kernel"]["version"] = "6.18.45"
    path.write_text(tomli_w.dumps(data))
    assert key(source) == before

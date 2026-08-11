"""The package rail builds one architecture, and proves which package it built.

The rail's sharpest rule is that it publishes the package *this run* produced.
`dist/` accumulates, so globbing it would let a package built from a different
commit be proved, installed, and shipped -- which is why the builder writes the
basename it created and this reads it back rather than looking around.
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import lzma
import os
import re
import shutil
import subprocess
import tarfile
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import crosscompile
from capsem.gate.content import ProfileContent
from capsem.gate.errors import GateError
from capsem.gate.packageinputs import pinned_toolchain, resolve_channel
from capsem.gate.packagerail import PackageRail
from capsem.gate.packagesigning import signing_key
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
BUILD_SCRIPT = CONFIG.package.build_script
TARGET = CONFIG.arch("arm64")
PACKAGE = "Capsem_9.9.9_arm64.deb"


def _asset_manifest(*arches: str) -> str:
    releases = {arch: {} for arch in arches}
    return (
        '{"format":2,"refresh_policy":"24h","assets":{"current":"test",'
        '"releases":{"test":{"arches":'
        + __import__("json").dumps(releases, sort_keys=True)
        + "}}}}"
    )


def _checkout(tmp_path: Path, *, toolchain: str = "9.99.9") -> Path:
    """A fake checkout carrying the real gate configuration.

    The rail reads `config/gate.toml` for volume names and scripts, so the
    fixture links it rather than inventing a second copy that could drift from
    the one the gate actually runs with.
    """
    tmp_path.mkdir(parents=True, exist_ok=True)
    (tmp_path / CONFIG.package.toolchain_pin).write_text(f'[toolchain]\nchannel = "{toolchain}"\n')
    (tmp_path / "scripts").mkdir()
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    for name in CONFIG.package.builder.identity_inputs:
        destination = tmp_path / name
        if destination.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT_ROOT / name, destination)
    for pattern in CONFIG.package.builder.identity_globs:
        for source in PROJECT_ROOT.glob(pattern):
            destination = tmp_path / source.relative_to(PROJECT_ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    (tmp_path / "assets" / TARGET.name).mkdir(parents=True)
    manifest = _asset_manifest(TARGET.name)
    (tmp_path / "assets" / CONFIG.install.manifest_name).write_text(manifest)
    config_root = tmp_path / CONFIG.functional.config_root
    (config_root / CONFIG.functional.profiles_subdir / "code").mkdir(parents=True)
    (config_root / CONFIG.functional.profiles_subdir / "code" / "profile.toml").write_text(
        'id = "code"\n'
    )
    config_manifest = config_root / CONFIG.suites.pytest.test_manifest
    config_manifest.parent.mkdir(parents=True, exist_ok=True)
    config_manifest.write_text(manifest)
    return tmp_path


class Building(RecordingRunner):
    """A runner whose builder container writes the package record."""

    def __init__(self, root: Path, *, records: str | None = PACKAGE, **kwargs) -> None:
        super().__init__(root, **kwargs)
        self._records = records

    def execute(self, command):
        completed = super().execute(command)
        if BUILD_SCRIPT in str(command) and self._records is not None:
            (self.root / "dist").mkdir(exist_ok=True)
            (self.root / "dist" / f".cross-compile-{TARGET.name}-deb").write_text(
                self._records + "\n"
            )
            target = self.root / "dist" / self._records
            if target.parent == self.root / "dist":
                target.write_text("package bytes")
        return completed


def _run_lane(rail):
    """Every phase of one lane, in the order the plan composes them.

    The rail used to have a `run()` that did all of this behind one `Call`.
    The phases are plan steps now, so a test that wants the whole lane says
    so -- and a test that wants one phase can finally ask for one.
    """
    rail.release_rails()
    rail.reserve()
    rail.sync_clock()
    rail.require_content()
    rail.materialize()
    rail.build()
    package = rail.resolve()
    rail.prove()
    rail.collect()
    return package


def _rail(runner: RecordingRunner, **kwargs) -> PackageRail:
    return PackageRail(
        runner,
        TARGET,
        content=ProfileContent.standalone(gate_config.load(runner.root)),
        **kwargs,
    )


# ---------------------------------------------------------------------------
# One typed content bundle
# ---------------------------------------------------------------------------


def test_profile_content_derives_both_trees_from_one_root_without_reading_it(
    tmp_path: Path,
) -> None:
    missing = tmp_path / "not-materialized-yet"

    content = ProfileContent.isolated(CONFIG, missing)

    assert content.root == missing
    assert content.assets == missing / CONFIG.assets.merged_assets_dir
    assert content.config == missing / CONFIG.assets.merged_config_dir
    assert content.profiles(CONFIG) == content.config / CONFIG.functional.profiles_subdir


@pytest.mark.parametrize("relative", [Path("/absolute"), Path("../sibling")])
def test_profile_content_refuses_a_path_outside_its_root(tmp_path: Path, relative: Path) -> None:
    with pytest.raises(ValueError, match="relative path under"):
        ProfileContent(tmp_path, relative, Path("config"))


def test_profile_content_completeness_is_explicitly_target_scoped(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    content = ProfileContent.standalone(gate_config.load(root))

    content.require_complete(gate_config.load(root), arches=(TARGET,))

    with pytest.raises(GateError, match="x86_64"):
        content.require_complete(gate_config.load(root))


def test_profile_content_refuses_an_architecture_not_declared_by_the_manifest(
    tmp_path: Path,
) -> None:
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    undeclared = config.arch("x86_64")
    (root / "assets" / undeclared.name).mkdir()

    with pytest.raises(GateError, match=r"manifest.*x86_64"):
        ProfileContent.standalone(config).require_complete(config, arches=(undeclared,))


def _assert_no_package_docker_started(runner: RecordingRunner) -> None:
    assert not any(command.argv[:2] == ("docker", "build") for command in runner.commands)
    assert not any(command.argv[:2] == ("docker", "create") for command in runner.commands)
    assert not any("Materializing locked package dependencies" in note for note in runner.notes)


def test_standalone_package_refuses_the_relative_assets_selector_before_docker(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    selected = root / "selected-assets"
    (root / "assets").rename(selected)
    (root / "assets").symlink_to(selected.name, target_is_directory=True)
    runner = Building(root, replies={"select-linux": "skip"})

    with pytest.raises(GateError, match=r"assets.*symlink"):
        _run_lane(_rail(runner))

    assert (root / "assets").readlink() == Path(selected.name)
    _assert_no_package_docker_started(runner)


def test_explicit_package_content_refuses_a_symlink_root_before_docker(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    checkout = _checkout(tmp_path / "checkout")
    config = gate_config.load(checkout)
    real = tmp_path / "real-content"
    real.mkdir()
    selected = tmp_path / "selected-content"
    selected.symlink_to(real.name, target_is_directory=True)
    runner = Building(checkout, replies={"select-linux": "skip"})
    rail = PackageRail(runner, TARGET, content=ProfileContent.isolated(config, selected))

    with pytest.raises(GateError, match=r"root.*symlink"):
        _run_lane(rail)

    _assert_no_package_docker_started(runner)


def test_package_content_refuses_a_symlink_config_directory_before_docker(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    selected = root / "selected-config"
    (root / config.functional.config_root).rename(selected)
    (root / config.functional.config_root).symlink_to(
        Path("..") / selected.name, target_is_directory=True
    )
    runner = Building(root, replies={"select-linux": "skip"})

    with pytest.raises(GateError, match=r"config.*symlink"):
        _run_lane(_rail(runner))

    _assert_no_package_docker_started(runner)


def test_package_mounts_only_the_concrete_paired_content_dirs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    isolated = root / "target" / "ironbank-assets" / "code"
    content = ProfileContent.isolated(config, isolated)
    content.assets.mkdir(parents=True)
    content.config.mkdir(parents=True)
    manifest = _asset_manifest(TARGET.name)
    (content.assets / TARGET.name).mkdir()
    (content.assets / config.install.manifest_name).write_text(manifest)
    profile = content.profiles(config) / "code" / "profile.toml"
    profile.parent.mkdir(parents=True)
    profile.write_text('id = "code"\n')
    config_manifest = content.config / config.suites.pytest.test_manifest
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_text(manifest)

    # This is the exact retained-prefix shape that Docker's `-v <prefix>/assets`
    # destroyed: a relative selector plus an older canonical config tree.  The
    # package must not even inspect or normalize either one.
    stale_assets = root / "stale-canonical-assets"
    (root / "assets").rename(stale_assets)
    (root / "assets").symlink_to(stale_assets.name)
    selector_inode = (root / "assets").lstat().st_ino
    selector_target = (root / "assets").readlink()
    sentinel = root / config.functional.config_root / "stale-sentinel"
    sentinel.write_bytes(b"canonical config must survive")
    runner = Building(root, replies={"select-linux": "skip"})

    rail = PackageRail(runner, TARGET, content=content)
    rail.require_content()
    rail.build()

    create = runner.matching(r"docker create")[0]
    assert f"{content.assets}:/src/assets:ro" in create
    assert f"{content.config}:/src/target/config:ro" in create
    assert f"{root / 'assets'}:/src/assets" not in create
    assert f"{root / 'target' / 'config'}:/src/target/config" not in create
    assert (root / "assets").lstat().st_ino == selector_inode
    assert (root / "assets").readlink() == selector_target
    assert sentinel.read_bytes() == b"canonical config must survive"


# ---------------------------------------------------------------------------
# Inputs read rather than repeated
# ---------------------------------------------------------------------------


def test_the_toolchain_comes_from_the_file_that_pins_it(tmp_path: Path) -> None:
    """It was spelled three times inside one inline shell script -- three
    chances for a toolchain bump to leave the package rail behind."""
    root = _checkout(tmp_path, toolchain="1.2.3")

    assert pinned_toolchain(root) == "1.2.3"


def test_a_checkout_with_no_pinned_toolchain_says_so(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    (root / CONFIG.package.toolchain_pin).write_text("[other]\n")

    with pytest.raises(GateError, match=r"no .toolchain. channel"):
        pinned_toolchain(root)


def test_release_keys_are_used_when_the_checkout_has_them(tmp_path: Path) -> None:
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")
    (tmp_path / "private" / "tauri" / "password.txt").write_text("PASS")

    assert signing_key(tmp_path, CONFIG) == {
        "TAURI_SIGNING_PRIVATE_KEY": "KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "PASS",
    }


def test_a_checkout_without_keys_injects_none(tmp_path: Path) -> None:
    """The container then makes a throwaway dev key. The authoritative keys
    live in Actions secrets and are applied only on publish."""
    (tmp_path / "private" / "tauri").mkdir(parents=True)
    (tmp_path / "private" / "tauri" / "capsem.key").write_text("KEY")

    assert signing_key(tmp_path, CONFIG) == {}


def test_release_signing_keys_can_arrive_through_the_configured_ci_environment(
    tmp_path: Path,
) -> None:
    variables = CONFIG.package.signing
    environment = {variables.key_variable: "CI-KEY", variables.password_variable: "CI-PASS"}

    assert signing_key(tmp_path, CONFIG, environment=environment) == environment


def test_a_half_exported_release_signing_environment_is_refused(tmp_path: Path) -> None:
    variables = CONFIG.package.signing

    with pytest.raises(GateError, match=r"both.*signing"):
        signing_key(tmp_path, CONFIG, environment={variables.key_variable: "CI-KEY"})


@pytest.mark.parametrize("channel", CONFIG.package.channels)
def test_known_channels_are_accepted(channel: str) -> None:
    assert resolve_channel(channel, CONFIG) == channel


def test_an_unknown_channel_is_refused_before_anything_is_built() -> None:
    with pytest.raises(GateError, match="stable, nightly, corp"):
        resolve_channel("prod", CONFIG)


# ---------------------------------------------------------------------------
# The build
# ---------------------------------------------------------------------------


def test_package_helper_inputs_and_ort_are_config_authoritative() -> None:
    builder = CONFIG.package.builder

    assert builder.materialize_build_network == "default"
    assert builder.source_build_network == "none"
    assert builder.runtime_network == "none"
    assert builder.apt_snapshot_base.startswith("https://snapshot.ubuntu.com/")
    assert re.fullmatch(r"[0-9]{8}T[0-9]{6}Z", builder.apt_snapshot_id)
    assert builder.cargo_store.startswith("/opt/capsem/")
    assert builder.pnpm_store.startswith("/opt/capsem/")
    assert set(builder.targets) == set(CONFIG.architectures)
    for name, target in builder.targets.items():
        assert target.ort_url.startswith("https://cdn.pyke.io/")
        assert len(target.ort_sha256) == 64
        int(target.ort_sha256, 16)
        assert target.ort_url.endswith(f"{CONFIG.arch(name).rust_target}.tar.lzma2")


@pytest.mark.parametrize(
    ("field", "invalid"),
    [
        ("materialize_build_network", "bridge"),
        ("materialize_build_network", "host"),
        ("materialize_build_network", "none"),
        ("source_build_network", "bridge"),
        ("source_build_network", "default"),
        ("source_build_network", "host"),
        ("runtime_network", "bridge"),
        ("runtime_network", "default"),
        ("runtime_network", "host"),
    ],
)
def test_package_network_vocabularies_are_schema_bound(
    tmp_path: Path, field: str, invalid: str
) -> None:
    root = _checkout(tmp_path)
    source = root / "config" / "gate.toml"
    text = source.read_text(encoding="utf-8")
    current = {
        "materialize_build_network": "default",
        "source_build_network": "none",
        "runtime_network": "none",
    }[field]
    source.write_text(
        text.replace(f'{field} = "{current}"', f'{field} = "{invalid}"', 1),
        encoding="utf-8",
    )

    with pytest.raises(GateError, match=field):
        gate_config.load(root)


def test_package_helper_materializes_locked_inputs_and_runtime_is_offline() -> None:
    builder = CONFIG.package.builder
    dockerfile = (PROJECT_ROOT / builder.dockerfile).read_text(encoding="utf-8")
    script = (PROJECT_ROOT / CONFIG.package.build_script).read_text(encoding="utf-8")

    assert "ENV RUSTUP_AUTO_INSTALL=0" in dockerfile
    assert 'grep -F "${selected}-"' in dockerfile
    assert 'rustup target list --toolchain "${selected}" --installed' in dockerfile
    assert dockerfile.count("cargo fetch --locked --target") == 2
    assert 'cargo fetch --locked --target "${RUST_TARGET}"' in dockerfile
    assert 'cargo fetch --locked --target "${HOST_RUST_TARGET}"' in dockerfile
    assert "ARG HOST_RUST_TARGET" in dockerfile
    assert "pnpm fetch --frozen-lockfile" in dockerfile
    assert "frontend/pnpm-workspace.yaml" in builder.identity_inputs
    assert "frontend/pnpm-workspace.yaml" in dockerfile
    assert "ORT_STRATEGY=system" in dockerfile
    assert "ORT_LIB_LOCATION" in dockerfile
    assert "materialize-package-ort.py" in dockerfile
    assert "cargo build" not in dockerfile
    assert "COPY --from=dependency-fetch /cargo-target" not in dockerfile
    assert 'test -n "${APT_SNAPSHOT_BASE}"' in dockerfile
    assert 'test -n "${APT_SNAPSHOT_ID}"' in dockerfile
    assert 'swap-dev-libs "${DPKG_ARCH}" "${APT_SNAPSHOT_BASE}" "${APT_SNAPSHOT_ID}"' in dockerfile
    assert "COPY --chmod=555 docker/swap-dev-libs.sh /usr/local/bin/swap-dev-libs" in dockerfile
    assert "ARG INPUT_IDENTITY" in dockerfile
    assert "org.capsem.package-builder.input-key=${INPUT_IDENTITY}" in dockerfile
    assert "ARG INPUT_KEY" not in dockerfile

    swap = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text(encoding="utf-8")
    assert 'SNAPSHOT_URL="${APT_SNAPSHOT_BASE%/}/${APT_SNAPSHOT_ID}"' in swap
    assert 'APT_SNAPSHOT_BASE="${2:?' in swap
    assert 'APT_SNAPSHOT_ID="${3:?' in swap
    assert "inherited apt sources" not in swap
    assert "archive.ubuntu.com" not in swap
    assert "security.ubuntu.com" not in swap
    assert "ports.ubuntu.com" not in swap

    assert "rustup toolchain install" not in script
    assert "rustup target add" not in script
    assert 'grep -F "$RUST_TOOLCHAIN-"' in script
    assert "swap-dev-libs" not in script
    assert "apt-get" not in script
    assert "pnpm install --offline --frozen-lockfile" in script
    assert script.count("cargo build --release --locked --offline") == 2
    assert "cargo tauri build" in script
    assert "--locked --offline" in script
    assert '"${CARGO_HOME:?}"' in script
    assert '"${CAPSEM_PNPM_STORE:?}"' in script


def test_package_helper_final_stage_contains_only_materialized_dependency_stores() -> None:
    builder = CONFIG.package.builder
    dockerfile = (PROJECT_ROOT / builder.dockerfile).read_text(encoding="utf-8")
    stages = dockerfile.split("FROM ${BASE}")

    assert len(stages) == 3
    fetch, final = stages[1:]
    assert " AS dependency-fetch" in fetch.splitlines()[0]
    assert "COPY crates /prefetch/crates" in fetch
    assert "COPY crates" not in final
    assert "COPY ." not in final
    assert "COPY --from=dependency-fetch /capsem-deps/cargo/registry" in final
    assert "COPY --from=dependency-fetch /capsem-deps/cargo/git" in final
    assert "COPY --from=dependency-fetch /capsem-deps/pnpm" in final
    assert "COPY --from=dependency-fetch /capsem-deps/ort" in final
    assert "ENV CARGO_HOME=${CARGO_STORE}" in final
    assert "ENV CAPSEM_PNPM_STORE=${PNPM_STORE}" in final

    script = (PROJECT_ROOT / CONFIG.package.build_script).read_text(encoding="utf-8")
    assert '--store-dir "$CAPSEM_PNPM_STORE"' in script


def test_source_only_bytes_do_not_change_the_dependency_helper_input_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate.docker import Docker
    from capsem.gate.packagebuilder import image_tag

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    source = root / "crates" / "capsem" / "src" / "main.rs"
    source.parent.mkdir(parents=True)
    source.write_text("fn main() {}\n", encoding="utf-8")
    docker = Docker(RecordingRunner(root))
    before = image_tag(config, config.arch("arm64"), docker)

    source.write_text('fn main() { println!("changed"); }\n', encoding="utf-8")

    assert image_tag(config, config.arch("arm64"), docker) == before


def _stubbed_swap(tmp_path: Path, *, native: str, remove_status: int = 0):
    apt_root = tmp_path / "apt"
    (apt_root / "sources.list.d").mkdir(parents=True)
    apt_lists = tmp_path / "apt-lists"
    apt_lists.mkdir()
    script = tmp_path / "swap-dev-libs.sh"
    source = (PROJECT_ROOT / "docker/swap-dev-libs.sh").read_text(encoding="utf-8")
    script.write_text(
        source.replace("/etc/apt", str(apt_root)).replace("/var/lib/apt/lists", str(apt_lists)),
        encoding="utf-8",
    )
    script.chmod(0o755)
    binary = tmp_path / "bin"
    binary.mkdir()
    log = tmp_path / "commands.log"
    dpkg = binary / "dpkg"
    dpkg.write_text(
        "#!/bin/sh\n"
        'printf \'dpkg %s\\n\' "$*" >> "$CAPSEM_STUB_LOG"\n'
        'if [ "$1" = --print-architecture ]; then printf \'%s\\n\' "$CAPSEM_STUB_NATIVE"; fi\n',
        encoding="utf-8",
    )
    dpkg.chmod(0o755)
    apt_get = binary / "apt-get"
    apt_get.write_text(
        "#!/bin/sh\n"
        'printf \'apt-get %s\\n\' "$*" >> "$CAPSEM_STUB_LOG"\n'
        'case "$1" in\n'
        '  update) exit "${CAPSEM_STUB_UPDATE_STATUS:-0}" ;;\n'
        '  remove) exit "${CAPSEM_STUB_REMOVE_STATUS:-0}" ;;\n'
        '  install) exit "${CAPSEM_STUB_INSTALL_STATUS:-0}" ;;\n'
        "esac\n",
        encoding="utf-8",
    )
    apt_get.chmod(0o755)
    environment = {
        **os.environ,
        "PATH": f"{binary}:/usr/bin:/bin",
        "CAPSEM_STUB_LOG": str(log),
        "CAPSEM_STUB_NATIVE": native,
        "CAPSEM_STUB_REMOVE_STATUS": str(remove_status),
    }

    def run(*arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", str(script), *arguments],
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )

    return run, log, apt_root / "sources.list.d" / "capsem-snapshot.sources"


@pytest.mark.parametrize(
    "arguments",
    [
        ("arm64",),
        ("arm64", "https://snapshot.ubuntu.com/ubuntu"),
    ],
)
def test_package_swap_refuses_missing_snapshot_authority_before_apt(
    tmp_path: Path, arguments: tuple[str, ...]
) -> None:
    run, log, sources = _stubbed_swap(tmp_path, native="arm64")

    result = run(*arguments)

    assert result.returncode != 0
    assert not log.exists()
    assert not sources.exists()


def test_native_package_swap_reinstalls_dev_libraries_from_the_snapshot(tmp_path: Path) -> None:
    run, log, sources = _stubbed_swap(tmp_path, native="arm64")

    result = run("arm64", "https://snapshot.ubuntu.com/ubuntu", "20260810T000000Z")

    assert result.returncode == 0, result.stderr
    recorded = log.read_text(encoding="utf-8")
    assert recorded.index("apt-get update -qq") < recorded.index("apt-get install")
    assert "apt-get remove" not in recorded
    assert "--reinstall" in recorded
    assert "--allow-downgrades" in recorded
    assert "libssl-dev:arm64" in recorded
    assert "https://snapshot.ubuntu.com/ubuntu/20260810T000000Z" in sources.read_text()


def test_foreign_package_swap_updates_then_removes_then_installs(tmp_path: Path) -> None:
    run, log, _sources = _stubbed_swap(tmp_path, native="amd64")

    result = run("arm64", "https://snapshot.ubuntu.com/ubuntu", "20260810T000000Z")

    assert result.returncode == 0, result.stderr
    recorded = log.read_text(encoding="utf-8")
    assert recorded.index("apt-get update -qq") < recorded.index("apt-get remove")
    assert recorded.index("apt-get remove") < recorded.index("apt-get install")
    assert "libssl-dev:arm64" in recorded


def test_foreign_package_swap_stops_when_native_removal_fails(tmp_path: Path) -> None:
    run, log, _sources = _stubbed_swap(tmp_path, native="amd64", remove_status=19)

    result = run("arm64", "https://snapshot.ubuntu.com/ubuntu", "20260810T000000Z")

    assert result.returncode == 19
    recorded = log.read_text(encoding="utf-8")
    assert "apt-get remove" in recorded
    assert "apt-get install" not in recorded


def _ort_materializer():
    path = PROJECT_ROOT / CONFIG.package.builder.ort_script
    spec = importlib.util.spec_from_file_location("package_ort_materializer", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _ort_archive(name: str, payload: bytes) -> bytes:
    tar = io.BytesIO()
    with tarfile.open(fileobj=tar, mode="w") as archive:
        member = tarfile.TarInfo(name)
        member.size = len(payload)
        archive.addfile(member, io.BytesIO(payload))
    return lzma.compress(
        tar.getvalue(),
        format=lzma.FORMAT_RAW,
        filters=[{"id": lzma.FILTER_LZMA2, "dict_size": 1 << 26}],
    )


def test_ort_materializer_verifies_and_extracts_only_the_static_distribution(
    tmp_path: Path,
) -> None:
    materializer = _ort_materializer()
    payload = _ort_archive("libonnxruntime.a", b"static-ort")
    source = tmp_path / "source.lzma2"
    source.write_bytes(payload)
    downloaded = tmp_path / "downloaded.lzma2"
    tar = tmp_path / "archive.tar"
    output = tmp_path / "ort"

    materializer._download(source.as_uri(), hashlib.sha256(payload).hexdigest(), downloaded)
    materializer._decompress(downloaded, tar)
    materializer._extract(tar, output)

    assert (output / "libonnxruntime.a").read_bytes() == b"static-ort"
    assert (output / "libonnxruntime.a").stat().st_mode & 0o777 == 0o444


def test_ort_materializer_rejects_an_archive_path_escape(tmp_path: Path) -> None:
    materializer = _ort_materializer()
    compressed = tmp_path / "escape.lzma2"
    compressed.write_bytes(_ort_archive("../escaped", b"no"))
    tar = tmp_path / "escape.tar"
    materializer._decompress(compressed, tar)

    with pytest.raises(ValueError, match="escapes its root"):
        materializer._extract(tar, tmp_path / "output")

    assert not (tmp_path / "escaped").exists()


@pytest.mark.parametrize(
    ("host_name", "target_name"),
    (("x86_64", "arm64"), ("arm64", "x86_64")),
)
def test_package_helper_is_host_native_and_target_specific(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    host_name: str,
    target_name: str,
) -> None:
    from capsem.gate.packagebuilder import materialize

    monkeypatch.setattr("capsem.gate.host.machine", lambda: host_name)
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    host = config.arch(host_name)
    target = config.arch(target_name)
    runner = RecordingRunner(
        root,
        failures=(
            f"docker image inspect --platform {host.docker_platform} "
            f"capsem-package-builder-{target.name}:",
        ),
        replies={
            f"--platform {host.docker_platform} --format '{{{{.Id}}}}' "
            f"capsem-package-builder-{target.name}": ("sha256:" + "d" * 64),
            f"--format '{{{{.Id}}}}' capsem-package-builder-{target.name}": ("sha256:" + "e" * 64),
        },
    )

    identity = materialize(runner, config, target)

    build_command = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.package-builder") for value in command.argv)
    )
    build = str(build_command)
    assert f"--platform {host.docker_platform}" in build
    assert "--network default" in build
    assert f"BASE=capsem-host-builder@sha256:{'0' * 64}" in build
    assert f"BASE=sha256:{'0' * 64}" not in build
    assert f"RUST_TARGET={target.rust_target}" in build
    assert f"HOST_RUST_TARGET={host.rust_target}" in build
    assert f"DPKG_ARCH={target.dpkg}" in build
    assert f"APT_SNAPSHOT_BASE={config.package.builder.apt_snapshot_base}" in build
    assert f"APT_SNAPSHOT_ID={config.package.builder.apt_snapshot_id}" in build
    assert f"CARGO_STORE={config.package.builder.cargo_store}" in build
    assert f"PNPM_STORE={config.package.builder.pnpm_store}" in build
    assert CONFIG.package.builder.targets[target.name].ort_sha256 in build
    assert f"INPUT_IDENTITY=capsem-package-builder-{target.name}:" in build
    assert "INPUT_KEY=" not in build
    assert any("sha256:" in note for note in runner.notes)
    assert identity.input_key.startswith(f"capsem-package-builder-{target.name}:")
    assert identity.image_id == "sha256:" + "d" * 64
    assert identity.image_reference == (f"capsem-package-builder-{target.name}@sha256:{'0' * 64}")
    from capsem.gate.invocation import ConsoleMode

    assert build_command.console is ConsoleMode.LOG_ONLY


def test_package_helper_key_uses_the_platform_child_not_the_provenance_index(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate.docker import Docker
    from capsem.gate.packagebuilder import image_tag

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    child = "sha256:" + "c" * 64

    class Identity(Docker):
        def __init__(self, provenance_index: str) -> None:
            self.provenance_index = provenance_index

        def image_id(self, tag: str, *, platform: str | None = None) -> str:
            del tag
            return child if platform == "linux/amd64" else self.provenance_index

    assert image_tag(config, config.arch("x86_64"), Identity("sha256:" + "1" * 64)) == image_tag(
        config, config.arch("x86_64"), Identity("sha256:" + "2" * 64)
    )


def test_package_helper_refuses_a_parent_tag_move_before_docker_build(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate.packagebuilder import materialize

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    runner = RecordingRunner(
        root,
        replies={
            "--platform linux/amd64 --format '{{.Id}}' capsem-host-builder:latest": (
                "sha256:" + "a" * 64
            ),
            "--platform linux/amd64 --format '{{.Id}}' capsem-host-builder@sha256:": (
                "sha256:" + "c" * 64
            ),
        },
    )

    with pytest.raises(GateError, match="moved while resolving"):
        materialize(runner, config, config.arch("x86_64"))

    assert not runner.ran(r"docker build")


@pytest.mark.parametrize(
    "changed_authority",
    [
        {"apt_snapshot_id": "20260811T000000Z"},
        {"apt_snapshot_base": "https://snapshot.example.invalid/ubuntu"},
    ],
)
def test_package_helper_snapshot_authority_changes_the_input_key(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    changed_authority: dict[str, str],
) -> None:
    from capsem.gate.docker import Docker
    from capsem.gate.packagebuilder import image_tag

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    changed_builder = config.package.builder.model_copy(update=changed_authority)
    changed = config.model_copy(
        update={"package": config.package.model_copy(update={"builder": changed_builder})}
    )
    docker = Docker(RecordingRunner(root))

    assert image_tag(config, config.arch("arm64"), docker) != image_tag(
        changed, changed.arch("arm64"), docker
    )


def test_package_helper_reuses_a_matching_warm_snapshot_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate.packagebuilder import materialize

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    runner = RecordingRunner(root)

    identity = materialize(runner, config, config.arch("arm64"))

    assert identity.input_key.startswith("capsem-package-builder-arm64:")
    assert runner.ran(r"index \.Config\.Labels")
    assert not runner.ran(r"docker build.*Dockerfile\.package-builder")


def test_package_helper_refuses_a_warm_tag_with_the_wrong_input_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem.gate.packagebuilder import materialize

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    runner = RecordingRunner(root, replies={"index .Config.Labels": "forged-input-key"})

    with pytest.raises(GateError, match="poisoned warm tag"):
        materialize(runner, config, config.arch("arm64"))


def test_package_helper_exact_identity_is_written_to_the_run_journal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from helpers.gate import RecordingJournal

    from capsem.gate.context import Context

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    root = _checkout(tmp_path)
    config = gate_config.load(root)
    runner = RecordingRunner(root)
    journal = RecordingJournal()

    crosscompile._phase(config.arch("arm64"), "materialize", ProfileContent.standalone(config))(
        Context(runner, config, journal=journal)
    )

    (recorded,) = [note for note in journal.notes if note.startswith("package helper arm64:")]
    assert "input key capsem-package-builder-arm64:" in recorded
    assert "exact image sha256:" in recorded
    assert "immutable reference capsem-package-builder-arm64@sha256:" in recorded


def test_package_source_image_and_runtime_are_network_none(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _rail(runner).build()

    source = runner.matching(r"docker build.*Dockerfile\.package")[0]
    runtime = runner.matching(r"docker create")[0]
    assert "--network none" in source
    assert "--network none" in runtime
    assert "--platform linux/arm64" in source
    assert "BASE=capsem-package-builder-arm64:" in source
    assert "@sha256:" not in source
    assert "BASE=sha256:" not in source
    from capsem.gate.invocation import ConsoleMode

    source_command = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.package") for value in command.argv)
    )
    runtime_command = next(
        command for command in runner.commands if command.argv[:3] == ("docker", "start", "-a")
    )
    assert source_command.console is ConsoleMode.LOG_ONLY
    assert runtime_command.console is ConsoleMode.LOG_ONLY


def test_the_package_dockerfile_waives_only_its_required_base_check() -> None:
    """BuildKit cannot infer the base which the package graph must supply.

    A default would make an uncomposed direct build silently consume a mutable
    or stale image.  Keep the argument required and waive only the generic
    check whose premise is that every Dockerfile must build with no arguments.
    Parser directives must be the first physical line to take effect.
    """
    lines = (PROJECT_ROOT / CONFIG.package.lane_dockerfile).read_text(encoding="utf-8").splitlines()

    assert lines[0] == "# check=skip=InvalidDefaultArgInFrom"
    assert "ARG BASE" in lines
    assert not any(line.startswith("ARG BASE=") for line in lines)
    assert "FROM ${BASE}" in lines


def test_the_package_lane_supplies_its_verified_local_input_key_helper(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The targeted Dockerfile waiver is safe only while this is indivisible."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith(f"/{CONFIG.package.lane_dockerfile}") for value in command.argv)
    )
    supplied = [
        build.argv[index + 1] for index, value in enumerate(build.argv) if value == "--build-arg"
    ]
    assert len(supplied) == 1
    assert supplied[0].startswith("BASE=capsem-package-builder-arm64:")
    assert "@sha256:" not in supplied[0]


def test_the_builder_receives_every_name_for_the_target(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path, toolchain="1.2.3"), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    # Forwarded by name, never as `NAME=value`: the same argv carries the
    # Tauri signing key, and a value in argv is world-readable through `ps`.
    # The values are asserted on the recorded environment instead.
    for name in ("TARGET_ARCH", "RUST_TARGET", "DPKG_ARCH", "RUST_TOOLCHAIN", "PKG_CONFIG_PATH"):
        assert f"-e {name}" in build, f"{name} is not handed to the builder"

    created = next(c for c in runner.commands if c.argv[:2] == ("docker", "create"))
    assert created.env["TARGET_ARCH"] == TARGET.name
    assert created.env["RUST_TARGET"] == TARGET.rust_target
    assert created.env["DPKG_ARCH"] == TARGET.dpkg
    assert created.env["RUST_TOOLCHAIN"] == "1.2.3"
    assert created.env["PKG_CONFIG_PATH"] == TARGET.pkg_config_path
    assert f"bash /src/{BUILD_SCRIPT}" in build


def test_the_lane_shares_no_named_volume_with_any_other_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A shared /cargo-target across architectures would rebuild the world on
    every alternation; a per-architecture registry would refetch the index."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    # Reimplemented from a test that required exactly the opposite, and was
    # right at the time: the cargo caches were shared named volumes and the
    # target dir was one per architecture.
    #
    # They mounted over `/usr/local/cargo` and `/usr/local/rustup`, which is
    # where `Dockerfile.host-builder` installs the toolchain, the cross-targets
    # and its tools -- so the image carried all of it and the container saw a
    # volume instead.
    #
    # The build directory is still off the host filesystem; it is simply
    # anonymous, so Docker allocates one per container and reclaims it with
    # that container rather than carrying it between two gates.
    assert "capsem-cargo-registry" not in build
    assert "capsem-rustup" not in build
    assert "capsem-host-target" not in build
    assert "-v /cargo-target" in build, f"the build directory left the container: {build}"


def test_the_package_lane_mounts_no_git_metadata(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Reimplemented: this asserted the opposite, and correctly so at the time.

    A linked worktree's `.git` is a file pointing into another repository, so
    the common directory had to be mounted at its absolute host path for a
    container to read a revision from it. Nothing reads a repository inside a
    container now -- `.dockerignore` excludes `.git` and the gate passes
    `CAPSEM_BUILD_REVISION` -- so a mount of the host's Git directory would be
    a mount of the checkout by another name.
    """
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    assert "/.git" not in build, f"the lane mounted a Git directory: {build}"


def test_the_builds_own_outputs_stay_container_local(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The build writes into its source; its *outputs* must not reach the host.

    `pnpm install` fills `frontend/node_modules`, `pnpm build` fills
    `frontend/dist`, and Tauri regenerates ACL schemas into the app crate.
    Through a plain mount those are host writes -- how a container and a host
    step came to share inodes and kill a release run on an intermittent EACCES.
    Anonymous volumes grafted over exactly those paths keep them
    container-local.

    The mount itself is still read-write, and this test used to claim
    otherwise. Making it `:ro` failed a real run: the frontend bundler writes
    atomic temporaries beside its target, directly in `frontend/` --
    `EROFS ... open '/src/frontend/_tmp_50_...'` -- and grafting scratch over
    `frontend/` would mask the source being compiled. No flag fixes that;
    baking the frontend into the builder image does, which is Phase 5's second
    half. Asserting only what is true keeps the difference visible.
    """
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    build = runner.matching(r"docker create")[0]
    for path in CONFIG.package.writable_paths:
        assert f"-v /src/{path} " in build + " ", (
            f"{path} is written by the build and has no container-local backing"
        )

    # And the scratch goes when the container does: an anonymous volume has no
    # name, so nothing else could ever collect the 356 MB node_modules one.
    assert runner.matching(r"docker rm -f -v"), "anonymous volumes would accumulate"


def test_the_builder_image_is_rebuilt_before_every_package() -> None:
    """Always rebuilt, and always before the package that runs inside it.

    The claim is unchanged; the evidence moved. The rail used to run `just
    _build-host-image` itself, and this asserted the ordering by watching the
    runner. That recipe never existed -- it has a heading in the justfile and
    no body -- so what this actually proved was that the rail issued a command
    which failed. Watching a runner cannot tell those apart.

    The image is a step now, and the order is an edge, so the assertion is
    about the graph rather than about a sequence of attempts.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import (
        cli,  # noqa: F401 - registers every command
        hostimage,
    )
    from capsem.gate.command import GateCommand

    plan = GateCommand.registry["cross-compile"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, arch=TARGET.name),
    )._describe()
    order = list(plan.labels)

    assert order.index(hostimage.STEP) < order.index(f"package.{TARGET.name}.build")
    # The lane's first phase depends on the image; the rest chain from there.
    # It was one step, so the edge landed on the whole lane -- which is also
    # why nothing could be ordered against a phase inside it.
    assert (hostimage.STEP, f"package.{TARGET.name}.storage-release") in plan.edges


def test_fresh_release_package_plan_owns_helper_prerequisites_in_order() -> None:
    import argparse

    from capsem.gate import cli, hostimage
    from capsem.gate.command import GateCommand

    del cli  # Importing registers every command; the registry is the value used below.

    args = argparse.Namespace(
        dry_run=False,
        graph=False,
        timing=False,
        arch=TARGET.name,
        content_root="target/package-content",
        defer_proof=True,
    )
    plan = GateCommand.registry["cross-compile"](RecordingRunner(PROJECT_ROOT), args)._describe()
    order = list(plan.labels)

    assert order.index(hostimage.STEP) < order.index(f"package.{TARGET.name}.content")
    assert order.index(f"package.{TARGET.name}.content") < order.index(
        f"package.{TARGET.name}.materialize"
    )
    assert order.index(f"package.{TARGET.name}.materialize") < order.index(
        f"package.{TARGET.name}.build"
    )
    assert not any(label.startswith("install-image") for label in order)


def test_the_container_clock_is_synced_only_on_macos(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Colima's VM clock drifts and apt rejects a repository signed in what it
    believes is the future. A Linux runner has no such VM."""
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    for system, expected in (("Darwin", True), ("Linux", False)):
        monkeypatch.setattr("capsem.gate.host.system", lambda system=system: system)
        runner = Building(_checkout(tmp_path / system), replies={"select-linux": "skip"})

        _run_lane(_rail(runner))

        assert runner.ran(r"sync-container-clock\.py") is expected


# ---------------------------------------------------------------------------
# Which package got built
# ---------------------------------------------------------------------------


def test_the_recorded_package_is_the_one_this_run_produced(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    # A package from an earlier build of a different commit, still in dist/.
    (root / "dist").mkdir()
    (root / "dist" / "Capsem_0.0.1_arm64.deb").write_text("stale")
    runner = Building(root, replies={"select-linux": "skip"})

    assert _run_lane(_rail(runner)) == root / "dist" / PACKAGE


def test_a_build_that_recorded_nothing_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), records=None)

    with pytest.raises(GateError, match="did not record the exact Debian package"):
        _run_lane(_rail(runner))


@pytest.mark.parametrize(
    "recorded, reason",
    [
        ("capsem.tar.gz", "invalid Debian package record"),
        ("../outside/capsem.deb", "escaped dist/"),
    ],
)
def test_a_nonsense_package_record_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, recorded: str, reason: str
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), records=recorded)

    with pytest.raises(GateError, match=reason):
        _run_lane(_rail(runner))


def test_the_record_does_not_survive_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Left behind, it would name this run's package to the next one."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    root = _checkout(tmp_path)
    runner = Building(root, replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    assert not (root / "dist" / f".cross-compile-{TARGET.name}-deb").exists()


# ---------------------------------------------------------------------------
# Whether the package gets proved
# ---------------------------------------------------------------------------


def test_a_provable_target_runs_the_systemd_kvm_proof(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    monkeypatch.setattr("capsem.gate.host.device_available", lambda _path: True)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "prove"})

    # The proof is called, not launched: the three `CAPSEM_PROOF_*` variables
    # existed only to carry these arguments across a process boundary that no
    # longer exists, and `DebProof` always took them as arguments. What is
    # asserted is therefore what it was handed, which is the same claim
    # without a subprocess in the middle.
    from capsem.gate import packagerail

    handed = {}

    class Recording:
        def __init__(self, _runner, **kwargs):
            handed.update(kwargs)

        def run(self):
            handed["ran"] = True

    monkeypatch.setattr(packagerail.debproof, "DebProof", Recording)

    _run_lane(_rail(runner, channel="nightly", manifest_url="file:///src/m.json"))

    assert handed.get("ran"), "a provable target did not run the proof"
    assert handed["channel"] == "nightly"
    assert handed["manifest_url"] == "file:///src/m.json"
    assert handed["package"].name == PACKAGE


def test_the_standalone_plan_keeps_the_exact_package_proof() -> None:
    plan = Plan("standalone-package")
    crosscompile.fragment(
        plan, CONFIG, CONFIG.host_arch(), content=ProfileContent.standalone(CONFIG)
    )

    rendered = "\n".join(plan.step_named(f"package.{CONFIG.host_arch().name}.prove").render())

    assert "prove that exact package in systemd + KVM" in rendered
    assert "defer exact package proof" not in rendered


def test_a_cross_target_skips_the_proof_and_says_why(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The decision belongs to `select-linux-deb-proof.sh`; this must not
    second-guess it, or the two disagree about what a green run proved."""
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: TARGET.name)
    runner = Building(_checkout(tmp_path), replies={"select-linux": "skip"})

    _run_lane(_rail(runner))

    assert not runner.ran(r"just _prove-linux-deb")
    assert any("Skipping exact Debian package proof" in note for note in runner.notes)


def test_the_builder_environment_follows_the_configured_names() -> None:
    """A rename in config must move the rail, not silently leave it behind.

    Asserted against a *changed* config rather than the real one: a test that
    reads the same literal the implementation reads passes whether or not the
    implementation reads config at all.
    """
    from capsem.gate.packageinputs import package_environment

    renamed = CONFIG.model_copy(
        update={
            "package": CONFIG.package.model_copy(
                update={"manifest_variable": "CAPSEM_RENAMED_MANIFEST"}
            )
        }
    )
    target = CONFIG.arch(next(iter(CONFIG.architectures)))

    environment = package_environment(
        renamed,
        target,
        toolchain="1.97.1",
        manifest_url="file:///src/assets/local/manifest.json",
        signing={},
        revision="abc1234",
    )

    assert environment["CAPSEM_RENAMED_MANIFEST"] == ("file:///src/assets/local/manifest.json")
    # Told, not discovered: the lane image carries no `.git`, so a builder that
    # asked got `fatal: not a git repository` and the package step died at
    # minute sixty-five of a gate run.
    assert environment[renamed.environment.package.build_revision] == "abc1234"
    assert "CAPSEM_INSTALL_MANIFEST_URL" not in environment


def test_the_builder_environment_carries_the_signing_material_it_was_given() -> None:
    from capsem.gate.packageinputs import package_environment

    target = CONFIG.arch(next(iter(CONFIG.architectures)))
    signing = {CONFIG.package.signing.key_variable: "secret-key-bytes"}

    environment = package_environment(
        CONFIG, target, toolchain="1.97.1", manifest_url="x", signing=signing, revision="abc1234"
    )

    assert environment[CONFIG.package.signing.key_variable] == "secret-key-bytes"


def test_the_disk_rail_is_measured_at_two_different_moments() -> None:
    """Twice, deliberately -- but not twice in the same breath.

    The pair exists because the builder image is itself part of what fills
    this rail: one check once it exists, one immediately before the package
    build spends the headroom. Both calls sat on adjacent lines, so they
    measured the same moment and the second could only ever agree with the
    first. Removing one looked right and would have lost a real check.
    """
    source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "packagerail.py").read_text(
        encoding="utf-8"
    )

    assert source.count('ensure_space("package")') == 2
    build = source.index("def build(self)")
    first = source.index('ensure_space("package")')
    second = source.index('ensure_space("package")', build)

    assert first < build < second, "both checks sit in one method, so they measure a single moment"

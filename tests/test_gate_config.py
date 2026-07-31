"""The gate's data lives in `config/gate.toml` and is validated on load.

Every value the gate works from used to be spelled inside whichever module
happened to need it, which is how the justfile ended up with eleven hand-written
copies of one storage command and four `case` blocks over architecture names
that were each free to disagree. The architecture table is the clearest case:
there is now one record per target and no second representation of it, so
`arm64` cannot mean `amd64` in one file and `arm64` in another.

Validation is why this is Pydantic rather than the dict `tomllib` returns. A
missing key or a mistyped timeout fails here, with the field named, instead of
surfacing forty minutes into a run as a `KeyError` inside a Docker call.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def config() -> gate_config.GateConfig:
    return gate_config.load(PROJECT_ROOT)


# ---------------------------------------------------------------------------
# Loading
# ---------------------------------------------------------------------------


def test_the_checked_in_configuration_is_valid(config: gate_config.GateConfig) -> None:
    assert config.version == 1
    assert config.root == PROJECT_ROOT


def test_an_unknown_key_is_refused_rather_than_ignored(tmp_path: Path) -> None:
    """A typo that silently does nothing is worse than one that fails."""
    source = tmp_path / "config"
    source.mkdir()
    original = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    (source / "gate.toml").write_text(
        original.replace('container = "capsem-install-test"', 'containr = "typo"')
    )

    with pytest.raises(GateError) as failure:
        gate_config.load(tmp_path)

    assert "is invalid" in str(failure.value)
    assert "containr" in str(failure.value), "the offending key must be named"


def test_a_missing_configuration_names_the_file(tmp_path: Path) -> None:
    with pytest.raises(GateError, match="cannot read gate configuration"):
        gate_config.load(tmp_path)


def test_malformed_toml_says_so(tmp_path: Path) -> None:
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text("version = [unclosed\n")

    with pytest.raises(GateError, match="not valid TOML"):
        gate_config.load(tmp_path)


def test_the_configuration_is_parsed_once_per_checkout() -> None:
    assert gate_config.for_root(PROJECT_ROOT) is gate_config.for_root(PROJECT_ROOT)


# ---------------------------------------------------------------------------
# Architectures
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "spelling, expected",
    [
        ("arm64", "arm64"),
        ("aarch64", "arm64"),
        ("AArch64", "arm64"),
        (" arm64 ", "arm64"),
        ("x86_64", "x86_64"),
        ("amd64", "x86_64"),
    ],
)
def test_every_accepted_spelling_reaches_one_record(
    config: gate_config.GateConfig, spelling: str, expected: str
) -> None:
    assert config.arch(spelling).name == expected


def test_intel_is_x86_64_to_capsem_and_amd64_to_dpkg(
    config: gate_config.GateConfig,
) -> None:
    """The distinction four shell `case` blocks each had to remember.

    A copy that used `x86_64` for both would look for a package Debian never
    names that way.
    """
    intel = config.arch("x86_64")
    arm = config.arch("arm64")

    assert (intel.name, intel.dpkg) == ("x86_64", "amd64")
    assert arm.name == arm.dpkg == "arm64"


def test_pkg_config_path_is_derived_from_the_multiarch_tuple(
    config: gate_config.GateConfig,
) -> None:
    assert config.arch("arm64").pkg_config_path == (
        "/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig"
    )


def test_an_unsupported_architecture_names_itself_and_the_alternatives(
    config: gate_config.GateConfig,
) -> None:
    with pytest.raises(GateError) as failure:
        config.arch("riscv64")

    message = str(failure.value)
    assert "riscv64" in message
    assert "arm64" in message and "x86_64" in message


def test_the_host_architecture_resolves_on_this_machine(
    config: gate_config.GateConfig,
) -> None:
    assert config.host_arch().name in config.architectures


def test_every_architecture_carries_the_key_that_names_it(
    config: gate_config.GateConfig,
) -> None:
    for key, arch in config.architectures.items():
        assert arch.name == key


# ---------------------------------------------------------------------------
# Derived values
# ---------------------------------------------------------------------------


def test_owned_paths_cover_the_scratch_the_container_writes(
    config: gate_config.GateConfig,
) -> None:
    """Anything the container writes as its own user must be handed back, or
    the host cannot rebuild without sudo."""
    owned = config.install.layout.owned_paths(config.install.mount)
    layout = config.install.layout

    for scratch in (layout.assets, layout.config, layout.channel, layout.packages):
        assert f"{config.install.mount}/{scratch}" in owned
    assert all(path.startswith(config.install.mount) for path in owned)


def test_the_preinstall_admin_is_not_the_installed_one(
    config: gate_config.GateConfig,
) -> None:
    """It authors a release graph that has to exist before the install, so it
    cannot come from the package being installed."""
    assert not config.install.preinstall_admin.startswith("/usr/bin")
    assert config.install.preinstall_admin.startswith(config.install.preinstall_root)


def test_the_package_target_volume_is_per_architecture(
    config: gate_config.GateConfig,
) -> None:
    """A shared /cargo-target would rebuild the world on every alternation."""
    arm = config.package.target_volume_for("arm64")
    intel = config.package.target_volume_for("x86_64")

    assert arm != intel
    assert "arm64" in arm and "x86_64" in intel


def test_relaxed_lint_roots_are_the_ones_not_checked_strictly(
    config: gate_config.GateConfig,
) -> None:
    assert set(config.lint.strict_roots) <= set(config.lint.python_roots)
    assert set(config.lint.relaxed_roots) == set(config.lint.python_roots) - set(
        config.lint.strict_roots
    )


def test_every_storage_phase_names_a_rail_the_policy_declares(
    config: gate_config.GateConfig,
) -> None:
    """A phase naming a rail that does not exist releases nothing, silently."""
    policy = tomllib.loads(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )

    unknown = sorted(
        {phase.rail for phase in config.storage.phases.values()} - set(policy["rails"])
    )
    assert not unknown, f"storage phases name rails the policy does not declare: {unknown}"


def test_every_phase_declares_a_distinct_boundary_and_rail_pair(
    config: gate_config.GateConfig,
) -> None:
    pairs = [(phase.boundary, phase.rail) for phase in config.storage.phases.values()]

    assert len(set(pairs)) == len(pairs), (
        "two names for one boundary/rail pair means one of them is dead"
    )

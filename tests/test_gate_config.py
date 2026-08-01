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

import re
import tomllib
from pathlib import Path

import pytest
from pydantic import ValidationError

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]


CONFIG = gate_config.load(PROJECT_ROOT)


@pytest.fixture(scope="module")
def config() -> gate_config.GateConfig:
    return CONFIG


def _checkout(tmp_path: Path) -> Path:
    """A tree carrying a copy of the real configuration, for mutating."""
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    return tmp_path


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


# ---------------------------------------------------------------------------
# Contention
# ---------------------------------------------------------------------------


def test_every_exclusive_says_why_it_exists(config: gate_config.GateConfig) -> None:
    """An exclusive without a reason is a serialization nobody can justify
    later, and therefore one nobody can safely remove."""
    for name, exclusive in config.execution.exclusives.items():
        assert exclusive.name == name
        assert len(exclusive.reason.split()) >= 5, (
            f"{name} needs a reason a reader can act on, not a restatement"
        )


def test_an_unknown_exclusive_names_itself_and_the_alternatives(
    config: gate_config.GateConfig,
) -> None:
    """A step that invents its own exclusive contends with nothing, and runs
    beside the step it was written to avoid."""
    with pytest.raises(GateError) as failure:
        config.exclusive("gpu")

    message = str(failure.value)
    assert "gpu" in message
    assert "apple_vz" in message


# ---------------------------------------------------------------------------
# The machine lock
# ---------------------------------------------------------------------------


def test_the_lockfile_lives_outside_every_tree_the_gate_wipes(
    config: gate_config.GateConfig,
) -> None:
    """The run takes the lock and *then* removes CAPSEM_HOME.

    A lockfile inside that tree would be deleted while held, and the next run
    would take a lock on a fresh inode -- two gates, both convinced they were
    alone, one of them deleting the other's home.
    """
    lock = Path(config.locks.gate.path)
    holder = Path(config.locks.gate.holder_record)

    wiped = [Path(entry) for entry in config.disk.reclaimable]
    for tree in wiped:
        assert tree not in lock.parents, f"{lock} sits inside reclaimable {tree}"
        assert tree not in holder.parents, f"{holder} sits inside reclaimable {tree}"


def test_the_lock_waits_long_enough_to_outlast_a_gate_run(
    config: gate_config.GateConfig,
) -> None:
    """A timeout shorter than a run turns queueing into a spurious failure."""
    settings = config.locks.gate

    assert settings.wait_timeout_seconds >= 3600
    assert 0 < settings.report_after_seconds < settings.wait_timeout_seconds


# ---------------------------------------------------------------------------
# The run log and the disk it occupies
# ---------------------------------------------------------------------------


def test_the_run_log_keeps_enough_history_to_compare_against(
    config: gate_config.GateConfig,
) -> None:
    settings = config.runlog

    assert settings.keep_runs >= 2, "one kept run cannot be compared with anything"
    assert settings.keep_bytes > 0
    assert settings.slow_action_seconds > 0


def test_the_run_log_is_itself_reclaimable(config: gate_config.GateConfig) -> None:
    """Rotation bounds it during a run; `gc` has to be able to reclaim the rest."""
    assert config.runlog.root in config.disk.reclaimable


def test_nothing_reclaimable_can_be_aimed_outside_the_checkout(
    config: gate_config.GateConfig,
) -> None:
    """These are whole-tree removals. The difference between a relative path
    and one that escapes upwards is a single editing mistake."""
    for entry in config.disk.reclaimable:
        assert not Path(entry).is_absolute()
        assert ".." not in Path(entry).parts


@pytest.mark.parametrize("escape", ["/etc", "../../elsewhere", "target/../.."])
def test_a_reclaimable_path_that_escapes_is_refused_at_load(
    tmp_path: Path, escape: str
) -> None:
    """Red-first, permanently: the loader must reject the shape it forbids."""
    source = tmp_path / "config"
    source.mkdir()
    original = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    (source / "gate.toml").write_text(
        original.replace('    "target/gate-runs",', f'    "{escape}",')
    )

    with pytest.raises(GateError) as failure:
        gate_config.load(tmp_path)

    assert "escape" in str(failure.value)


def test_the_free_space_floor_exceeds_what_one_run_is_warned_about(
    config: gate_config.GateConfig,
) -> None:
    """Otherwise the gate refuses to start on a footprint it considers normal."""
    assert config.disk.required_free_gb > config.disk.run_footprint_warn_gb


# ---------------------------------------------------------------------------
# Meaning, not just shape
# ---------------------------------------------------------------------------


def test_the_schema_version_is_the_one_this_code_understands(tmp_path) -> None:
    """Pydantic accepted any integer, so a file written for a later schema
    loaded happily and was then read with the wrong meaning."""
    from capsem.gate.errors import GateError

    root = _checkout(tmp_path)
    source = root / "config" / "gate.toml"
    source.write_text(
        source.read_text(encoding="utf-8").replace("version = 1", "version = 2", 1),
        encoding="utf-8",
    )

    with pytest.raises((GateError, ValidationError)):
        gate_config.load(root)


@pytest.mark.parametrize(
    ("field", "value"),
    [("keep_runs", 0), ("keep_bytes", -1), ("slow_action_seconds", -1)],
)
def test_a_retention_policy_that_keeps_nothing_is_refused(
    tmp_path, field: str, value: int
) -> None:
    """`keep_runs = 0` prunes every run including the one being written, and
    the failure surfaces as a missing directory rather than as a bad policy."""
    from capsem.gate.errors import GateError

    root = _checkout(tmp_path)
    source = root / "config" / "gate.toml"
    text = source.read_text(encoding="utf-8")
    replaced = re.sub(rf"^{field} = .*$", f"{field} = {value}", text, count=1, flags=re.M)
    assert replaced != text, f"{field} is not written where this test expects"
    source.write_text(replaced, encoding="utf-8")

    with pytest.raises((GateError, ValidationError)):
        gate_config.load(root)


def test_the_default_channel_is_one_of_the_declared_channels() -> None:
    """Otherwise every release defaults to a channel that does not exist."""
    assert CONFIG.package.default_channel in CONFIG.package.channels


def test_the_base_profile_is_a_checked_in_profile() -> None:
    """The broad suite runs against it, so a name nobody built is a gate that
    proves nothing about anything."""
    from capsem.gate import imagebuild

    assert CONFIG.suites.pytest.base_profile in imagebuild.profiles(CONFIG)


def test_no_two_architectures_claim_the_same_alias() -> None:
    """`uname -m` is resolved through these, so a collision resolves the wrong
    way exactly once and silently."""
    seen: dict[str, str] = {}
    for name, arch in CONFIG.architectures.items():
        for alias in arch.aliases:
            assert alias not in seen, (
                f"{alias!r} is claimed by both {seen[alias]} and {name}"
            )
            seen[alias] = name


def test_every_architecture_knows_its_own_key() -> None:
    """The table key is stamped in at load; a mismatch would make `config.arch`
    hand back something that disagrees with how it was looked up."""
    for name, arch in CONFIG.architectures.items():
        assert arch.name == name

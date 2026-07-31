"""Work goes through the primitives, so the run log and the dry run see it.

`actions` and `fileactions` exist so that a unit of gate work can describe
itself before it runs and be timed while it runs. Both properties are lost the
moment a module calls `shutil.rmtree` or `subprocess.run` directly: the dry run
cannot mention what it does not know about, and the run log cannot record it.

So there are exactly three modules allowed to touch the machine.
`fileactions` is the filesystem primitives themselves. `proc` is the single
funnel every invocation passes through -- the one place the run log has to
hook. `pidfiles` is the one place a signal is sent, deliberately narrow so that
"which process did the gate kill" has one answer.

The rest are on a ratchet while they are extracted. Entries may leave, nothing
may join, and a module that no longer applies must be struck, so the list
cannot quietly describe a past that is no longer true.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE_PACKAGE = PROJECT_ROOT / "src" / "capsem" / "gate"

CONFIG = gate_config.load(PROJECT_ROOT)
BOUNDARY = CONFIG.boundary

# Calls resolvable to their module, so there is no guessing about the receiver.
# Read-only members are deliberately absent: `shutil.which` discovers a tool and
# `os.environ` reads one, and neither changes anything a log would want to know.
QUALIFIED = {
    "shutil": {"rmtree", "copytree", "copy", "copy2", "copyfile", "move"},
    "tempfile": {"mkdtemp", "mkstemp", "TemporaryDirectory"},
    "os": {"replace", "remove", "removedirs", "makedirs", "rename", "unlink", "kill", "killpg"},
}

# Bare method names that only `Path` has, so a call to one is unambiguous.
# `remove` is excluded on purpose -- `Docker.remove` detaches a container, and
# flagging it would train readers to ignore this guard.
PATH_MUTATORS = {"mkdir", "unlink", "symlink_to", "write_text", "write_bytes", "touch", "rmdir"}


def _modules() -> list[Path]:
    modules = sorted(GATE_PACKAGE.glob("*.py"))
    assert len(modules) > 10, "scanned too few modules to trust this guard"
    return modules


def _violations(module: Path) -> list[str]:
    """Every direct reach for the machine, as `line: what`."""
    tree = ast.parse(module.read_text(encoding="utf-8"))
    found: list[str] = []

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found += [
                f"{node.lineno}: import {alias.name}"
                for alias in node.names
                if alias.name == "subprocess"
            ]
        elif isinstance(node, ast.ImportFrom) and node.module == "subprocess":
            found.append(f"{node.lineno}: from subprocess")
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            owner, member = node.func.value, node.func.attr
            if isinstance(owner, ast.Name) and member in QUALIFIED.get(owner.id, ()):
                found.append(f"{node.lineno}: {owner.id}.{member}()")
            elif member in PATH_MUTATORS:
                found.append(f"{node.lineno}: .{member}()")

    return sorted(found)


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_only_the_primitives_touch_the_machine(module: Path) -> None:
    """Anything else is work the dry run cannot show and the log cannot time."""
    allowed = set(BOUNDARY.direct_machine_access) | set(
        BOUNDARY.modules_bypassing_primitives
    )
    if module.name in allowed:
        pytest.skip("allowed to reach the machine directly, or on the ratchet")

    found = _violations(module)

    assert not found, (
        f"{module.name} reaches past the primitives; compose an action from "
        "capsem.gate.actions or capsem.gate.fileactions so the dry run can "
        "show it and the run log can time it:\n  " + "\n  ".join(found)
    )


def test_the_extraction_ratchet_never_runs_backwards() -> None:
    """A module that no longer bypasses the primitives must leave the list.

    Otherwise the ratchet stops describing outstanding work and starts
    describing policy, which is how a temporary exemption becomes permanent.
    """
    stale = sorted(
        name
        for name in BOUNDARY.modules_bypassing_primitives
        if not _violations(GATE_PACKAGE / name)
    )

    assert not stale, (
        "these no longer reach past the primitives -- strike them from "
        "config/gate.toml's modules_bypassing_primitives so the outstanding "
        f"work stays honest: {stale}"
    )


def test_every_ratchet_entry_still_exists() -> None:
    missing = sorted(
        name
        for name in BOUNDARY.modules_bypassing_primitives
        if not (GATE_PACKAGE / name).is_file()
    )

    assert not missing, f"these modules no longer exist: {missing}"


def test_the_permitted_modules_are_the_ones_that_have_to_be() -> None:
    """Widening this is a design decision, not a convenience.

    Each of these owns one piece of machine state as its entire purpose, which
    is why routing it through an action would be ceremony rather than
    visibility -- there is no gate work here for a dry run to show.

    `fileactions` is the primitives themselves. `proc` is the funnel every
    invocation passes through, which is why the run log has one place to hook.
    `pidfiles` is where a signal is sent, so "which process did the gate kill"
    has a single answer. `locks` owns the lockfile that makes one gate per
    machine true, and it has to place that file before any workspace exists.
    """
    assert set(BOUNDARY.direct_machine_access) == {
        "fileactions.py",
        "proc.py",
        "pidfiles.py",
        "locks.py",
    }


#: Reaching for any of these is scheduling work outside the graph.
SCHEDULERS = {"threading", "multiprocessing", "concurrent", "asyncio"}


def _schedulers(module: Path) -> list[str]:
    tree = ast.parse(module.read_text(encoding="utf-8"))
    found: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found += [
                f"{node.lineno}: import {alias.name}"
                for alias in node.names
                if alias.name.split(".")[0] in SCHEDULERS
            ]
        elif (
            isinstance(node, ast.ImportFrom)
            and node.module
            and node.module.split(".")[0] in SCHEDULERS
        ):
            found.append(f"{node.lineno}: from {node.module}")
    return sorted(found)


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_only_the_plan_schedules_concurrent_work(module: Path) -> None:
    """Parallelism the graph cannot see is parallelism the exclusives cannot
    constrain -- which is exactly what seven bare `&` in one recipe body were.

    `assetlanes` hand-rolled its own pool and is on the ratchet until it is
    expressed as two independent steps that both contend for the daemon.
    """
    allowed = set(BOUNDARY.direct_concurrency) | set(
        BOUNDARY.modules_bypassing_primitives
    )
    if module.name in allowed:
        pytest.skip("the scheduler itself, or on the ratchet")

    found = _schedulers(module)

    assert not found, (
        f"{module.name} schedules its own concurrency; declare the work as "
        "independent steps in a Plan, and name what they contend for:\n  "
        + "\n  ".join(found)
    )


def test_the_plan_is_the_only_scheduler() -> None:
    """Widening this is a design decision. The graph decides what overlaps."""
    assert set(BOUNDARY.direct_concurrency) == {"plan.py"}


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_nothing_kills_a_process_by_name(module: Path) -> None:
    """`_ensure-service` stopped only the pidfile-tracked service and never
    ran `pkill capsem-service`, because the developer running the gate may
    have their own daemon up. That care was deliberate and unenforced."""
    source = module.read_text(encoding="utf-8")

    for weapon in ("pkill", "killall", "pgrep"):
        assert weapon not in source, (
            f"{module.name} reaches for {weapon}, which cannot tell this run's "
            "processes from someone else's; stop what the pidfile names"
        )


def test_a_direct_call_would_be_caught(tmp_path: Path) -> None:
    """Red-first, permanently: the guard must see the shapes it forbids."""
    module = tmp_path / "example.py"
    module.write_text(
        "import subprocess\n"
        "import shutil\n"
        "def go(path, spare):\n"
        "    shutil.rmtree(path)\n"
        "    path.mkdir()\n"
        "    spare.which('just')\n"
        "    spare.remove('container')\n"
    )

    found = _violations(module)

    assert [entry.split(": ", 1)[1] for entry in found] == [
        "import subprocess",
        "shutil.rmtree()",
        ".mkdir()",
    ], "and neither `which` nor a container `remove` is mistaken for one"

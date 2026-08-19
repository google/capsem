"""Work goes through the primitives, so the run log and the dry run see it.

`actions` and `fileactions` exist so that a unit of gate work can describe
itself before it runs and be timed while it runs. Both properties are lost the
moment a module calls `shutil.rmtree` or `subprocess.run` directly: the dry run
cannot mention what it does not know about, and the run log cannot record it.

So only the harness may touch the machine: the primitives themselves, the
funnel every invocation passes through, and the modules that own one piece of
machine state as their entire purpose -- the pidfiles, the lock, the run
directory. Everything built on top of that is gate work, and gate work goes
through actions or it is invisible.

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
    """Anything else is work the dry run cannot show and the log cannot time.

    There is no extraction ratchet any more. Every module that reached past
    the primitives has been moved onto them, and an empty list describing no
    outstanding work is a list that gets read as permission for some -- which
    is the rule this file's sibling states about its own.
    """
    if module.name in set(BOUNDARY.direct_machine_access):
        pytest.skip("the harness itself; the primitives are what it provides")

    found = _violations(module)

    assert not found, (
        f"{module.name} reaches past the primitives; compose an action from "
        "capsem.gate.actions or capsem.gate.fileactions so the dry run can "
        "show it and the run log can time it:\n  " + "\n  ".join(found)
    )


def test_the_permitted_modules_are_the_ones_that_have_to_be() -> None:
    """Widening this is a design decision, not a convenience.

    Each of these owns one piece of machine state as its entire purpose, which
    is why routing it through an action would be ceremony rather than
    visibility -- there is no gate work here for a dry run to show.

    `actions` and `fileactions` are the primitives themselves. `proc` is the funnel every
    invocation passes through, which is why the run log has one place to hook.
    `pidfiles` is where a signal is sent, so "which process did the gate kill"
    has a single answer. `locks` owns the lockfile that makes one gate per
    machine true, and has to place it before any workspace exists. `runlog`
    owns the run directory and cannot write through actions because actions
    report into it; `runhistory` reclaims those directories, and `disk`
    reclaims every other tree the gate creates, and `workspace` owns the
    isolated home the actions run against.

    The observation four are the deliberate widening. They are here for the
    inverse of the usual reason: the others own machine state, these *watch*
    it. `faults` stats and hashes what changed, `faultlog` writes and fsyncs
    the report so a killed run still leaves one, `observation` judges each
    change as it lands, and `interception` is the primitives proxied -- its
    entire purpose is that nothing reaches `os` without passing through it,
    which routing through an action would defeat rather than express.

    They were added after a release run died reading a file that was `0644`
    before and `0644` after, because nothing in the gate was in the path of
    the call that changed it.

    `prefix` is the newest, and it is here for a third reason again: it is not
    that it owns machine state or watches it, but that it runs *before the
    run*. The private copy of the checkout has to exist before the process
    that works in it, so it is built from `reexec()` -- above every resource,
    outside the machine lock, with no journal yet to record into. An action
    would have to be a plan step that creates the directory the plan is
    already executing from.

    The through-line is that these are the harness, and the harness is what
    gate work is expressed *in*. A capability or a command appearing here
    would mean work that the dry run cannot show and the log cannot time.
    """
    assert set(BOUNDARY.direct_machine_access) == {
        "actions.py",
        "fileactions.py",
        "filesystem.py",
        "proc.py",
        "processgroup.py",
        "pidfiles.py",
        "locks.py",
        "runlog.py",
        # The same run directory, written after the run it describes.
        "summary.py",
        "runhistory.py",
        "disk.py",
        "workspace.py",
        "faults.py",
        "faultlog.py",
        "interception.py",
        "observation.py",
        # And the one that runs before there is a run. `prefix` builds the tree
        # the gate executes *from*, consulted by `reexec()` above every
        # resource and outside the machine lock -- so there is no journal for
        # an action to report into, and a plan expressing it would have to
        # create the directory it is already running in.
        "prefix.py",
        "snapshot.py",
        # The third pre-run half: `buildcache` lends the machine's build output
        # to the prefix about to run and takes it back on the way out, both
        # where `prefix` works -- before the journal, outside the lock.
        "buildcache.py",
        # And the shared build directory those two work against, split out of
        # `prefix` at the module ceiling and in the same pre-run position: it
        # links the profile directories the child compiles into and bounds
        # their size, both from `reexec()`.
        "cargotarget.py",
        "sourcecommit.py",
        "commitsnapshot.py",
        "prefixlease.py",
        # And the one that writes the rules the run is refused by. Same reason
        # as `prefix`: the profile has to exist before the process that
        # executes under it, so it is rendered from `reexec()` -- above every
        # resource, outside the machine lock, with no journal to record into.
        # An action expressing it would have to be a plan step that creates
        # the sandbox the plan is already running inside.
        "sandbox.py",
        # And the one that reads back what that profile permitted. A different
        # reason from the two above: this one is inside the run, but it holds a
        # `log stream` open for the whole of it. Actions are commands that
        # finish and are journalled once; a process that must outlive the call
        # that starts it and be killed on the way out is a `Resource`, and a
        # resource needs `Popen` rather than the runner's exec accounting.
        "sandboxreport.py",
        # The pre-sandbox capability resource and its deliberately planless
        # process half. The owning plan still journals every brokered command
        # through GuardedRunner; these two only own the irreversible boundary.
        "egress.py",
        "egressbroker.py",
        # And the one place a hardlink into published output may be made. The
        # choice between linking and copying is a filesystem question, so it
        # cannot sit behind the action layer that exists to record such calls.
        "auditfs.py",
    }


# Whole worlds of their own scheduling; importing one at all is the decision.
FORBIDDEN_RUNTIMES = {"multiprocessing", "asyncio"}

# Things that *create* concurrent execution, as opposed to constraining it.
# `threading.Lock` is deliberately absent: a mutex serializes access to
# something and starts nothing, which is the opposite of what this guards.
SPAWNERS = {
    "Thread",
    "Timer",
    "ThreadPoolExecutor",
    "ProcessPoolExecutor",
    "Process",
    "Pool",
}


def _schedulers(module: Path) -> list[str]:
    """Every place a module starts concurrent work of its own."""
    tree = ast.parse(module.read_text(encoding="utf-8"))
    found: list[str] = []

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found += [
                f"{node.lineno}: import {alias.name}"
                for alias in node.names
                if alias.name.split(".")[0] in FORBIDDEN_RUNTIMES
            ]
        elif (
            isinstance(node, ast.ImportFrom)
            and node.module
            and node.module.split(".")[0] in FORBIDDEN_RUNTIMES
        ):
            found.append(f"{node.lineno}: from {node.module}")
        elif isinstance(node, ast.Name) and node.id in SPAWNERS:
            found.append(f"{node.lineno}: {node.id}")
        elif isinstance(node, ast.Attribute) and node.attr in SPAWNERS:
            found.append(f"{node.lineno}: .{node.attr}")

    return sorted(set(found))


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_only_the_plan_schedules_concurrent_work(module: Path) -> None:
    """Parallelism the graph cannot see is parallelism the exclusives cannot
    constrain -- which is exactly what seven bare `&` in one recipe body were.

    `assetlanes` hand-rolled its own pool, because the plan could express only
    "one holder" and its two lanes must overlap. A claim carries a mode now,
    so they are two steps holding the daemon shared -- and this guard has no
    exceptions left beyond the scheduler itself.
    """
    if module.name in set(BOUNDARY.direct_concurrency):
        pytest.skip("the scheduler itself")

    found = _schedulers(module)

    assert not found, (
        f"{module.name} schedules its own concurrency; declare the work as "
        "independent steps in a Plan, and name what they contend for:\n  " + "\n  ".join(found)
    )


def test_the_plan_is_the_only_scheduler() -> None:
    """Widening this is a design decision. The graph decides what overlaps.

    `planrunner`, not `plan`: the scheduler was split out when `plan` outgrew
    the module ceiling. One module still, and still the only one -- the graph
    decides what may overlap, and executing that decision is its own job.
    """
    assert set(BOUNDARY.direct_concurrency) == {"planrunner.py"}


def test_a_mutex_is_not_mistaken_for_a_scheduler(tmp_path: Path) -> None:
    """Serializing access to a file starts nothing.

    `runlog` is appended to by steps the plan is running concurrently, so it
    needs a lock; forbidding that would push it towards either corrupting its
    own output or inventing a worse way to avoid it.
    """
    module = tmp_path / "example.py"
    module.write_text(
        "import threading\n"
        "from concurrent.futures import ThreadPoolExecutor\n"
        "guard = threading.Lock()\n"
        "pool = ThreadPoolExecutor()\n"
    )

    found = [entry.split(": ", 1)[1] for entry in _schedulers(module)]

    assert found == ["ThreadPoolExecutor"], "the pool, not the lock"


#: Ways to name a process that are not "the pid in this run's pidfile".
BY_NAME = ("pkill", "killall", "pgrep")


@pytest.mark.parametrize("module", _modules(), ids=lambda p: p.name)
def test_nothing_kills_a_process_by_name(module: Path) -> None:
    """`_ensure-service` stopped only the pidfile-tracked service and never
    ran `pkill capsem-service`, because the developer running the gate may
    have their own daemon up. That care was deliberate and unenforced.

    Checked against string *values* rather than the file's text, so a
    docstring may say `pkill` while explaining why nothing uses it.
    """
    tree = ast.parse(module.read_text(encoding="utf-8"))
    docstrings = {
        ast.get_docstring(node, clean=False)
        for node in ast.walk(tree)
        if isinstance(node, ast.Module | ast.ClassDef | ast.FunctionDef)
    }

    offenders = [
        f"{node.lineno}: {node.value!r}"
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and node.value not in docstrings
        and any(weapon in node.value for weapon in BY_NAME)
    ]

    assert not offenders, (
        f"{module.name} reaches for a process by name, which cannot tell this "
        "run's processes from someone else's; stop what the pidfile names:\n  "
        + "\n  ".join(offenders)
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

"""Containers get their source copied in, not the developer's checkout mounted.

Every gate container does `-v <repo_root>:/src`, which is two defects wearing
one flag.

It is an isolation hole: the container can read and often write the live
checkout, so "the gate runs in a container" says nothing about what the
container can reach.

And it is a *race*. `rust-coverage` runs on the host and churns hardlinks
inside that tree while `linux-rust` reads the same inodes through virtiofs.
A release run died on it -- `Permission denied` opening
`config/profiles/code/root/root/.gemini/projects.json`, a file that was `0644`
before and `0644` after -- and no amount of declaring `contends` would have
prevented it, because the two steps genuinely share nothing except the
filesystem nobody wrote down.

Copying the source into an image removes the class rather than guarding it.
There is no mount to police, no host path to rewrite, and nothing for two
steps to contend over: each container holds its own bytes.

These are argv-level assertions on purpose. The mount is a flag, the network
mode is a flag, and a flag is what a future change will quietly reintroduce.
"""

from __future__ import annotations

import ast
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE = PROJECT_ROOT / "src" / "capsem" / "gate"

#: The one module allowed to spell `docker` as a command name. Everything else
#: asks it, so that mounts and network mode have a single place to be decided.
WRAPPER = {"docker.py", "dockerimage.py"}


def _modules() -> list[Path]:
    return sorted(path for path in GATE.glob("*.py") if path.name not in WRAPPER)


def _docker_argv_literals(tree: ast.AST) -> list[ast.List]:
    """Every list literal whose first element is the string `docker`."""
    found: list[ast.List] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.List) or not node.elts:
            continue
        first = node.elts[0]
        if isinstance(first, ast.Constant) and first.value == "docker":
            found.append(node)
    return found


#: Modules that still build docker argv by hand. A ratchet rather than an
#: `xfail`: an `xfail` says "this is broken" and hides how broken, whereas
#: this refuses a tenth site while the nine are migrated. Each removal here is
#: a module that can no longer choose its own mount or network mode.
#:
#: These are exact counts, not ceilings -- see
#: `test_the_ratchet_carries_no_slack`. Started at nine; `hostimage.py` sat at
#: 5 against an actual 2 for long enough that the guard would have permitted
#: three new hand-built sites in the module Phase 5 was about to touch.
UNMIGRATED = {
    "hostimage.py": 2,
    "installimage.py": 2,
}


def test_no_new_module_builds_docker_argv_by_hand() -> None:
    """One place decides the flags, or every call site decides them again.

    The mount, the network mode and the removal policy have to be identical
    everywhere; spread across four modules they were made differently in nine
    places -- including the `-v <repo_root>:/src` that raced a host step and
    killed a release run.
    """
    counted: dict[str, int] = {}
    for path in _modules():
        tree = ast.parse(path.read_text(encoding="utf-8"))
        found = _docker_argv_literals(tree)
        if found:
            counted[path.name] = len(found)

    new = {name: count for name, count in counted.items() if name not in UNMIGRATED}
    assert not new, f"new modules building docker argv directly: {new}"

    grown = {
        name: (count, UNMIGRATED[name])
        for name, count in counted.items()
        if count > UNMIGRATED[name]
    }
    assert not grown, f"these grew new hand-built docker argv (now, allowed): {grown}"

    # And the debt only shrinks: a migrated module leaves the list.
    assert set(counted) <= set(UNMIGRATED), sorted(set(counted) - set(UNMIGRATED))


def test_the_ratchet_carries_no_slack() -> None:
    """`UNMIGRATED` records what is there, not what would be tolerated.

    Without this, paying the debt down and leaving the number alone is
    invisible, and the gap silently becomes an allowance for new sites. That
    is not hypothetical: the table said `hostimage.py: 5` against an actual 2,
    so three hand-built `docker` argv could have been added to the module Phase
    5 is about to rewrite, and every guard here would have stayed green.

    The cost is one line per migration -- lower the count in the same commit --
    and the benefit is that the ratchet cannot quietly stop being one.
    """
    counted: dict[str, int] = {}
    for path in _modules():
        found = _docker_argv_literals(ast.parse(path.read_text(encoding="utf-8")))
        if found:
            counted[path.name] = len(found)

    slack = {
        name: (allowed, counted.get(name, 0))
        for name, allowed in UNMIGRATED.items()
        if allowed > counted.get(name, 0)
    }
    assert not slack, (
        "these carry more allowance than debt (allowed, actual) -- lower them "
        f"to the actual count, or drop the entry entirely at zero: {slack}"
    )


def test_a_mount_cannot_point_at_the_checkout() -> None:
    """The specific hole: `-v <repo_root>:/src`.

    Refused at construction rather than reviewed, because this is the flag
    that made a host step and a container step share inodes.
    """
    import pytest

    from capsem.gate import config as gate_config
    from capsem.gate.dockermount import Mount
    from capsem.gate.errors import GateError

    root = gate_config.load(PROJECT_ROOT).root

    with pytest.raises(GateError, match="checkout"):
        Mount(source=str(root), target="/src")

    with pytest.raises(GateError, match="checkout"):
        Mount(source=str(root / "config"), target="/src/config")

    # A named volume and a path outside the checkout stay legal: this refuses
    # the checkout, not mounting.
    assert Mount(source="capsem-cargo-registry", target="/usr/local/cargo/registry")
    assert Mount(source="/tmp/capsem-scratch", target="/scratch")


def test_every_container_declares_its_network() -> None:
    """No default, so `--network` is a decision rather than an omission.

    Nothing in the gate passes `--network` today, which means every container
    has outbound access and several use it mid-run. A required keyword makes
    that visible at each call site instead of invisible everywhere.
    """
    import inspect

    from capsem.gate.docker import Docker

    for name in ("run_detached", "run_once"):
        method = getattr(Docker, name, None)
        assert method is not None, f"Docker.{name} is missing"
        parameter = inspect.signature(method).parameters.get("network")
        assert parameter is not None, f"Docker.{name} does not take a network mode"
        assert parameter.default is inspect.Parameter.empty, (
            f"Docker.{name} defaults its network mode, so a call site can omit "
            "the decision and get outbound access without saying so"
        )


def test_the_checkout_mounts_are_enumerated_and_shrinking() -> None:
    """Four mounts of the working tree remain, and all four say so.

    `Mount.unmigrated` is deliberately ugly and deliberately greppable. The
    alternative was switching the guard off globally while the modules are
    converted, which is how a temporary exemption becomes the behaviour.

    `gitmetadata.py` and `packagerail.py` joined this list without any new
    mount being created -- both were bare `-v` argv where no guard could see
    them. The count went up while the truth stayed the same, which is the one
    reason a ratchet may move this way, and it belongs here rather than in a
    commit message nobody reads.

    `packagerail.py` is the one with a way out. It mounts the checkout writable
    because the builder runs `pnpm install && pnpm build` inside
    `/src/frontend`; baking the frontend into the builder image is what lets
    that mount go, exactly as it did for the parity lane. A
    linked worktree's common directory lives under the primary checkout, and
    that mount was assembled as bare `-v` argv where no guard could see it. It
    is declared now, so the count went up while the truth stayed the same --
    which is the direction a ratchet is allowed to move only for this reason,
    and the reason belongs here rather than in a commit message nobody reads.
    """
    remaining: dict[str, int] = {}
    for path in _modules():
        count = path.read_text(encoding="utf-8").count("Mount.unmigrated(")
        if count:
            remaining[path.name] = count

    assert remaining == {
        "debproof.py": 1,
        "gitmetadata.py": 1,
        "installcontainer.py": 1,
        "packagerail.py": 1,
    }, (
        f"the checkout-mount debt changed: {remaining}. It may shrink -- update "
        "this expectation when a module moves to COPY -- but a new one is the "
        "race that killed a release run coming back."
    )

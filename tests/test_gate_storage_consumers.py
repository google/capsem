"""No resource is reclaimed before the step the policy says last needs it.

`config/storage-policy.toml` records a `last_consumer` for every working
resource. Nothing read it. It was documentation sitting beside the machinery it
described, and the two disagreed:

    [resources.capsem-host-builder]
    docker_name = "capsem-host-builder:latest"
    last_consumer = "package-x86_64"
    release_boundary = "after-packages"
    release_boundaries = ["after-linux-rust-builder", "after-packages"]
    reason = "Final tag is needed by both package builds; ..."

The extra boundary released the image before either package build, and
`package.arm64` died with docker exit 125 after 37 minutes of gate. The shell
survived it because the cross-compile lane rebuilt the image first; composed
into one plan, `hostimage.fragment` is `plan.shared`, so it runs once and
whatever destroys its output afterwards is simply destruction.

A shared step is built once. That makes "released after its last consumer" a
property the graph has to hold, and this is where it is held.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _policy() -> dict:
    return tomllib.loads(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )


def _boundaries(resource: dict) -> set[str]:
    found = {str(value) for value in resource.get("release_boundaries", [])}
    if resource.get("release_boundary"):
        found.add(str(resource["release_boundary"]))
    return found


def _schedule() -> dict[str, int]:
    """Where each label runs in the complete gate, by graph order."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return {label: index for index, label in enumerate(gate_labels("candidate"))}


def _release_positions(schedule: dict[str, int]) -> dict[str, list[int]]:
    """Every point in the plan at which each boundary is released."""
    from capsem_builder.gate import config as gate_config

    phases = gate_config.load(PROJECT_ROOT).storage.phases
    positions: dict[str, list[int]] = {}
    for label, index in schedule.items():
        if ".storage." not in label:
            continue
        phase = phases.get(label.rsplit(".storage.", 1)[1])
        if phase is not None:
            positions.setdefault(phase.boundary, []).append(index)
    return positions


def test_nothing_is_released_before_its_declared_last_consumer() -> None:
    schedule = _schedule()
    positions = _release_positions(schedule)

    offences: list[str] = []
    for name, resource in _policy()["resources"].items():
        consumer = resource.get("last_consumer")
        if not consumer:
            continue
        # `package-x86_64` is the step `package.x86_64`; the policy names
        # events, the plan names steps.
        consumed_at = schedule.get(consumer.replace("-", ".", 1))
        if consumed_at is None:
            continue  # not a step this host's plan contains
        for boundary in _boundaries(resource):
            for released_at in positions.get(boundary, []):
                if released_at < consumed_at:
                    offences.append(
                        f"{name} is released at {boundary!r} before its last "
                        f"consumer {consumer!r} runs"
                    )

    assert not offences, "; ".join(sorted(set(offences)))


def test_every_release_boundary_reclaims_something() -> None:
    """A boundary with no resources is a step that cannot do anything.

    Kept because removing `capsem-host-builder` from `after-linux-rust-builder`
    is only correct if that leaves the boundary genuinely empty -- in which
    case the phase and its step go too, rather than staying as ceremony.
    """
    from capsem_builder.gate import config as gate_config

    claimed = {
        phase.boundary for phase in gate_config.load(PROJECT_ROOT).storage.phases.values()
    }
    served = set()
    for resource in _policy()["resources"].values():
        served |= _boundaries(resource)

    assert not (claimed - served), (
        "these storage phases release nothing: "
        + ", ".join(sorted(claimed - served))
    )

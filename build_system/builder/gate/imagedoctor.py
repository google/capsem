"""Host diagnosis shared by the profile-owned image rails."""

from __future__ import annotations

from .actions import Call, Run
from .config import GateConfig
from .execution import Kind, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects


def doctor(config: GateConfig, *, skips: dict[str, str] | None = None) -> Step:
    """Check host wiring while omitting assets and KVM that are not built yet."""
    from . import doctor as diagnosis

    return step(
        "doctor",
        Call(
            "would the gate work if we started now",
            diagnosis.report,
            justification=CallJustification(
                kind=OpaqueKind.PURE_INSPECTION,
                reason="reports every wiring problem it can find and changes nothing at all",
                effects=machine_effects(Effect.PROCESS),
            ),
        ),
        Run(
            ["bash", config.doctor.common_script],
            env=dict(config.imagebuild.doctor_skips if skips is None else skips),
        ),
        kind=Kind.STATIC_TEST,
        speed=Speed.FAST,
    )

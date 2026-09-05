"""Pure impact-aware complete-test admission decisions."""

from __future__ import annotations

from .models import AdmissionDecision, FocusGroup, TestAdmissionPolicy


def _groups(policy: TestAdmissionPolicy, paths: tuple[str, ...]) -> tuple[FocusGroup, ...] | None:
    selected: set[FocusGroup] = set()
    for path in paths:
        matches = [route for route in policy.routes if path.startswith(route.prefix)]
        if not matches:
            return None
        longest = max(len(route.prefix) for route in matches)
        for route in matches:
            if len(route.prefix) == longest:
                selected.update(route.groups)
    return tuple(sorted(selected, key=str))


def decide_admission(
    *,
    policy: TestAdmissionPolicy,
    baseline: str | None,
    target: str,
    changed_paths: tuple[str, ...],
    commits_since_success: int,
    forced: bool,
    force_reason: str,
    prior_forced: bool,
    failed_attempt: bool = False,
) -> AdmissionDecision:
    """Admit full proof only when its source impact or cadence justifies it."""
    routed = _groups(policy, changed_paths) if changed_paths else None
    high_impact = routed is None
    if forced and not force_reason.strip():
        allowed, explanation = False, "forced full test requires a non-empty reason"
    elif forced and prior_forced and not failed_attempt:
        allowed, explanation = False, "consecutive forced full-test attempts are refused"
    elif forced:
        allowed, explanation = True, f"forced by operator: {force_reason.strip()}"
    elif failed_attempt:
        allowed, explanation = False, "the last full attempt failed or was interrupted; use focused checks or an explicitly approved retry"
    elif baseline is None:
        allowed, explanation = True, "no complete baseline exists; initial local proof is optional"
    elif high_impact:
        allowed, explanation = True, "changed paths are high-impact or unknown"
    elif commits_since_success >= policy.minimum_commits:
        allowed = True
        explanation = f"low-impact cadence reached {commits_since_success} commits"
    else:
        allowed = False
        explanation = (
            f"low-impact source has {commits_since_success} of "
            f"{policy.minimum_commits} commits since complete proof"
        )
    return AdmissionDecision(
        allowed=allowed,
        forced=forced and allowed,
        high_impact=high_impact,
        baseline=baseline,
        target=target,
        commits_since_success=commits_since_success,
        changed_paths=tuple(sorted(changed_paths)),
        groups=() if routed is None else routed,
        explanation=explanation,
    )

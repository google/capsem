"""Citadel guard for immutable release-transition baselines."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE_RATIONALE = (
    "A release candidate must be compared with the latest verified stable graph. "
    "Selecting the target channel makes nightly depend on arbitrary previous-nightly "
    "health and turns stable qualification into a mutable cross-channel dependency."
)


def _selector(workflow: str) -> str:
    start = workflow.index("- name: Select exact public-before manifest")
    end = workflow.index("\n      - name:", start + 8)
    return workflow[start:end]


def _violations(binary: str, profile: str, staging: str) -> list[str]:
    violations = []
    for name, workflow in (("binary", binary), ("profile", profile)):
        selector = _selector(workflow)
        if '--channel "stable"' not in selector:
            violations.append(f"{name} public-before is not verified stable")
    if "CAPSEM_RELEASE_BASELINE_CHANNEL=" not in binary:
        violations.append("binary pairing drops the verified baseline identity")
    if "CAPSEM_RELEASE_BASELINE_CHANNEL=" not in staging:
        violations.append("profile pairing drops the verified baseline identity")
    if "CAPSEM_RELEASE_TRANSITION=auto" not in staging:
        violations.append("profile pairing prevents cross-channel classification")
    return violations


def _sources() -> tuple[str, str, str]:
    return (
        (ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8"),
        (ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8"),
        (ROOT / "scripts/stage-profile-pairing.sh").read_text(encoding="utf-8"),
    )


def test_release_transitions_use_one_verified_stable_baseline() -> None:
    violations = _violations(*_sources())
    assert not violations, BASELINE_RATIONALE + "\n  " + "\n  ".join(violations)


def test_guard_rejects_previous_nightly_as_the_baseline() -> None:
    binary, profile, staging = _sources()
    binary = binary.replace('--channel "stable"', '--channel "$RELEASE_CHANNEL"', 1)
    profile = profile.replace('--channel "stable"', '--channel "${{ inputs.channel }}"', 1)

    violations = _violations(binary, profile, staging)
    assert len(violations) == 2, BASELINE_RATIONALE

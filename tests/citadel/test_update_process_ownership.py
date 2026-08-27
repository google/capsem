"""Citadel guard: a systemd service must not own its package updater.

The release qualification that found this reached the exact candidate, launched
``capsem update --yes``, and then stopped ``capsem.service`` from the Debian
preinstall hook. Because the updater, apt, and dpkg were children of that unit,
systemd killed all of them in the middle of package replacement.

The Rust regression proves command construction. This source guard records why
the ownership boundary exists and fails in the fast phase if it is weakened.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SERVICE = PROJECT_ROOT / "crates/capsem-service/src/main.rs"

SYSTEMD_UPDATE_OWNERSHIP_RATIONALE = """\
Linux package replacement stops capsem.service from the Debian preinstall hook.
An updater launched as a child of that service stays in its cgroup, so stopping
the service kills capsem, apt, dpkg, and the preinstall script halfway through
the transaction.

The complete `capsem update --yes` transaction must run in a fixed, sibling
transient user service. Wrapping only apt leaves the parent unable to activate
profiles or record the audit result. `--pipe` is forbidden because the reader
dies with capsem.service and can SIGPIPE the surviving updater. The fixed unit
name also prevents the restarted service from launching a duplicate update.
"""


def _function(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 0
    for index in range(opening, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    raise AssertionError(f"unterminated function: {signature}")


def _ownership_violations(body: str) -> list[str]:
    required = (
        'std::env::var_os("INVOCATION_ID")',
        'program: "systemd-run".to_string()',
        '"--user".to_string()',
        '"--wait".to_string()',
        '"--collect".to_string()',
        '"--unit=capsem-update".to_string()',
        '"--".to_string()',
        "transient_args.extend(args)",
    )
    violations = [f"missing `{needle}`" for needle in required if needle not in body]
    if '"--pipe"' in body:
        violations.append("uses `--pipe`, tying the updater back to capsem.service")
    if (
        "transient_args.extend(args)" in body
        and "program," in body
        and body.index("program,") > body.index("transient_args.extend(args)")
    ):
        violations.append("places the capsem program after its update arguments")
    return violations


def test_systemd_update_owns_the_complete_transaction_outside_capsem_service() -> None:
    body = _function(
        SERVICE.read_text(),
        "fn update_command_plan(kind: UpdateCommandKind)",
    )
    violations = _ownership_violations(body)
    assert not violations, SYSTEMD_UPDATE_OWNERSHIP_RATIONALE + "\n" + "\n".join(violations)


def test_guard_rejects_the_failure_shapes_that_reached_release_qualification() -> None:
    good = _function(
        SERVICE.read_text(),
        "fn update_command_plan(kind: UpdateCommandKind)",
    )
    bad_shapes = {
        "direct service child": good.replace(
            'program: "systemd-run".to_string()',
            "program: program.clone()",
        ),
        "pipe back to stopped service": good.replace(
            '"--wait".to_string()',
            '"--wait".to_string(), "--pipe".to_string()',
        ),
        "anonymous unit permits duplicate": good.replace(
            '"--unit=capsem-update".to_string()',
            '"--property=Type=exec".to_string()',
        ),
        "apt-only wrapper": good.replace(
            "transient_args.extend(args)",
            'transient_args.extend(["apt-get".to_string()])',
        ),
    }
    undetected = [name for name, source in bad_shapes.items() if not _ownership_violations(source)]
    assert not undetected, f"ownership guard accepts known failure shapes: {undetected}"

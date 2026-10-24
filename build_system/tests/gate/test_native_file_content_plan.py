"""Guest FileContent uses an explicit test artifact before the existing VM owner."""

from helpers.gate import gate_plan


def ancestors(plan, label):
    found = set()
    pending = [label]
    while pending:
        for before in plan.after_of(pending.pop()):
            if before not in found:
                found.add(before)
                pending.append(before)
    return found


def test_native_artifact_is_prepared_before_existing_functional_broad_owner():
    plan = gate_plan("test-functional")
    assert "functional.native-file-content.assemble" in ancestors(plan, "functional.pytest.broad")
    assert "functional.native-file-content.compile" in ancestors(
        plan, "functional.native-file-content.assemble"
    )
    assert "functional.native-file-content.source" in ancestors(
        plan, "functional.native-file-content.compile"
    )

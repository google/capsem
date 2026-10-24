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


def test_direct_assembly_uses_existing_machine_owner(monkeypatch):
    import os
    import sys
    from contextlib import contextmanager

    from capsem_builder.gate.config import load
    from capsem_builder.gate.tools import native_file_content as owner
    from helpers.gate import PROJECT_ROOT

    config = load(PROJECT_ROOT)
    entered = []

    @contextmanager
    def leased(command, root, inherited, *, force):
        assert force and root == config.root and inherited is os.environ
        entered.append("lease")
        yield {}
        entered.append("released")

    def assemble(*args):
        assert entered == ["lease"], "admission/copy must run inside the canonical machine owner"

    monkeypatch.setattr(owner, "leased", leased, raising=False)
    monkeypatch.setattr(owner, "assemble", assemble)
    monkeypatch.setattr(
        sys, "argv", ["native", "assemble", "--bin-dir", config.modules.default_bin_dir]
    )
    assert owner.main(PROJECT_ROOT) == 0
    assert entered == ["lease", "released"]

"""Existing Docker/Tart ownership, never an inner installer on the host."""

import ast
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.installproof import InstallProof
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner

from build_system.tests.gate.test_gate_install_ordering import gate as gate

ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(ROOT)
SHA = SourceCommit("a" * 40)


def pairing(tmp_path):
    from capsem_builder.gate.installpairing import NativePairing

    return NativePairing(
        channel="nightly",
        before_manifest=tmp_path / "before.json",
        after_manifest=tmp_path / "after.json",
        before_package=tmp_path / "before.deb",
        before_inputs=tmp_path / "before-inputs",
        after_inputs=tmp_path / "after-inputs",
    )


def test_pairing_reuses_actual_docker_exec_and_exact_installed_owner(tmp_path):
    selected = pairing(tmp_path)
    runner = RecordingRunner(ROOT)
    InstallProof(runner, CONFIG, source_commit=SHA, pairing=selected).prove_glowup(
        "/src/after.deb", boots_a_guest=True
    )
    assert runner.ran(r"docker exec .*local-release-glowup\.py")
    assert runner.ran(r"--release-transition binary_only")
    assert runner.ran(r"--source-commit " + SHA)
    assert runner.ran(r"--before-manifest " + str(selected.before_manifest))
    assert not runner.ran(r"--skip-install")


def test_no_guest_pairing_refuses_instead_of_running_a_packaging_substitute(tmp_path):
    runner = RecordingRunner(ROOT)
    with pytest.raises(ValueError, match="guest"):
        InstallProof(runner, CONFIG, source_commit=SHA, pairing=pairing(tmp_path)).prove_glowup(
            "/src/after.deb", boots_a_guest=False
        )
    assert not runner.ran(r"docker")


def test_existing_outer_owner_still_stops_its_container_when_handoff_cleanup_fails(
    gate, monkeypatch
):
    owner, runner = gate

    def fail():
        raise GateError("synthetic handoff cleanup failure")

    monkeypatch.setattr(owner._graph, "clear_handoff", fail)
    with pytest.raises(GateError, match="handoff cleanup failure"):
        owner.run()
    assert runner.last_index_of(r"docker rm -f -v capsem-install-test") > runner.index_of(
        r"dpkg -i"
    )


def test_actual_native_owner_invocation_is_inside_existing_sigterm_unwinding():
    from benchmarks.helpers import install_owner

    tree = ast.parse(Path(install_owner.__file__).read_text())
    main = next(
        node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "main"
    )
    scopes = [
        node
        for node in ast.walk(main)
        if isinstance(node, ast.With)
        and any(
            isinstance(item.context_expr, ast.Call)
            and isinstance(item.context_expr.func, ast.Name)
            and item.context_expr.func.id == "unwind_sigterm"
            for item in node.items
        )
    ]
    assert len(scopes) == 1
    assert any(
        isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "invoke"
        for node in ast.walk(scopes[0])
    )


@pytest.mark.parametrize(
    "field",
    ["before_manifest", "after_manifest", "before_package", "before_inputs", "after_inputs"],
)
def test_pairing_never_mounts_host_root_or_relative_authority(tmp_path, field):
    from dataclasses import replace

    selected = pairing(tmp_path)
    for invalid in (Path("/"), Path("relative"), tmp_path / ".." / "foreign"):
        with pytest.raises(ValueError, match="absolute"):
            replace(selected, **{field: invalid})


def test_existing_output_refuses_before_any_native_fixture_admission(tmp_path, monkeypatch):
    from benchmark_install_fixture import load, prepared

    from benchmarks.helpers.install_owner import invoke

    path, declaration = prepared(tmp_path, monkeypatch)
    selected = load(path, declaration)
    work = tmp_path / "native"
    work.mkdir()
    (work / "report.json").write_text('{"stale":true}')
    with pytest.raises(ValueError, match="fresh"):
        invoke(ROOT, selected, work, RecordingRunner(ROOT))

"""Fresh Criterion evidence contracts; process doubles never run benchmarks."""

import json
import runpy
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture
def collector(monkeypatch):
    settings = gate_config.load(ROOT).benchmark.run
    result = runpy.run_path(str(ROOT / settings.collectors / "criterion"))["main"].__globals__
    monkeypatch.setitem(
        result, "prepare", lambda *a, **k: pytest.fail("unmocked Criterion preparation")
    )
    return result


def write_case(root, case="group/case", *, full_id=None, sample=None, directory=None):
    path = root / case / "new"
    path.mkdir(parents=True, exist_ok=True)
    metadata = json.dumps({"full_id": full_id or case, "directory_name": directory or case})
    original = json.dumps(
        sample
        if sample is not None
        else {"sampling_mode": "Linear", "times": [100, 300], "iters": [10, 20]}
    )
    (path / "benchmark.json").write_text(metadata)
    (path / "sample.json").write_text(original)
    return metadata, original


def process_double(collector, monkeypatch, listing, emit, *, fail=False):
    outputs = []

    class Execution:
        def __init__(self, budget):
            self.budget = budget
            self.receipt = {"source_double": True}
            self.runtime_metrics = {
                "observed_cpu_lower_bound_s": {"unit": "seconds", "samples": [0.5]}
            }

        def run(self, arguments, output, phase):
            outputs.append(output)
            if phase == "declare":
                assert arguments == ["--list"]
                if len(listing.encode()) > self.budget["bytes"]:
                    raise ValueError("budget")
                self.budget["bytes"] -= len(listing.encode())
                return listing
            if fail:
                raise subprocess.CalledProcessError(1, arguments)
            emit(output)
            return ""

    monkeypatch.setitem(collector, "prepare", lambda *a, **k: Execution(k["budget"]))
    return outputs


@pytest.mark.parametrize(
    "case", ["security_registry_builtin_clone", "group/case", "security_actions/case"]
)
def test_target_output_includes_every_exact_listed_case(collector, tmp_path, monkeypatch, case):
    retained = tmp_path / "retained/security_actions/old/new/sample.json"
    retained.parent.mkdir(parents=True)
    retained.write_text('{"times":[999],"iters":[1]}')
    originals = []
    outputs = process_double(
        collector,
        monkeypatch,
        case + ": benchmark\n",
        lambda output: originals.append(write_case(output, case)),
    )
    values, receipt, resources = collector["run_bench"](
        "capsem-core", "security_actions", sample_size=None
    )
    assert values == {case.removeprefix("security_actions/"): [10.0, 15.0]}
    assert receipt["cases"][case] == dict(
        zip(("benchmark_json", "sample_json"), originals[0], strict=True)
    )
    assert receipt["listing"] == case + ": benchmark\n"
    assert receipt["target"] == "security_actions" and receipt["crate"] == "capsem-core"
    assert resources["observed_cpu_lower_bound_s"] == {"unit": "seconds", "samples": [0.5]}
    assert outputs[0] != outputs[1] and outputs[0].parent == outputs[1].parent
    assert all(not path.parent.exists() for path in outputs)
    assert retained.read_text() == '{"times":[999],"iters":[1]}'


@pytest.mark.parametrize("listing", ["", "not a declaration\n", "a: benchmark\na: benchmark\n"])
def test_listing_must_be_nonempty_exact_and_unique(collector, monkeypatch, listing):
    outputs = process_double(
        collector, monkeypatch, listing, lambda root: pytest.fail("must not measure")
    )
    with pytest.raises(ValueError):
        collector["run_bench"]("capsem-core", "security_actions", sample_size=None)
    assert len(outputs) == 1 and not outputs[0].parent.exists()


@pytest.mark.parametrize("case", [None, "unlisted", "a"])
def test_one_case_cannot_hide_missing_or_unknown_cases(collector, monkeypatch, case):
    def emit(root):
        if case:
            write_case(root, case)

    outputs = process_double(collector, monkeypatch, "a: benchmark\nb: benchmark\n", emit)
    with pytest.raises(ValueError, match=r"coverage|identity"):
        collector["run_bench"]("capsem-core", "security_actions", sample_size=None)
    assert all(not path.parent.exists() for path in outputs)


def test_successful_child_without_output_cannot_reuse_retained_samples(
    collector, tmp_path, monkeypatch
):
    retained = tmp_path / "old"
    write_case(retained)
    outputs = process_double(collector, monkeypatch, "group/case: benchmark\n", lambda root: None)
    with pytest.raises(ValueError, match="coverage"):
        collector["run_bench"]("capsem-core", "security_actions", sample_size=None)
    assert (retained / "group/case/new/sample.json").is_file()
    assert all(not path.parent.exists() for path in outputs)


@pytest.mark.parametrize(
    "sample",
    [
        {"times": [1, 2], "iters": [1]},
        {"times": [], "iters": []},
        {"times": [1], "iters": [0]},
        {"times": [1], "iters": [-1]},
        {"times": [0], "iters": [1]},
        {"times": [-1], "iters": [1]},
        {"times": [float("nan")], "iters": [1]},
        {"times": [1], "iters": [float("inf")]},
        {"times": [float("inf")], "iters": [1]},
        {"times": [True], "iters": [1]},
        {"times": [1], "iters": ["1"]},
        {"times": [1], "iters": [1.5]},
        {"times": "123", "iters": [1]},
        {"iters": [1]},
        {"times": [5e-324], "iters": [1e308]},
    ],
)
def test_raw_arrays_cannot_drop_bad_or_unpaired_samples(collector, tmp_path, sample):
    write_case(tmp_path, sample=sample)
    with pytest.raises((ValueError, KeyError)):
        collector["samples_for"](tmp_path, "target", {"group/case"}, collector["input_budget"]())


@pytest.mark.parametrize(
    "mutate", ["truncated", "empty", "directory", "full_id", "missing", "linked", "special"]
)
def test_corrupt_foreign_and_special_raw_inputs_fail(collector, tmp_path, mutate):
    write_case(tmp_path)
    path = tmp_path / "group/case/new/sample.json"
    if mutate == "truncated":
        path.write_text('{"times":[')
    elif mutate == "empty":
        path.write_bytes(b"")
    elif mutate in ("directory", "full_id"):
        write_case(
            tmp_path,
            directory="wrong" if mutate == "directory" else None,
            full_id="foreign" if mutate == "full_id" else None,
        )
    elif mutate == "missing":
        path.with_name("benchmark.json").unlink()
    elif mutate == "linked":
        path.unlink()
        path.symlink_to(__file__)
    else:
        path.unlink()
        path.mkdir()
    with pytest.raises((ValueError, OSError)):
        collector["samples_for"](tmp_path, "target", {"group/case"}, collector["input_budget"]())


@pytest.mark.parametrize("budget", [{"bytes": 5, "samples": 100}, {"bytes": 10000, "samples": 1}])
def test_original_byte_and_sample_budgets_fail_without_thinning(collector, tmp_path, budget):
    write_case(tmp_path)
    with pytest.raises(ValueError, match="budget"):
        collector["samples_for"](tmp_path, "target", {"group/case"}, budget)


def test_failed_child_does_not_publish_partial_samples_and_cleans_generation(
    collector, monkeypatch
):
    outputs = process_double(collector, monkeypatch, "a: benchmark\n", lambda root: None, fail=True)
    with pytest.raises(subprocess.CalledProcessError):
        collector["run_bench"]("crate", "target", sample_size=10)
    assert all(not path.parent.exists() for path in outputs)


@pytest.mark.parametrize(
    "arguments",
    [
        ["--target", "security_actions", "--target", "unknown"],
        ["--target", "security_actions", "--target", "security_actions"],
        ["--reuse"],
        ["--target", "ipc_framing_micro"],
        ["--target", "db_read_write_micro"],
        ["--target", "vsock_framing_micro"],
        ["--target", "ledger_codec"],
    ],
)
def test_unknown_reuse_and_manual_selections_fail_before_children(
    collector, monkeypatch, arguments
):
    monkeypatch.setattr(collector["sys"], "argv", ["criterion", *arguments])
    with pytest.raises(SystemExit) as error:
        collector["main"]()
    assert error.value.code == 2


def test_default_excludes_only_declared_manual_diagnostics(collector, monkeypatch, capsys):
    called = []

    def run(crate, target, **kwargs):
        called.append((crate, target))
        return (
            {"case": [1.0]},
            {"crate": crate, "target": target},
            {"observed_cpu_lower_bound_s": {"unit": "seconds", "samples": [0.5]}},
        )

    monkeypatch.setitem(collector, "run_bench", run)
    monkeypatch.setattr(collector["sys"], "argv", ["criterion"])
    assert collector["main"]() == 0
    output = capsys.readouterr()
    diagnostic = collector["SETTINGS"].criterion.diagnostic_targets
    assert all(f"{crate}::{target}" not in diagnostic for crate, target in called)
    assert len(called) == len(collector["declared_targets"]()) - len(diagnostic)
    metrics = json.loads(output.out)["metrics"]
    assert len(metrics) == 2 * len(called)
    assert all(f"{target}.runtime.observed_cpu_lower_bound_s" in metrics for _, target in called)
    for name in diagnostic:
        assert name in output.err


def test_manual_classification_has_real_producers_and_no_criterion_macro(collector):
    targets = set(collector["declared_targets"]())
    manual = collector["SETTINGS"].criterion.diagnostic_targets
    for name, reason in manual.items():
        crate, target = name.split("::")
        assert (crate, target) in targets and reason
        source = (ROOT / "crates" / crate / "benches" / f"{target}.rs").read_text()
        assert "fn main()" in source and "criterion_main!" not in source


def test_one_target_without_cases_fails_entire_collection_without_stdout(
    collector, monkeypatch, capsys
):
    calls = []

    def run(crate, target, **kwargs):
        calls.append(target)
        if len(calls) == 2:
            raise ValueError("missing current invocation case")
        return {"case": [1.0]}, {}, {}

    monkeypatch.setitem(collector, "run_bench", run)
    monkeypatch.setattr(
        collector["sys"],
        "argv",
        ["criterion", "--target", "security_actions", "--target", "parser_sse"],
    )
    with pytest.raises(ValueError, match="missing"):
        collector["main"]()
    assert len(calls) == 2 and capsys.readouterr().out == ""


@pytest.mark.parametrize("kind", ["duplicate_id", "metric_collision"])
def test_case_identity_collisions_fail(collector, tmp_path, kind):
    write_case(tmp_path, "a")
    write_case(tmp_path, "case-group/a", full_id="a" if kind == "duplicate_id" else None)
    with pytest.raises(ValueError, match="identity"):
        collector["samples_for"](
            tmp_path, "case-group", {"a", "case-group/a"}, collector["input_budget"]()
        )


def test_case_and_listing_byte_budgets_fail_closed(collector, tmp_path, monkeypatch):
    settings = collector["SETTINGS"]
    monkeypatch.setitem(
        collector,
        "SETTINGS",
        settings.model_copy(
            update={"criterion": settings.criterion.model_copy(update={"max_cases": 1})}
        ),
    )
    write_case(tmp_path, "a")
    write_case(tmp_path, "b")
    with pytest.raises(ValueError, match="budget"):
        collector["samples_for"](tmp_path, "target", {"a", "b"}, collector["input_budget"]())
    outputs = process_double(
        collector, monkeypatch, "a: benchmark\n", lambda root: pytest.fail("must not measure")
    )
    with pytest.raises(ValueError, match="budget"):
        collector["run_bench"](
            "crate", "target", sample_size=None, budget={"bytes": 1, "samples": 100}
        )
    assert len(outputs) == 1 and not outputs[0].parent.exists()


def test_final_wire_budget_fails_before_any_stdout(collector, monkeypatch, capsys):
    settings = collector["SETTINGS"]
    monkeypatch.setitem(
        collector,
        "SETTINGS",
        settings.model_copy(
            update={"run": settings.run.model_copy(update={"max_collector_bytes": 10})}
        ),
    )
    monkeypatch.setitem(collector, "run_bench", lambda *a, **k: ({"case": [1.0]}, {}, {}))
    monkeypatch.setattr(collector["sys"], "argv", ["criterion", "--target", "security_actions"])
    with pytest.raises(ValueError, match="output budget"):
        collector["main"]()
    assert capsys.readouterr().out == ""


@pytest.mark.parametrize("quick_sample_size,max_cases", [(9, 1), (10, 0)])
def test_typed_configuration_rejects_invalid_sampling_bounds(
    collector, quick_sample_size, max_cases
):
    cls = type(collector["SETTINGS"].criterion)
    with pytest.raises(ValueError):
        cls(quick_sample_size=quick_sample_size, max_cases=max_cases, diagnostic_targets={})

"""Linux must execute the complete canonical Rust owner, not a package snapshot."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def test_linux_runner_does_not_freeze_a_package_allowlist() -> None:
    source = (ROOT / 'build_system/scripts/test/test-linux-rust.sh').read_text()
    assert 'packages=(' not in source, 'new workspace crates must enter Linux qualification automatically'


def test_linux_uses_canonical_coverage_ratchet_and_doctests() -> None:
    from capsem_builder.gate.config import load
    from capsem_builder.gate.linuxrustchecks import actions

    config = load(ROOT)
    rendered = [action.render() for action in actions(config, ROOT / 'proof-output')]
    coverage = rendered[0]
    assert all(flag in coverage.split() for flag in ('--workspace', '--bins', '--lib', '--tests', '--no-clean', '--no-fail-fast', '--locked'))
    assert all(floor in rendered[1] for floor in config.modules.rust_coverage_floors)
    assert any(config.modules.rust_coverage_ratchet in command for command in rendered)
    assert any('cargo test --workspace --doc' in command for command in rendered)
    assert any('codecov.json' in command for command in rendered)


def test_linux_keeps_failure_and_executes_later_checks() -> None:
    from capsem_builder.gate.config import load
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.invocation import Command
    from capsem_builder.gate.linuxrustchecks import actions, run
    from capsem_builder.gate.proc import Completed, Runner

    class FailingCoverage(Runner):
        def __init__(self):
            super().__init__(ROOT)
            self.seen = []

        def execute(self, command: Command) -> Completed:
            self.seen.append(command.argv)
            return Completed(command.argv, 17 if len(self.seen) == 1 else 0)

    config = load(ROOT)
    runner = FailingCoverage()
    checks = actions(config, ROOT / 'proof-output')
    assert run(Context(runner=runner, config=config, observing=True), checks) == 1
    assert len(runner.seen) == len(checks)
    assert any(argv[:2] == ('cargo', 'test') and '--doc' in argv for argv in runner.seen)


def test_native_linux_reports_use_the_configured_coverage_owner(monkeypatch) -> None:
    from capsem_builder.gate import host
    from capsem_builder.gate.config import load
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.linuxrustimage import linux_rust
    from capsem_builder.gate.plan import Plan
    from helpers.gate import RecordingRunner

    monkeypatch.setattr(host, 'on_linux', lambda: True)
    config = load(ROOT)
    runner = RecordingRunner(ROOT)
    owner = linux_rust(Plan('native-linux-output'), config)
    for action in owner.actions:
        action.perform(Context(runner=runner, config=config, observing=True))
    assert runner.commands[-1].env[config.environment.linux_rust.output_dir] == str(config.path(config.hostimage.extract_to))


def test_failed_linux_tests_still_render_lcov_before_judging_crate_floors() -> None:
    from capsem_builder.gate.config import load
    from helpers.linux_rust import coverage_commands

    config = load(ROOT)
    issued = coverage_commands(ROOT)
    execution = next(command for command in issued if command.argv[:3] == ('cargo', 'llvm-cov', 'nextest'))
    assert '--no-report' in execution.argv, 'failed test execution must not suppress report production'
    assert '--lcov' not in execution.argv and '--output-path' not in execution.argv
    report_index = next(index for index, command in enumerate(issued) if command.argv[:3] == ('cargo', 'llvm-cov', 'report') and '--lcov' in command.argv)
    ratchet_index = next(index for index, command in enumerate(issued) if any(config.modules.rust_coverage_ratchet in part for part in command.argv))
    assert report_index < ratchet_index
    report = issued[report_index]
    assert set(config.modules.rust_coverage_floors) <= set(report.argv)
    path = report.argv[report.argv.index('--output-path') + 1]
    assert Path(path).parent == config.path(config.hostimage.extract_to)
    assert path in issued[ratchet_index].argv

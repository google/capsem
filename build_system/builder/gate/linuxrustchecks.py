"""The complete Rust checks inside the already isolated Linux owner.

Reuse the native workspace command and ratchet, rather than maintaining a
second package list. Finish every check after a test failure so one compile
reports all defects; preserve failure even when later reports succeed.
"""
from __future__ import annotations

import os
from pathlib import Path

from . import rustchecks
from .actions import Action, Run
from .config import GateConfig, load
from .context import Context
from .errors import GateError
from .filesystem import make_dir
from .proc import Runner


def actions(config: GateConfig, output: Path) -> tuple[Action, ...]:
    settings = config.modules
    execution = list(settings.rust_coverage)
    # llvm-cov does not render its implicit report after a failed test run.
    # Preserve the canonical test selectors but render exactly once separately.
    position = execution.index('--output-path')
    if execution[position + 1] != settings.rust_coverage_report:
        raise GateError('canonical Rust coverage report path disagrees with its command')
    del execution[position:position + 2]
    execution.remove('--lcov')
    report = output / Path(settings.rust_coverage_report).name
    checks: list[Action] = [
        Run(
            [*execution, '--no-report', '--no-clean', '--no-fail-fast', '--locked'],
            env=rustchecks.environment(config),
        ),
        Run([*settings.rust_coverage_report_command, str(report), *settings.rust_coverage_floors]),
        rustchecks.coverage_ratchet(config, report=report),
        *rustchecks.doctests(config).actions,
    ]
    for filename, argv in config.hostimage.coverage_reports.items():
        report = output / filename
        checks.append(Run(
            [part.format(output=report) for part in argv],
            log=None if any('{output}' in part for part in argv) else report,
        ))
    return tuple(checks)


def run(context: Context, checks: tuple[Action, ...]) -> int:
    failures: list[str] = []
    for action in checks:
        try:
            action.perform(context)
        except GateError as error:
            failures.append(str(error))
            context.runner.note(str(error))
    if failures:
        context.runner.note(f'Linux Rust checks failed: {len(failures)}')
    return int(bool(failures))


def main() -> int:
    config = load(Path.cwd())
    output = Path(os.environ.get(config.environment.linux_rust.output_dir, str(config.path(config.hostimage.extract_to))))
    make_dir(output)
    return run(Context(runner=Runner(config.root), config=config), actions(config, output))

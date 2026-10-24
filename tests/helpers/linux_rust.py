"""Observe the Linux script's real shared Rust actions without running tools."""
from pathlib import Path

from capsem_builder.gate.config import load
from capsem_builder.gate.context import Context
from capsem_builder.gate.invocation import Command
from capsem_builder.gate.linuxrustchecks import actions, run
from capsem_builder.gate.shellnodes import commands
from capsem_builder.gate.shellparse import parse

from helpers.gate import RecordingRunner


def coverage_commands(root: Path) -> list[Command]:
    config = load(root)
    script = (root / config.hostimage.script).read_text()
    entry = ('uv', 'run', '--project', 'build_system', '--frozen', 'python', 'build_system/scripts/test/linux-rust-coverage.py')
    assert any(command.argv == entry for command in commands(parse(script))), 'Linux must invoke its canonical Rust owner'
    runner = RecordingRunner(root)
    assert run(Context(runner=runner, config=config, observing=True), actions(config, config.path(config.hostimage.extract_to))) == 0
    return runner.commands

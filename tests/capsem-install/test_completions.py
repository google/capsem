"""Shell completions tests for Polish: Completions + Uninstall."""

from __future__ import annotations

import pytest

from .conftest import run_capsem


class TestCompletions:
    """capsem completions command generates valid shell scripts."""

    def test_bash_completions(self, installed_layout):
        result = run_capsem("completions", "bash", timeout=10)
        assert result.returncode == 0, f"bash completions failed: {result.stderr}"
        assert "capsem" in result.stdout
        assert "complete" in result.stdout or "COMPREPLY" in result.stdout

    def test_zsh_completions(self, installed_layout):
        result = run_capsem("completions", "zsh", timeout=10)
        assert result.returncode == 0, f"zsh completions failed: {result.stderr}"
        assert "capsem" in result.stdout

    def test_fish_completions(self, installed_layout):
        result = run_capsem("completions", "fish", timeout=10)
        assert result.returncode == 0, f"fish completions failed: {result.stderr}"
        assert "capsem" in result.stdout

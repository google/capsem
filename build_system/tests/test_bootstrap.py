"""A venv's identity is its prefix, not its shared interpreter binary."""

import subprocess
import sys
import venv
from pathlib import Path

import pytest
from capsem_builder import bootstrap


@pytest.mark.parametrize("in_project_environment", [False, True])
def test_bootstrap_selects_environment_when_interpreter_binary_is_shared(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, in_project_environment: bool
) -> None:
    environment = tmp_path / "build_system" / ".venv"
    interpreter = environment / "bin" / "python"
    interpreter.parent.mkdir(parents=True)
    interpreter.symlink_to(Path(sys.executable).resolve())
    prefix = environment if in_project_environment else tmp_path / "other-environment"
    monkeypatch.setattr(sys, "prefix", str(prefix))
    calls = []
    monkeypatch.setattr(bootstrap.os, "execv", lambda program, argv: calls.append((program, argv)))
    script = tmp_path / "launcher.py"

    bootstrap.reexec_project_python(tmp_path, script, ["--help"])

    expected = (
        []
        if in_project_environment
        else [(str(interpreter), [str(interpreter), str(script), "--help"])]
    )
    assert calls == expected


def test_bootstrap_enters_real_venv_sharing_the_base_interpreter(tmp_path: Path) -> None:
    environment = tmp_path / "build_system" / ".venv"
    venv.EnvBuilder(symlinks=True).create(environment)
    script = tmp_path / "launcher.py"
    script.write_text(
        "import sys\nfrom pathlib import Path\n"
        f"sys.path.insert(0, {str(Path(bootstrap.__file__).parent)!r})\n"
        "from bootstrap import reexec_project_python\n"
        f"reexec_project_python(Path({str(tmp_path)!r}), Path(__file__), [])\n"
        "print(sys.prefix)\n"
    )

    result = subprocess.run(
        [str(Path(sys.executable).resolve()), str(script)],
        capture_output=True,
        text=True,
        timeout=10,
        check=True,
    )

    assert Path(result.stdout.strip()).resolve() == environment.resolve()

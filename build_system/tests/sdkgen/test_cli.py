"""The CLI must generate each language and report drift without writing."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.sdkgen.__main__ import main

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


@pytest.mark.parametrize("target", ["--python-package", "--typescript-source"])
def test_cli_creates_then_checks_generated_sources(target: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    package = tmp_path / "package"
    args = ["sdkgen", "--specification", str(SPEC), target, str(package)]
    monkeypatch.setattr("sys.argv", [*args, "--check"])
    assert main() == 1
    assert not package.exists()
    monkeypatch.setattr("sys.argv", args)
    assert main() == 0
    monkeypatch.setattr("sys.argv", [*args, "--check"])
    assert main() == 0


def test_cli_requires_one_language_owner(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("sys.argv", ["sdkgen", "--specification", str(SPEC),
                                    "--python-package", str(tmp_path), "--typescript-source", str(tmp_path)])
    with pytest.raises(SystemExit) as error:
        main()
    assert error.value.code == 2

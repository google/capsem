from __future__ import annotations

import importlib.util
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "install-deb-runtime-dependencies.py"
CONFIG = ROOT / "config" / "gate.toml"
SPEC = importlib.util.spec_from_file_location(
    "install_deb_runtime_dependencies",
    SCRIPT,
)
assert SPEC is not None and SPEC.loader is not None
INSTALL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALL)


class FakeRunner:
    def __init__(self, dependency_field: str = "") -> None:
        self.dependency_field = dependency_field
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self,
        command: tuple[str, ...],
        *,
        check: bool,
        capture_output: bool = False,
        text: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        self.commands.append(command)
        assert check is True
        if command[:3] == ("dpkg-deb", "--field", command[2]):
            assert capture_output is True
            assert text is True
            return subprocess.CompletedProcess(
                command,
                0,
                stdout=self.dependency_field,
                stderr="",
            )
        assert capture_output is False
        assert text is False
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")


def test_installs_dependencies_declared_by_exact_package(tmp_path: Path) -> None:
    package = tmp_path / "Capsem_1.2.3_amd64.deb"
    package.write_bytes(b"exact package")
    runner = FakeRunner(
        "libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3 (>= 1:3.20160805.1), libc6 (>= 2.39)\n"
    )

    dependencies = INSTALL.install_runtime_dependencies(package, runner=runner)

    assert dependencies == (
        "libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3 (>= 1:3.20160805.1), libc6 (>= 2.39)"
    )
    assert runner.commands == [
        ("dpkg-deb", "--field", str(package.resolve()), "Depends"),
        (
            "sudo",
            "bash",
            str((ROOT / "scripts" / "configure-apt-snapshot.sh").resolve()),
            "https://snapshot.ubuntu.com/ubuntu",
            "20260810T000000Z",
        ),
        ("sudo", "apt-get", "update"),
        (
            "sudo",
            "apt-get",
            "satisfy",
            "--yes",
            "--no-install-recommends",
            dependencies,
        ),
    ]


def test_empty_dependency_field_is_dependency_drift_and_does_not_invoke_apt(
    tmp_path: Path,
) -> None:
    package = tmp_path / "Capsem_1.2.3_amd64.deb"
    package.write_bytes(b"exact package")
    runner = FakeRunner()

    with pytest.raises(ValueError, match="differ from config authority"):
        INSTALL.install_runtime_dependencies(package, runner=runner)
    assert runner.commands == [("dpkg-deb", "--field", str(package.resolve()), "Depends")]


def test_missing_package_fails_before_any_privileged_command(tmp_path: Path) -> None:
    runner = FakeRunner("libgtk-3-0")

    with pytest.raises(FileNotFoundError, match="exact Debian package"):
        INSTALL.install_runtime_dependencies(
            tmp_path / "missing.deb",
            runner=runner,
        )

    assert runner.commands == []


def test_package_path_is_passed_as_one_argument(tmp_path: Path) -> None:
    package = tmp_path / "candidate; touch SHOULD_NOT_EXIST.deb"
    package.write_bytes(b"exact package")
    runner = FakeRunner("libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3, libc6 (>= 2.39)")

    INSTALL.install_runtime_dependencies(package, runner=runner)

    assert runner.commands[0][2] == str(package.resolve())
    assert not (tmp_path / "SHOULD_NOT_EXIST").exists()


def test_dependency_drift_fails_before_snapshot_or_apt_mutation(tmp_path: Path) -> None:
    package = tmp_path / "Capsem_1.2.3_amd64.deb"
    package.write_bytes(b"exact package")
    runner = FakeRunner("libgtk-3-0, unexpected-runtime")

    with pytest.raises(ValueError, match="differ from config authority"):
        INSTALL.install_runtime_dependencies(package, config_path=CONFIG, runner=runner)

    assert runner.commands == [("dpkg-deb", "--field", str(package.resolve()), "Depends")]


def test_verify_only_uses_the_same_authority_without_mutating_apt(tmp_path: Path) -> None:
    package = tmp_path / "Capsem_1.2.3_amd64.deb"
    package.write_bytes(b"exact package")
    declared = "libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3, libc6 (>= 2.39)"
    runner = FakeRunner(declared)

    assert (
        INSTALL.verify_runtime_dependencies(package, config_path=CONFIG, runner=runner) == declared
    )
    assert runner.commands == [("dpkg-deb", "--field", str(package.resolve()), "Depends")]


@pytest.mark.parametrize(
    "value",
    [
        "libgtk-3-0 | libgtk-4-0",
        "libgtk-3-0, libgtk-3-0",
        "libgtk-3-0; touch /tmp/nope",
    ],
)
def test_ambiguous_or_malformed_dependency_authority_is_refused(value: str) -> None:
    with pytest.raises(ValueError):
        INSTALL.dependency_names(value)

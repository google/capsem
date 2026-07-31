"""What the install gate actually proves, once the container is up.

Staging comes in two shapes, and they are not interchangeable. A release lane
supplies profile inputs its manifest already resolved and verified; a local
gate has only this checkout's freshly built assets and must publish a channel
from them before anything can install against it. Only the second produces an
authoritative release graph, which is why only the second hands one over --
see `releasegraph` for why handing over the other kind is worse than handing
over nothing.
"""

from __future__ import annotations

import time
from pathlib import Path

from . import config as gate_config
from .docker import Docker
from .errors import GateError
from .proc import Runner


class InstallProof:
    """Stages inputs, installs the package, and runs the proofs against it."""

    def __init__(
        self, runner: Runner, config: gate_config.GateConfig, *, sleep=time.sleep
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._settings = config.install
        self._guest = self._settings.guest_user
        self._suite = self._settings.suite
        self._container = self._settings.container
        self._layout = self._settings.layout
        self._mount = self._settings.mount
        self._sleep = sleep

    # -- staging -----------------------------------------------------------

    def stage_verified_inputs(self, inputs: str) -> None:
        """Use the profile inputs a release lane resolved from its manifest."""
        self._docker.shell(
            self._container,
            f'test -f "{inputs}/manifest.json" && test -f "{inputs}/release-inputs.json"',
            user=self._guest.name, cwd=self._mount,
        )
        self._docker.shell(
            self._container,
            f'rm -rf "{self._layout.assets}" "{self._layout.config}" && uv run python '
            f'{self._suite.stage_inputs_script} --input-dir "{inputs}" '
            f'--assets-dir "{self._layout.assets}" --config-root "{self._layout.config}"',
            user=self._guest.name, cwd=self._mount, env={"UV_PROJECT_ENVIRONMENT": self._settings.venv},
        )

    def stage_local_assets(self) -> None:
        """Copy this checkout's built assets, and serve them over a local URL."""
        self._docker.shell(
            self._container,
            "test -f assets/manifest.json || { echo 'ERROR: installed VM proof "
            "requires rebuilt local assets or verified pulled profile inputs' >&2; "
            "exit 1; }",
            user=self._guest.name, cwd=self._mount,
        )
        self._docker.shell(
            self._container,
            f'test -d target/config/profiles && rm -rf "{self._layout.assets}" '
            f'"{self._layout.config}" && mkdir -p "{self._layout.assets}" '
            f'"{self._layout.config}" && cp -R assets/. "{self._layout.assets}/" '
            f'&& cp -R target/config/. "{self._layout.config}/"',
            user=self._guest.name, cwd=self._mount,
        )
        self._docker.shell(
            self._container,
            f'rm -rf "{self._layout.channel}" && mkdir -p "{self._layout.channel}" '
            f"&& rm -f {self._settings.serve_ready_file}",
            user=self._guest.name, cwd=self._mount,
        )
        self._docker.shell(
            self._container,
            f'python3 {self._suite.serve_script} --root "{self._layout.channel}" '
            f"--ready-file {self._settings.serve_ready_file}",
            user=self._guest.name, cwd=self._mount, detach=True,
        )
        for _ in range(self._settings.serve_ready_attempts):
            ready = self._docker.exists(
                self._settings.serve_ready_file, self._container, user=self._guest.name
            )
            if ready:
                return
            self._sleep(self._settings.serve_ready_interval_seconds)
        raise GateError("the local release server never reported itself ready")

    # -- installation ------------------------------------------------------

    def packaged_version(self, package: str) -> str:
        return self._docker.capture(self._container, ["dpkg-deb", "-f", package, "Version"])

    def install(self, package: str, *, expected: str) -> None:
        self._runner.note(f"Installing exact release package via dpkg: {package}")
        self._docker.shell(
            self._container, f'dpkg -i "{package}" 2>&1 || apt-get install -f -y'
        )
        installed = self._docker.capture(
            self._container,
            ["dpkg-query", "-W", "-f=${Version}", self._suite.package_name],
        )
        if installed != expected:
            raise GateError(
                f"dpkg reports capsem {installed} installed, expected {expected}"
            )

    # -- proofs ------------------------------------------------------------

    def run_install_suite(self) -> None:
        self._runner.note("Running install e2e tests...")
        self._docker.shell(
            self._container,
            f"mkdir -p {self._guest.tmp} && uv run python -m pytest "
            f"{self._suite.path} -v --tb=short -o cache_dir={self._guest.pytest_cache}",
            user=self._guest.name,
            cwd=self._mount,
            env={
                "XDG_RUNTIME_DIR": self._guest.runtime_dir,
                "CAPSEM_DEB_INSTALLED": "1",
                "CAPSEM_BIN_SRC": "/usr/bin",
                "CAPSEM_TEST_ASSET_MANIFEST": self._guest.asset_manifest,
                "UV_PROJECT_ENVIRONMENT": self._settings.venv,
                "TMPDIR": self._guest.tmp,
            },
        )

    def prove_glowup(self, package: str, *, boots_a_guest: bool) -> None:
        """Install, channel switch, and upgrade against the installed package.

        Without a bootable guest this still proves Linux package and channel
        assembly; it just cannot exercise the nested VM, so it skips the
        install half rather than pretending to have run it.
        """
        if boots_a_guest:
            self._runner.note(
                "Running Linux native release glow-up (install, channel switch, upgrade)..."
            )
        else:
            self._runner.note(
                "Validating Linux package/channel assembly without unsupported "
                "nested ARM VM boot..."
            )
        glowup = (
            f'uv run python {self._suite.glowup_script} --input-deb "{package}" '
            f'--bin-dir /usr/bin --assets-dir "{self._layout.assets}" '
            f'--config-root "{self._layout.config}" '
            f"--work-dir {self._layout.glowup} --package-ready"
        )
        self._docker.shell(
            self._container,
            glowup if boots_a_guest else f"{glowup} --skip-install",
            user=self._guest.name, cwd=self._mount,
            env={
                "XDG_RUNTIME_DIR": self._guest.runtime_dir,
                "UV_PROJECT_ENVIRONMENT": self._settings.venv,
            },
        )

    def validate_macos_glowup(self, report: str | None, *, cargo_toml: Path) -> None:
        self._runner.note("Validating the native macOS installed doctor/Winterfell proof...")
        if not report:
            raise GateError(
                "macOS install rail requires the native glow-up report from this module"
            )
        self._runner.script(
            self._suite.macos_report_check,
            "--report", report,
            "--cargo-toml", cargo_toml,
        )

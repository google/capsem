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
from dataclasses import dataclass
from pathlib import Path

from .docker import Docker
from .errors import GateError
from .installimage import VENV
from .proc import Runner

SERVE_READY_FILE = "/tmp/capsem-install-release.json"
SERVE_READY_ATTEMPTS = 100
SERVE_READY_INTERVAL = 0.05


@dataclass(frozen=True)
class Layout:
    """Where the install gate keeps its scratch, all under `target/`."""

    assets: str
    config: str
    channel: str
    packages: str
    glowup: str

    @property
    def owned_paths(self) -> tuple[str, ...]:
        """Everything the container writes as its own user."""
        return (
            *(f"/src/{path}" for path in
              (self.assets, self.config, self.channel, self.packages, self.glowup)),
            "/src/release-site/node_modules",
            "/src/release-site/dist",
        )


class InstallProof:
    """Stages inputs, installs the package, and runs the proofs against it."""

    def __init__(
        self, runner: Runner, container: str, layout: Layout, *, sleep=time.sleep
    ) -> None:
        self._runner = runner
        self._docker = Docker(runner)
        self._container = container
        self._layout = layout
        self._sleep = sleep

    # -- staging -----------------------------------------------------------

    def stage_verified_inputs(self, inputs: str) -> None:
        """Use the profile inputs a release lane resolved from its manifest."""
        self._docker.shell(
            self._container,
            f'test -f "{inputs}/manifest.json" && test -f "{inputs}/release-inputs.json"',
            user="capsem", cwd="/src",
        )
        self._docker.shell(
            self._container,
            f'rm -rf "{self._layout.assets}" "{self._layout.config}" && uv run python '
            f'scripts/stage-release-test-inputs.py --input-dir "{inputs}" '
            f'--assets-dir "{self._layout.assets}" --config-root "{self._layout.config}"',
            user="capsem", cwd="/src", env={"UV_PROJECT_ENVIRONMENT": VENV},
        )

    def stage_local_assets(self) -> None:
        """Copy this checkout's built assets, and serve them over a local URL."""
        self._docker.shell(
            self._container,
            "test -f assets/manifest.json || { echo 'ERROR: installed VM proof "
            "requires rebuilt local assets or verified pulled profile inputs' >&2; "
            "exit 1; }",
            user="capsem", cwd="/src",
        )
        self._docker.shell(
            self._container,
            f'test -d target/config/profiles && rm -rf "{self._layout.assets}" '
            f'"{self._layout.config}" && mkdir -p "{self._layout.assets}" '
            f'"{self._layout.config}" && cp -R assets/. "{self._layout.assets}/" '
            f'&& cp -R target/config/. "{self._layout.config}/"',
            user="capsem", cwd="/src",
        )
        self._docker.shell(
            self._container,
            f'rm -rf "{self._layout.channel}" && mkdir -p "{self._layout.channel}" '
            f"&& rm -f {SERVE_READY_FILE}",
            user="capsem", cwd="/src",
        )
        self._docker.shell(
            self._container,
            f'python3 scripts/serve-release-test-root.py --root "{self._layout.channel}" '
            f"--ready-file {SERVE_READY_FILE}",
            user="capsem", cwd="/src", detach=True,
        )
        for _ in range(SERVE_READY_ATTEMPTS):
            if self._docker.exists(SERVE_READY_FILE, self._container, user="capsem"):
                return
            self._sleep(SERVE_READY_INTERVAL)
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
            self._container, ["dpkg-query", "-W", "-f=${Version}", "capsem"]
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
            "mkdir -p /home/capsem/tmp && cd /src && uv run python -m pytest "
            "tests/capsem-install/ -v --tb=short -o cache_dir=/home/capsem/.pytest_cache",
            user="capsem",
            env={
                "XDG_RUNTIME_DIR": "/run/user/1000",
                "CAPSEM_DEB_INSTALLED": "1",
                "CAPSEM_BIN_SRC": "/usr/bin",
                "CAPSEM_TEST_ASSET_MANIFEST": "/home/capsem/.capsem/assets/manifest.json",
                "UV_PROJECT_ENVIRONMENT": VENV,
                "TMPDIR": "/home/capsem/tmp",
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
            f'uv run python scripts/local-release-glowup.py --input-deb "{package}" '
            f'--bin-dir /usr/bin --assets-dir "{self._layout.assets}" '
            f'--config-root "{self._layout.config}" '
            f"--work-dir {self._layout.glowup} --package-ready"
        )
        self._docker.shell(
            self._container,
            glowup if boots_a_guest else f"{glowup} --skip-install",
            user="capsem", cwd="/src",
            env={"XDG_RUNTIME_DIR": "/run/user/1000", "UV_PROJECT_ENVIRONMENT": VENV},
        )

    def validate_macos_glowup(self, report: str | None, *, cargo_toml: Path) -> None:
        self._runner.note("Validating the native macOS installed doctor/Winterfell proof...")
        if not report:
            raise GateError(
                "macOS install rail requires the native glow-up report from this module"
            )
        self._runner.script(
            "scripts/check-macos-native-glowup.py",
            "--report", report,
            "--cargo-toml", cargo_toml,
        )

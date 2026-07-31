"""Authoring a local release graph, and handing it to the package installer.

The install gate proves that the exact package about to be published installs
and hydrates. To do that hermetically it needs a release graph -- and the
program that authors one, `capsem-admin`, ships *inside* the package being
installed. The shell resolved that circle by installing first and authoring
afterwards, which left the postinst with nothing to read.

The postinst does not fail in that case. `capsem_resolve_install_manifest`
falls back to the URL baked into the package when no request file exists, so
the whole-world local proof quietly hydrated from `release.capsem.org` instead
of from the artifacts under test -- and broke when those public artifacts were
retired, reporting it as a product failure.

`dpkg-deb --extract` breaks the circle: the exact `capsem-admin` being shipped,
available before its own package is installed. The order is then fixed and
worth stating, because every step depends on the one before it:

    extract admin -> stage assets -> record binary -> build graph
                  -> write handoff -> dpkg -i -> clear handoff

The handoff is the fragile link. Written after `dpkg -i` it is never read;
pointed at a file that does not exist it is refused; pointed at
`assets/manifest.json` it hands the installer the legacy runtime projection
rather than the authoritative graph, which is a different failure that also
looks like a working install.
"""

from __future__ import annotations

from .docker import Docker
from .errors import GateError

# Where the extracted-but-not-installed package tree lands, and the admin
# binary inside it. Deliberately not /usr/bin: nothing here may depend on the
# package having been installed.
PREINSTALL_ROOT = "/tmp/capsem-preinstall"
PREINSTALL_ADMIN = f"{PREINSTALL_ROOT}/usr/bin/capsem-admin"

REQUEST_SCRIPT = "/src/scripts/install-manifest-request.sh"

# The two manifests an operator can confuse. Only the first is authoritative.
GRAPH_MANIFEST = "assets/local/manifest.json"
LEGACY_PROJECTION = "manifest.json"


class ReleaseGraph:
    """Authors and publishes a local channel from inside the gate container."""

    def __init__(self, docker: Docker, container: str, *, mount: str = "/src") -> None:
        self._docker = docker
        self._container = container
        self._mount = mount
        self._handoff_written = False

    # -- breaking the circular dependency ----------------------------------

    def extract_admin(self, package: str) -> str:
        """Unpack the package without installing it, and return its admin binary.

        The binary that authors the graph is the exact binary being shipped, so
        the proof covers the code under test rather than whatever `capsem-admin`
        happened to be on the image.
        """
        self._docker.shell(
            self._container,
            f"rm -rf {PREINSTALL_ROOT} && mkdir -p {PREINSTALL_ROOT} "
            f'&& dpkg-deb --extract "{package}" {PREINSTALL_ROOT} '
            f"&& test -x {PREINSTALL_ADMIN}",
        )
        return PREINSTALL_ADMIN

    # -- authoring ---------------------------------------------------------

    def record_binary(
        self,
        admin: str,
        *,
        package: str,
        version: str,
        assets_manifest: str,
        candidate_base: str,
    ) -> None:
        """Record the candidate package and its SBOM against the staged manifest."""
        candidate_dir = f"{candidate_base}/v{version}"
        candidate_deb = f"{candidate_dir}/{package.rsplit('/', 1)[-1]}"
        sbom = f"{candidate_dir}/capsem-sbom.spdx.json"

        self._docker.shell(
            self._container,
            f'rm -rf "{candidate_base}" && mkdir -p "{candidate_dir}" '
            f'&& cp "{package}" "{candidate_deb}" '
            f'&& python3 scripts/generate-host-binary-sbom.py --output "{sbom}" "{candidate_deb}"',
            user="capsem",
            cwd=self._mount,
        )
        self._docker.exec(
            self._container,
            [
                admin, "assets", "channel", "record-binary",
                "--manifest-path", assets_manifest,
                "--version", version,
                "--artifact", candidate_deb,
                "--artifact", sbom,
            ],
            user="capsem",
            env={"CAPSEM_RELEASE_URL": f"file://{candidate_base}"},
        )

    def build_channel(
        self,
        admin: str,
        *,
        assets_dir: str,
        profiles_dir: str,
        channel: str,
        manifest_version: str,
        out_dir: str,
    ) -> str:
        """Build the authoritative graph, and return the manifest it published."""
        self._docker.shell(
            self._container,
            " ".join(
                [
                    admin, "assets", "channel", "build",
                    "--manifest", f'"file://{self._mount}/{assets_dir}/manifest.json"',
                    "--assets-dir", f'"{assets_dir}"',
                    "--profiles-dir", f'"{profiles_dir}"',
                    "--channel", channel,
                    "--manifest-version", manifest_version,
                    "--out-dir", f'"{out_dir}"',
                ]
            ),
            user="capsem",
            cwd=self._mount,
        )
        return f"{out_dir}/{GRAPH_MANIFEST}"

    def build_site(self, *, dist: str) -> None:
        """Render the release site over the generated distribution."""
        self._docker.shell(
            self._container, "pnpm install --frozen-lockfile",
            user="capsem", cwd=f"{self._mount}/release-site",
        )
        self._docker.shell(
            self._container,
            "bash scripts/check-web-surface.sh release-site-build",
            cwd=self._mount,
            env={"CAPSEM_RELEASE_CHANNEL_DIST": f"{self._mount}/{dist}"},
        )

    def check_channel(self, admin: str, *, channel: str, dist: str, manifest: str) -> None:
        self._docker.shell(
            self._container,
            f'test -f "{manifest}" '
            f'&& {admin} assets channel check --channel {channel} --dist "{dist}"',
            user="capsem",
            cwd=self._mount,
        )

    # -- the handoff -------------------------------------------------------

    def hand_off(self, manifest: str) -> None:
        """Point the package's postinst at the graph just authored.

        Refuses two mistakes the installer cannot report. A target that does
        not exist means the postinst finds no request and silently hydrates
        from the public channel; and the legacy runtime projection under
        `assets/` is not the authoritative graph, so handing it over produces
        an install that looks fine and carries the wrong manifest.
        """
        absolute = manifest if manifest.startswith("/") else f"{self._mount}/{manifest}"
        if absolute.endswith(f"/{LEGACY_PROJECTION}") and not absolute.endswith(
            f"/{GRAPH_MANIFEST}"
        ):
            raise GateError(
                f"the install handoff must select the authoritative release graph, "
                f"not the legacy runtime projection: {absolute}"
            )
        if not self._docker.exists(absolute, self._container):
            raise GateError(
                f"install handoff target does not exist: {absolute}; the postinst "
                "would find no request and hydrate from the public channel instead"
            )
        self._docker.shell(
            self._container, f"bash {REQUEST_SCRIPT} write {absolute}"
        )
        self._handoff_written = True

    def clear_handoff(self) -> None:
        """Remove the request, so a later install cannot inherit this one's."""
        if not self._handoff_written:
            return
        self._docker.shell(
            self._container, f"bash {REQUEST_SCRIPT} clear", check=False
        )
        self._handoff_written = False

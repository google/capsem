"""Citadel guard for the guest npm global-install prefix."""

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ROOTFS_DEPENDENCIES = PROJECT_ROOT / "config/docker/Dockerfile.rootfs-dependencies.j2"

GUEST_NPM_PREFIX_RATIONALE = """\
The guest rootfs is exported as a filesystem and repacked into EROFS, so OCI
ENV metadata does not survive boot. The profile-owned npm prefix must be
persisted in npm's global configuration after lockfile installation. Otherwise
`npm install -g` silently writes under the Node archive while PATH advertises
/opt/ai-clis/bin, and Doctor catches the missing command only after the full
multi-architecture asset build.

Keep the exact persistent `npm config set prefix` command. PATH or an
NPM_CONFIG_PREFIX environment variable is not an equivalent replacement.
The prefix's `bin` must remain a real directory: npm creates future global
command links there. Locked profile tools may bridge into that directory one
command at a time, but replacing it with a symlink hides later global installs.
See skills/build-images/SKILL.md and skills/ironbank/SKILL.md.
"""


def test_guest_npm_prefix_is_persisted_in_the_exported_filesystem() -> None:
    template = ROOTFS_DEPENDENCIES.read_text(encoding="utf-8")
    node_install = template.index("tar -xJf /tmp/node.tar.xz")
    persist = template.index("npm config set prefix {{ npm_prefix }} --global")
    conditional_packages = template.index("{% if npm_packages %}")

    assert node_install < persist < conditional_packages, GUEST_NPM_PREFIX_RATIONALE
    assert "test \"$(npm config get prefix)\" = '{{ npm_prefix }}'" in template, (
        GUEST_NPM_PREFIX_RATIONALE
    )
    assert "ENV NPM_CONFIG_PREFIX" not in template, GUEST_NPM_PREFIX_RATIONALE


def test_guest_npm_bin_accepts_locked_and_future_global_commands() -> None:
    template = ROOTFS_DEPENDENCIES.read_text(encoding="utf-8")

    assert "mkdir -p {{ npm_prefix }}/bin" in template, GUEST_NPM_PREFIX_RATIONALE
    assert "ln -s {{ npm_prefix }}/node_modules/.bin {{ npm_prefix }}/bin" not in template, (
        GUEST_NPM_PREFIX_RATIONALE
    )
    assert (
        'ln -s "../node_modules/.bin/$(basename "$cli")" "{{ npm_prefix }}/bin/$(basename "$cli")"'
    ) in template, GUEST_NPM_PREFIX_RATIONALE

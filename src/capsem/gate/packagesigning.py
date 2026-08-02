"""Where the local Tauri signing material lives, and how it is exported.

Its own module because it is its own question. `crosscompile` builds packages;
whether this checkout can sign one, and under what variable names, is a
property of the machine and the workflow secrets rather than of the build.

The names come from `[package.signing]` so the workflow and the gate cannot
drift: they were spelled inside a function here, and the secrets that have to
match them are spelled in YAML.
"""

from __future__ import annotations

from pathlib import Path

from . import config as gate_config


def signing_key(root: Path, config: gate_config.GateConfig) -> dict[str, str]:
    """The real Tauri release keys, if this checkout has them.

    Absent, the container generates a throwaway dev key so `cargo tauri build`
    can finish. The authoritative keys live in GitHub Actions secrets and are
    applied only on publish.

    Where they live and what they export comes from `[package.signing]`: the
    workflow secrets must agree with these names, and two places spelling them
    independently is how they stop agreeing.
    """
    settings = config.package.signing
    directory = Path(root) / settings.directory
    private = directory / settings.key
    password = directory / settings.password
    if not (private.is_file() and password.is_file()):
        return {}
    return {
        settings.key_variable: private.read_text(encoding="utf-8"),
        settings.password_variable: password.read_text(encoding="utf-8"),
    }

"""Clippy, keyed by checkout like every other workspace compile.

Every checkout shares one Cargo target directory. Workspace crates stay apart
because `build.rustc-workspace-wrapper` names a checkout-local script and
Cargo hashes that path into each workspace artifact. `cargo clippy` forces
`RUSTC_WORKSPACE_WRAPPER` to its own `clippy-driver`, one path for every
checkout, so clippy output lost that key: a checkout whose sources were older
than another checkout's newer clippy artifact took that artifact as fresh and
reported the other tree's lints, or none. In a shared target that is a false
red or a false green.

Clippy is therefore run the way `cargo clippy` runs it -- `cargo check` with
`clippy-driver` as the workspace wrapper and the lint arguments in
`CLIPPY_ARGS` -- except that the wrapper is a checkout-local script, which
restores the key. `CLIPPY_ARGS` and its separator are clippy's own contract
with `cargo clippy`; `test_clippy_checkout_key.py` proves lints still arrive on
the pinned toolchain, so a toolchain that changes it fails there first.
"""

from __future__ import annotations

import json
from collections.abc import Sequence

CLIPPY_ARGS = "CLIPPY_ARGS"
_SEPARATOR = "__CLIPPY_HACKERY__"


def invocation(
    wrapper: str, cargo_args: Sequence[str], lint_args: Sequence[str]
) -> tuple[list[str], dict[str, str]]:
    """The argv and environment of `cargo clippy <cargo_args> -- <lint_args>`,
    with clippy output keyed by `wrapper`'s resolved path.

    A relative `wrapper` resolves against the directory Cargo runs in, exactly
    as the checked-in `.cargo/config.toml` wrapper resolves against its tree.
    """
    argv = [
        "cargo",
        "check",
        "--config",
        f"build.rustc-workspace-wrapper={json.dumps(wrapper)}",
        *cargo_args,
    ]
    return argv, {CLIPPY_ARGS: "".join(f"{argument}{_SEPARATOR}" for argument in lint_args)}


def from_cargo_clippy(
    command: Sequence[str], wrapper: str
) -> tuple[list[str], dict[str, str]] | None:
    """`cargo [+toolchain] clippy [args] [-- lints]` in its keyed form.

    None for any other command, and for `clippy --fix`, which is `cargo fix`
    underneath and edits sources rather than reporting on them.
    """
    if not command or command[0] != "cargo":
        return None
    rest = list(command[1:])
    toolchain = [rest.pop(0)] if rest and rest[0].startswith("+") else []
    if not rest or rest[0] != "clippy" or "--fix" in rest:
        return None
    arguments = rest[1:]
    split = arguments.index("--") if "--" in arguments else len(arguments)
    argv, environment = invocation(wrapper, arguments[:split], arguments[split + 1 :])
    return [argv[0], *toolchain, *argv[1:]], environment

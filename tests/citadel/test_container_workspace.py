"""Citadel guard: the guest builder workspace excludes dotfiles on purpose.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards a constraint that looks exactly like an oversight,
which is why it needs a guard rather than a comment: it has already been
"fixed" once, on reasoning that was entirely sensible and entirely wrong.
"""

from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock, patch

from capsem_builder.image import guestbuilder
from capsem_builder.image.config import load_guest_config
from capsem_builder.image.docker import GUEST_BINARIES, container_compile_agent

PROJECT_ROOT = Path(__file__).resolve().parents[2]
BUILD = load_guest_config(PROJECT_ROOT / "config/docker/image").build

WORKSPACE_DOTFILE_RATIONALE = """\
The guest builder's `/src/*` glob must not be widened to match dotfiles.

`container_compile_agent` mounts the checkout read-only at /src and symlinks it
into a writable /build with:

    for f in /src/*; do b=$(basename "$f"); \\
      [ "$b" != target ] && [ "$b" != crates ] && ln -s "$f" /build/; done

`/src/*` does not match dotfiles, so `.cargo/config.toml` never reaches the
container and no guest build has ever applied the checked-in Cargo
configuration. That reads like a bug. It is not.

`.cargo/config.toml` declares:

    [target.x86_64-unknown-linux-musl]
    linker = "rust-lld"

On a developer host, `x86_64-unknown-linux-musl` is a cross target and rust-lld
is correct. Inside the Alpine builder that same triple **is the host target**,
so inheriting the file makes every proc-macro crate -- serde_derive,
tokio-macros -- link its host `.so` with rust-lld and die on:

    rust-lld: error: unable to find library -lgcc_s
    rust-lld: error: unable to find library -lc
    error: could not compile `tokio-macros`

Verified against the real image, not assumed: widening the glob to
`/src/* /src/.[!.]*` fails at tokio-macros.

The rule this generalizes to: **checked-in Cargo configuration is
developer-host configuration.** The builder container owns its own toolchain
settings and receives them as environment on the docker run -- which is why the
cross linker and `CC` are passed explicitly rather than read out of the tree.

See skills/build-images/SKILL.md, section "The guest Rust builder workspace".
"""


def _seed_identity(root: Path) -> None:
    """The identity inputs `image_tag` reads, so the call reaches the run."""
    import shutil

    for relative in (
        BUILD.guest_rust_builder.dockerfile,
        *BUILD.guest_rust_builder.identity_inputs,
    ):
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT_ROOT / relative, destination)


@patch("capsem_builder.image.docker.run_cmd")
@patch("capsem_builder.image.docker.detect_runtime", return_value="docker")
def test_container_workspace_excludes_dotfiles(
    _detect: MagicMock, run: MagicMock, tmp_path: Path
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    _seed_identity(repo)
    output = tmp_path / "output"

    def produce(cmd, **_kwargs):
        if "inspect" in cmd:
            platform = guestbuilder.environment(BUILD, "arm64").docker_platform
            return MagicMock(stdout=f"{platform}\n")
        if "run" in cmd:
            for binary in GUEST_BINARIES:
                (output / binary).write_bytes(b"elf")
        return MagicMock(stdout="")

    run.side_effect = produce
    container_compile_agent(BUILD, "arm64", repo, output)
    script = next(call.args[0] for call in run.call_args_list if "run" in call.args[0])[-1]

    problems: list[str] = []
    if "for f in /src/*; do" not in script:
        problems.append("the workspace assembly loop is no longer recognizable")
    # The two spellings that widen it to dotfiles, and the file that makes
    # doing so fatal.
    for widened in ("/src/.[!.]*", "/src/.*"):
        if widened in script:
            problems.append(f"glob widened to dotfiles with {widened}")
    if ".cargo" in script:
        problems.append("`.cargo` is being linked into the container workspace")

    assert not problems, WORKSPACE_DOTFILE_RATIONALE + "\n" + "\n".join(problems)

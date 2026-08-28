"""AF_UNIX paths the asset lane creates must fit in `sun_path`.

macOS allows 104 bytes. The gateway takes the lane's `CAPSEM_RUN_DIR` and
appends `instances/<uuid>-ws.sock` -- 36 characters of session id plus 18 of
fixed text -- so the run dir has at most ~50 to spend. `config/gate.toml`
answers that with `/tmp/capsem-a.XXXXXX`, and the comment beside it says why.

The code took the template's *name* as an `mkdtemp` prefix and dropped its
parent, so the directory landed in `$TMPDIR` -- which on macOS is
`/var/folders/<11>/<24>/T/`, 57 characters before anything else. The result
was 129 bytes, and every terminal connection failed with `path must be shorter
than SUN_LEN`: 12,024 of them in one gate run, while the VM sat at a healthy
prompt that the TUI could never show.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: macOS `sun_path`. Linux allows 108; the smaller bound is the binding one.
SUN_LEN = 104

#: What the gateway appends to a run dir: `instances/<uuid>-ws.sock`.
GATEWAY_SUFFIX = len("instances/") + 36 + len("-ws.sock")


def test_the_asset_lane_creates_its_run_dir_where_it_was_configured_to() -> None:
    """The template names a directory, not just a prefix."""
    from capsem_builder.gate import config as gate_config

    template = Path(gate_config.load(PROJECT_ROOT).assets.run_dir_template)
    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "assets.py").read_text()

    assert "mkdtemp" in source
    assert "dir=" in source, (
        f"the run dir must be created under {template.parent}; without `dir=` "
        "mkdtemp uses $TMPDIR, which on macOS is 57 characters before the "
        "session path is appended"
    )


def test_the_longest_terminal_socket_path_fits_in_sun_path() -> None:
    """The claim the template exists to satisfy, checked as arithmetic."""
    import tempfile

    from capsem_builder.gate import config as gate_config

    template = Path(gate_config.load(PROJECT_ROOT).assets.run_dir_template)
    prefix = template.name.split(".")[0] + "."
    run_dir = Path(tempfile.mkdtemp(prefix=prefix, dir=template.parent))
    try:
        # `/tmp` is a symlink on macOS and the kernel sees the resolved path.
        longest = len(str(run_dir.resolve())) + 1 + GATEWAY_SUFFIX
        assert longest < SUN_LEN, (
            f"a terminal socket under {run_dir} would be {longest} bytes, over "
            f"the {SUN_LEN}-byte limit; the gateway cannot connect and the TUI "
            "shows a session whose shell never appears"
        )
    finally:
        run_dir.rmdir()

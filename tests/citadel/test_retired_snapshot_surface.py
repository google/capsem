"""Workspace snapshots stay retired (#228).

The feature cost a rootfs.img copy per slot, ran two independent schedulers
over the same session directory, and gave the guest seven MCP tools that wrote
and reverted host-side session state. Its removal is only durable if nothing
brings back a piece of it: a route, a tool, the session ring, or the path-based
copy that followed guest symlinks.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Workspace snapshots were removed in #228, not disabled: no route, MCP tool,
scheduler, `auto_snapshots/` ring, SDK method or settings key may return.
Fork and create-from still clone a sandbox, through
capsem_foundation::unix::tree_clone, which never follows a guest path; a
subprocess `cp` or a path-based walker in that role would reintroduce the
symlink-swap host-file read the clone path was hardened against.
"""

# Production Rust and the client surfaces that could carry the feature back.
SCANNED = ("crates", "sdk/python/capsem", "sdk/typescript/src", "sdk/rust/src", "mcp/typescript/src", "web/app/src/lib")
RETIRED = re.compile(
    r"auto_snapshots"
    r"|\bsnapshots_(?:changes|list|revert|create|delete|history|compact)\b"
    r"|/snapshots/(?:status|list)"
    r"|/vms/\{id\}/changes\b"
    r"|capsem_(?:snapshots|snapshot_status|file_history)\b"
    r"|\bvm\.snapshots\b"
)
# The clone path composes foundation descriptors; it spawns nothing.
CLONE_PATH = ("crates/capsem-core/src/session", "crates/capsem-foundation/src/unix/tree_clone.rs",
              "crates/capsem-foundation/src/unix/fs")
SUBPROCESS_OR_PATH_WALK = re.compile(r"Command::new|walkdir::|WalkDir::|std::fs::copy\b")


def _is_test(path: Path) -> bool:
    parts = path.parts
    return "tests" in parts or "__tests__" in parts or path.name in {"tests.rs"} or ".test." in path.name


def _sources(roots: tuple[str, ...], root: Path = ROOT) -> dict[str, str]:
    files: dict[str, str] = {}
    for entry in roots:
        base = root / entry
        candidates = [base] if base.is_file() else sorted(base.rglob("*")) if base.exists() else []
        for path in candidates:
            if path.is_file() and path.suffix in {".rs", ".py", ".ts", ".svelte"} and not _is_test(path.relative_to(root)):
                files[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8", errors="replace")
    return files


def _retired_hits(files: dict[str, str]) -> list[str]:
    return [
        f"{name}:{number}: {line.strip()}"
        for name, text in files.items()
        for number, line in enumerate(text.splitlines(), 1)
        if RETIRED.search(line.split("//", 1)[0])
    ]


def _clone_path_hits(files: dict[str, str]) -> list[str]:
    return [
        f"{name}:{number}: {line.strip()}"
        for name, text in files.items()
        for number, line in enumerate(text.splitlines(), 1)
        if SUBPROCESS_OR_PATH_WALK.search(line.split("//", 1)[0])
    ]


def test_no_retired_snapshot_surface_returns() -> None:
    hits = _retired_hits(_sources(SCANNED))
    assert not hits, RATIONALE + "\n" + "\n".join(hits)


def test_clone_path_spawns_nothing_and_walks_no_paths() -> None:
    hits = _clone_path_hits(_sources(CLONE_PATH))
    assert not hits, RATIONALE + "\n" + "\n".join(hits)


def test_the_guard_sees_each_way_the_feature_could_return(tmp_path: Path) -> None:
    evasions = {
        "route.rs": '.route("/vms/{id}/snapshots/status", get(handler))',
        "changes.rs": 'doc.get::<X>("/vms/{id}/changes", "getVmChanges");',
        "tool.rs": '#[tool(name = "snapshots_revert")]',
        "session.rs": 'std::fs::create_dir_all(dir.join("auto_snapshots"))?;',
        "sdk.ts": "readonly snapshots = new Snapshots(ctx); vm.snapshots.list();",
        "mcp.ts": "server.registerTool('capsem_file_history', {});",
    }
    for name, line in evasions.items():
        assert _retired_hits({name: line}), name
    legitimate = {
        "suspend.rs": "ServiceToProcess::PrepareSnapshot => with_quiescence(...)",
        "ledger.rs": "capsem_logger::snapshot_session_db(&src, &dst)?;",
        "policy.rs": "let policy = snapshot_plugin_policy(&shared);",
        "comment.rs": "// the old auto_snapshots ring is gone",
    }
    for name, line in legitimate.items():
        assert not _retired_hits({name: line}), name

    for line in ('std::process::Command::new("cp")', "for entry in walkdir::WalkDir::new(src) {",
                 "std::fs::copy(src, dst)?;"):
        assert _clone_path_hits({"clone.rs": line}), line
    assert not _clone_path_hits({"clone.rs": "fs::clone_file_into(&source, dir, name, mode)?;"})

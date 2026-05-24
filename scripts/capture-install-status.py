#!/usr/bin/env python3
"""Capture evidence for the capsem install/status release gate."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


SCHEMA = "capsem.install_status_capture.v1"
EXPECTED_BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run `capsem status --json` and write a deterministic evidence bundle."
    )
    parser.add_argument(
        "--capsem-bin",
        default=os.environ.get("CAPSEM_BIN"),
        help="Path to the capsem binary. Defaults to $CAPSEM_BIN or ~/.capsem/bin/capsem.",
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        help="Directory for the evidence bundle. Defaults under test-artifacts/install-gate.",
    )
    parser.add_argument(
        "--label",
        default="status",
        help="Label used in the default output directory name.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="Timeout in seconds for each capsem command.",
    )
    parser.add_argument(
        "--debug-timeout",
        type=float,
        default=10.0,
        help="Timeout in seconds for optional `capsem debug` capture.",
    )
    parser.add_argument(
        "--skip-debug",
        action="store_true",
        help="Skip optional `capsem debug` capture.",
    )
    parser.add_argument(
        "--tree-max-depth",
        type=int,
        default=3,
        help="Maximum depth for the CAPSEM_HOME filesystem snapshot.",
    )
    parser.add_argument(
        "--tree-max-entries",
        type=int,
        default=500,
        help="Maximum number of entries in the CAPSEM_HOME filesystem snapshot.",
    )
    return parser.parse_args()


def utc_now() -> dt.datetime:
    return dt.datetime.now(dt.timezone.utc)


def safe_label(label: str) -> str:
    cleaned = "".join(ch if ch.isalnum() or ch in "-._" else "-" for ch in label.strip())
    return cleaned.strip("-._") or "status"


def default_capsem_bin() -> Path:
    return Path.home() / ".capsem" / "bin" / "capsem"


def default_out_dir(label: str) -> Path:
    stamp = utc_now().strftime("%Y%m%dT%H%M%SZ")
    return Path("test-artifacts") / "install-gate" / f"{stamp}-{safe_label(label)}"


def write_text(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def command_output(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def run_command(argv: list[str], timeout: float, env: dict[str, str]) -> dict[str, Any]:
    started = time.monotonic()
    try:
        result = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            check=False,
        )
        return {
            "argv": argv,
            "returncode": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "duration_ms": round((time.monotonic() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "argv": argv,
            "returncode": 124,
            "stdout": command_output(exc.stdout),
            "stderr": command_output(exc.stderr) or f"timed out after {timeout:g}s\n",
            "duration_ms": round((time.monotonic() - started) * 1000),
            "timed_out": True,
        }
    except FileNotFoundError as exc:
        return {
            "argv": argv,
            "returncode": 127,
            "stdout": "",
            "stderr": f"{exc}\n",
            "duration_ms": round((time.monotonic() - started) * 1000),
            "timed_out": False,
        }


def relative_name(root: Path, path: Path) -> str:
    if path == root:
        return "."
    return str(path.relative_to(root))


def snapshot_tree(root: Path, max_depth: int, max_entries: int) -> list[dict[str, Any]]:
    if not root.exists():
        return [{"path": ".", "kind": "missing"}]

    entries: list[dict[str, Any]] = []
    stack: list[tuple[Path, int]] = [(root, 0)]

    while stack and len(entries) < max_entries:
        path, depth = stack.pop()
        try:
            stat = path.lstat()
        except OSError as exc:
            entries.append(
                {
                    "path": relative_name(root, path),
                    "kind": "error",
                    "error": str(exc),
                }
            )
            continue

        item: dict[str, Any] = {
            "path": relative_name(root, path),
            "mode": oct(stat.st_mode & 0o7777),
            "size": stat.st_size,
        }

        if path.is_symlink():
            item["kind"] = "symlink"
            try:
                item["target"] = os.readlink(path)
            except OSError as exc:
                item["target_error"] = str(exc)
        elif path.is_dir():
            item["kind"] = "dir"
        elif path.is_file():
            item["kind"] = "file"
        else:
            item["kind"] = "other"

        entries.append(item)

        if item["kind"] != "dir" or depth >= max_depth:
            continue

        try:
            children = sorted(path.iterdir(), key=lambda child: child.name)
        except OSError as exc:
            entries.append(
                {
                    "path": relative_name(root, path),
                    "kind": "error",
                    "error": str(exc),
                }
            )
            continue

        for child in reversed(children):
            stack.append((child, depth + 1))

    if stack:
        entries.append(
            {
                "path": ".",
                "kind": "truncated",
                "remaining_entries": len(stack),
                "max_entries": max_entries,
            }
        )

    return entries


def file_state(path: Path, include_contents: bool = False) -> dict[str, Any]:
    item: dict[str, Any] = {"path": path.name}
    try:
        stat = path.lstat()
    except FileNotFoundError:
        item["kind"] = "missing"
        return item
    except OSError as exc:
        item["kind"] = "error"
        item["error"] = str(exc)
        return item

    item["mode"] = oct(stat.st_mode & 0o7777)
    item["size"] = stat.st_size
    if path.is_symlink():
        item["kind"] = "symlink"
        try:
            item["target"] = os.readlink(path)
        except OSError as exc:
            item["target_error"] = str(exc)
    elif path.is_dir():
        item["kind"] = "dir"
    elif path.is_file():
        item["kind"] = "file"
        if include_contents and stat.st_size <= 4096:
            try:
                item["contents"] = path.read_text(encoding="utf-8", errors="replace").strip()
            except OSError as exc:
                item["contents_error"] = str(exc)
    elif path.exists():
        item["kind"] = "other"
    else:
        item["kind"] = "missing"
    return item


def run_state(capsem_home: Path, env: dict[str, str]) -> dict[str, Any]:
    run_dir = Path(env.get("CAPSEM_RUN_DIR", str(capsem_home / "run"))).expanduser()
    entries = [
        file_state(run_dir / "service.pid", include_contents=True),
        file_state(run_dir / "service.sock"),
        file_state(run_dir / "gateway.pid", include_contents=True),
        file_state(run_dir / "gateway.port", include_contents=True),
        file_state(run_dir / "gateway.token"),
    ]
    for entry in entries:
        if entry["path"] == "gateway.token" and entry["kind"] != "missing":
            entry["contents_redacted"] = True
    return {"run_dir": str(run_dir), "entries": entries}


def saved_vm_state(capsem_home: Path, env: dict[str, str]) -> dict[str, Any]:
    run_dir = Path(env.get("CAPSEM_RUN_DIR", str(capsem_home / "run"))).expanduser()
    registry_path = run_dir / "persistent_registry.json"
    persistent_dir = run_dir / "persistent"
    state: dict[str, Any] = {
        "run_dir": str(run_dir),
        "registry": file_state(registry_path),
        "persistent_dir": file_state(persistent_dir),
        "persistent_tree": snapshot_tree(persistent_dir, max_depth=2, max_entries=200),
    }
    if not registry_path.is_file():
        return state

    try:
        raw = registry_path.read_text(encoding="utf-8")
        parsed = json.loads(raw)
    except (OSError, json.JSONDecodeError) as exc:
        state["registry_parse_error"] = str(exc)
        return state

    vms = parsed.get("vms") if isinstance(parsed, dict) else None
    if not isinstance(vms, dict):
        state["registry_parse_error"] = "registry JSON does not contain a vms object"
        return state

    summaries: dict[str, Any] = {}
    for key, value in sorted(vms.items()):
        if not isinstance(value, dict):
            summaries[str(key)] = {"invalid_entry": True}
            continue
        env_value = value.get("env")
        summary = {
            "name": value.get("name", key),
            "base_version": value.get("base_version"),
            "session_dir": value.get("session_dir"),
            "suspended": bool(value.get("suspended", False)),
            "defunct": bool(value.get("defunct", False)),
            "checkpoint_path": value.get("checkpoint_path"),
            "last_error_present": value.get("last_error") is not None,
            "env_present": env_value is not None,
        }
        if isinstance(env_value, dict):
            summary["env_keys"] = sorted(str(k) for k in env_value.keys())
        asset_references = extract_asset_references(value)
        if asset_references:
            summary["asset_references"] = asset_references
        summaries[str(key)] = summary

    state["vm_count"] = len(summaries)
    state["registry_vms"] = summaries
    return state


def extract_asset_references(entry: dict[str, Any]) -> dict[str, Any]:
    references: dict[str, Any] = {}
    for container_name in ("asset_references", "base_assets", "assets"):
        container = entry.get(container_name)
        if isinstance(container, dict):
            for key, value in container.items():
                if isinstance(value, (str, int, float, bool)) or value is None:
                    references[str(key)] = value

    for key in (
        "asset_version",
        "asset_arch",
        "kernel_hash",
        "initrd_hash",
        "rootfs_hash",
        "kernel_path",
        "initrd_path",
        "rootfs_path",
    ):
        if key not in entry:
            continue
        value = entry.get(key)
        if isinstance(value, (str, int, float, bool)) or value is None:
            references[key] = value

    file_states = {}
    for logical, key in (
        ("kernel", "kernel_path"),
        ("initrd", "initrd_path"),
        ("rootfs", "rootfs_path"),
    ):
        path = references.get(key)
        if isinstance(path, str) and path:
            file_states[logical] = file_state(Path(path))
    if file_states:
        references["files"] = file_states

    return references


def install_layout(capsem_home: Path, capsem_bin: Path, env: dict[str, str]) -> dict[str, Any]:
    bin_dir = capsem_bin.parent
    assets_dir = Path(env.get("CAPSEM_ASSETS_DIR", str(capsem_home / "assets"))).expanduser()
    service_unit = platform_service_unit_path(env)
    layout: dict[str, Any] = {
        "bin_dir": str(bin_dir),
        "binaries": {name: file_state(bin_dir / name) for name in EXPECTED_BINARIES},
        "assets_dir": str(assets_dir),
        "assets": {
            "manifest.json": file_state(assets_dir / "manifest.json"),
            "manifest.json.minisig": file_state(assets_dir / "manifest.json.minisig"),
            "manifest-sign.dev.pub": file_state(assets_dir / "manifest-sign.dev.pub"),
        },
        "setup_state": file_state(capsem_home / "setup-state.json"),
    }
    if service_unit is not None:
        layout["service_unit"] = file_state(service_unit, include_contents=True)
    if platform.system() == "Darwin":
        app_bundle = Path(env.get("CAPSEM_APP_BUNDLE", "/Applications/Capsem.app")).expanduser()
        layout["macos_app_bundle"] = file_state(app_bundle)
    return layout


def platform_service_unit_path(env: dict[str, str]) -> Path | None:
    home = Path(env.get("HOME", str(Path.home()))).expanduser()
    system = platform.system()
    if system == "Darwin":
        return home / "Library" / "LaunchAgents" / "com.capsem.service.plist"
    if system == "Linux":
        return home / ".config" / "systemd" / "user" / "capsem.service"
    return None


def json_object_summary(stdout: str) -> tuple[dict[str, Any] | None, str | None]:
    if not stdout.strip():
        return None, "empty stdout"
    try:
        value = json.loads(stdout)
    except json.JSONDecodeError as exc:
        return None, str(exc)
    if not isinstance(value, dict):
        return None, "status JSON root is not an object"
    return value, None


def main() -> int:
    args = parse_args()
    capsem_bin = Path(args.capsem_bin).expanduser() if args.capsem_bin else default_capsem_bin()
    out_dir = args.out_dir or default_out_dir(args.label)
    out_dir.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    capsem_home = Path(env.get("CAPSEM_HOME", str(Path.home() / ".capsem"))).expanduser()

    version = run_command([str(capsem_bin), "version"], args.timeout, env)
    write_text(out_dir / "version.stdout.txt", version["stdout"])
    write_text(out_dir / "version.stderr.txt", version["stderr"])

    status = run_command([str(capsem_bin), "status", "--json"], args.timeout, env)
    write_text(out_dir / "status.stdout.txt", status["stdout"])
    write_text(out_dir / "status.stderr.txt", status["stderr"])

    status_json, status_parse_error = json_object_summary(status["stdout"])
    if status_json is not None:
        write_json(out_dir / "status.json", status_json)

    debug: dict[str, Any] | None = None
    debug_parse_error: str | None = None
    if not args.skip_debug:
        run_dir = Path(env.get("CAPSEM_RUN_DIR", str(capsem_home / "run"))).expanduser()
        debug = run_command(
            [str(capsem_bin), "--uds-path", str(run_dir / "service.sock"), "debug"],
            args.debug_timeout,
            env,
        )
        write_text(out_dir / "debug.stdout.txt", debug["stdout"])
        write_text(out_dir / "debug.stderr.txt", debug["stderr"])
        debug_json, debug_parse_error = json_object_summary(debug["stdout"])
        if debug_json is not None:
            write_json(out_dir / "debug.json", debug_json)

    write_json(
        out_dir / "capsem-home-tree.json",
        snapshot_tree(capsem_home, args.tree_max_depth, args.tree_max_entries),
    )
    write_json(out_dir / "run-state.json", run_state(capsem_home, env))
    write_json(out_dir / "saved-vm-state.json", saved_vm_state(capsem_home, env))
    write_json(out_dir / "install-layout.json", install_layout(capsem_home, capsem_bin, env))

    metadata = {
        "schema": SCHEMA,
        "captured_at": utc_now().isoformat(),
        "cwd": str(Path.cwd()),
        "platform": {
            "machine": platform.machine(),
            "platform": platform.platform(),
            "python": platform.python_version(),
            "system": platform.system(),
        },
        "environment": {
            "CAPSEM_ASSETS_DIR": env.get("CAPSEM_ASSETS_DIR"),
            "CAPSEM_HOME": env.get("CAPSEM_HOME"),
            "CAPSEM_RUN_DIR": env.get("CAPSEM_RUN_DIR"),
        },
        "paths": {
            "capsem_bin": str(capsem_bin),
            "capsem_home": str(capsem_home),
            "out_dir": str(out_dir),
        },
        "commands": {
            "version": {
                "argv": version["argv"],
                "duration_ms": version["duration_ms"],
                "returncode": version["returncode"],
                "timed_out": version["timed_out"],
            },
            "status": {
                "argv": status["argv"],
                "duration_ms": status["duration_ms"],
                "returncode": status["returncode"],
                "timed_out": status["timed_out"],
            },
        },
        "status_parse_error": status_parse_error,
    }

    if debug is not None:
        metadata["commands"]["debug"] = {
            "argv": debug["argv"],
            "duration_ms": debug["duration_ms"],
            "returncode": debug["returncode"],
            "timed_out": debug["timed_out"],
        }
        metadata["debug_parse_error"] = debug_parse_error

    if status_json is not None:
        metadata["status_ok"] = status_json.get("ok")
        metadata["status_state"] = status_json.get("state")
        metadata["status_checks"] = status_json.get("checks")
        metadata["status_issue_codes"] = [
            issue.get("code")
            for issue in status_json.get("issues", [])
            if isinstance(issue, dict) and issue.get("code")
        ]

    write_json(out_dir / "capture.meta.json", metadata)
    print(out_dir)
    return int(status["returncode"])


if __name__ == "__main__":
    sys.exit(main())

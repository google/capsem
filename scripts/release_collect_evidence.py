#!/usr/bin/env python3
"""Collect a timestamped release evidence bundle without mutating the install."""

from __future__ import annotations

import argparse
import ast
import json
import subprocess
from dataclasses import dataclass, asdict
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Callable


MANUAL_GATES_PENDING = [
    "macOS package install via `just install` with user-present sudo/package context",
    "Installed `capsem status` and `capsem debug` output captured after the macOS install",
    "Installed UI/TUI smoke: profile cards, session actions, stats/detail panes, and no API 404s",
    "AGY OAuth/manual poem smoke through installed Capsem with stats and credential broker evidence",
    "Installed `capsem stop` and TUI service stop do not trigger credential prompts",
]


@dataclass(frozen=True)
class CommandResult:
    command: str
    returncode: int
    stdout: str
    stderr: str


RunCommand = Callable[[list[str], Path], CommandResult]

FORBIDDEN_INSTALLED_CREDENTIAL_MARKERS = [
    b"CAPSEM_CREDENTIAL_BROKER_TEST_STORE",
    b"org.capsem.credentials",
    b"com.capsem.credential",
    b"open default keychain",
    b"credential_store_backend_native",
    b"durable_store_write_native",
    b"durable_store_read_native",
    b"durable_store_hydrate_native",
]


def _run_command(args: list[str], cwd: Path) -> CommandResult:
    proc = subprocess.run(
        args,
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )
    return CommandResult(
        command=" ".join(args),
        returncode=proc.returncode,
        stdout=proc.stdout,
        stderr=proc.stderr,
    )


def _timestamp() -> str:
    return datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")


def _write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def _load_json(path: Path) -> dict[str, Any] | None:
    try:
        with path.open() as handle:
            data = json.load(handle)
    except (OSError, json.JSONDecodeError):
        return None
    return data if isinstance(data, dict) else None


def _benchmark_paths(project_root: Path) -> list[Path]:
    root = project_root / "benchmarks"
    if not root.exists():
        return []
    return sorted(root.glob("**/*.json"))


def _benchmark_summaries(project_root: Path) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    for path in _benchmark_paths(project_root):
        data = _load_json(path)
        if not data:
            continue
        protocol = data.get("mock_server_protocol")
        if isinstance(protocol, dict):
            for scenario in protocol.get("scenarios") or []:
                if not isinstance(scenario, dict):
                    continue
                latency = scenario.get("latency_ms") or {}
                summaries.append(
                    {
                        "source": str(path.relative_to(project_root)),
                        "kind": "mock_server_protocol",
                        "name": scenario.get("name"),
                        "sample_count": scenario.get("total_requests"),
                        "concurrency": scenario.get("concurrency"),
                        "successful": scenario.get("successful"),
                        "failed": scenario.get("failed"),
                        "requests_per_sec": scenario.get("requests_per_sec"),
                        "p50_ms": latency.get("p50"),
                        "p95_ms": latency.get("p95"),
                        "p99_ms": latency.get("p99"),
                    }
                )
            for scenario in protocol.get("websocket") or []:
                if not isinstance(scenario, dict):
                    continue
                latency = scenario.get("latency_ms") or {}
                summaries.append(
                    {
                        "source": str(path.relative_to(project_root)),
                        "kind": "mock_server_websocket",
                        "name": scenario.get("name"),
                        "sample_count": scenario.get("frames"),
                        "concurrency": None,
                        "successful": None if scenario.get("failed") else scenario.get("frames"),
                        "failed": 1 if scenario.get("failed") else 0,
                        "requests_per_sec": scenario.get("frames_per_sec"),
                        "p50_ms": latency.get("p50"),
                        "p95_ms": latency.get("p95"),
                        "p99_ms": latency.get("p99"),
                    }
                )
        throughput = data.get("throughput")
        if isinstance(throughput, dict):
            summaries.append(
                {
                    "source": str(path.relative_to(project_root)),
                    "kind": "http_throughput",
                    "name": throughput.get("url"),
                    "sample_count": throughput.get("size_bytes"),
                    "concurrency": None,
                    "successful": 1 if throughput.get("http_code") == 200 else 0,
                    "failed": 0 if throughput.get("http_code") == 200 else 1,
                    "requests_per_sec": None,
                    "p50_ms": None,
                    "p95_ms": None,
                    "p99_ms": None,
                    "throughput_mbps": throughput.get("throughput_mbps"),
                }
            )
    return summaries


def _benchmark_markdown(summaries: list[dict[str, Any]]) -> str:
    lines = [
        "# Benchmark Summary",
        "",
        "| source | kind | scenario | sample_count | c | success | failed | rps | p50 ms | p95 ms | p99 ms |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for row in summaries:
        lines.append(
            "| {source} | {kind} | {name} | {sample_count} | {concurrency} | "
            "{successful} | {failed} | {requests_per_sec} | {p50_ms} | {p95_ms} | {p99_ms} |".format(
                **{key: "" if value is None else value for key, value in row.items()}
            )
        )
    return "\n".join(lines) + "\n"


def _manual_gates_markdown() -> str:
    lines = ["# Manual Gates Pending", ""]
    lines.extend(f"- {gate}" for gate in MANUAL_GATES_PENDING)
    return "\n".join(lines) + "\n"


def _call_name(node: ast.AST) -> str | None:
    parts: list[str] = []
    current = node
    while isinstance(current, ast.Attribute):
        parts.append(current.attr)
        current = current.value
    if isinstance(current, ast.Name):
        parts.append(current.id)
        return ".".join(reversed(parts))
    return None


def _ironbank_disabled_test_findings(project_root: Path) -> list[dict[str, Any]]:
    ironbank = project_root / "tests" / "ironbank"
    if not ironbank.exists():
        return []

    findings: list[dict[str, Any]] = []
    forbidden_markers = {
        "pytest.mark.skip",
        "pytest.mark.skipif",
        "pytest.mark.slow",
        "pytest.mark.optional",
    }
    forbidden_calls = {
        "pytest.skip",
        "pytest.importorskip",
    }
    for path in sorted(ironbank.glob("test_*.py")):
        rel = str(path.relative_to(project_root))
        try:
            tree = ast.parse(path.read_text(), filename=str(path))
        except SyntaxError as error:
            findings.append(
                {
                    "path": rel,
                    "line": error.lineno,
                    "kind": "syntax_error",
                    "symbol": error.msg,
                }
            )
            continue

        for node in ast.walk(tree):
            decorators = getattr(node, "decorator_list", None)
            if decorators:
                for decorator in decorators:
                    name = _call_name(
                        decorator.func if isinstance(decorator, ast.Call) else decorator
                    )
                    if name in forbidden_markers:
                        findings.append(
                            {
                                "path": rel,
                                "line": getattr(decorator, "lineno", None),
                                "kind": "forbidden_marker",
                                "symbol": name,
                            }
                        )
            if isinstance(node, ast.Assign):
                for target in node.targets:
                    if isinstance(target, ast.Name) and target.id == "pytestmark":
                        name = _call_name(
                            node.value.func if isinstance(node.value, ast.Call) else node.value
                        )
                        if name in forbidden_markers:
                            findings.append(
                                {
                                    "path": rel,
                                    "line": node.lineno,
                                    "kind": "forbidden_pytestmark",
                                    "symbol": name,
                                }
                            )
            if isinstance(node, ast.Call):
                name = _call_name(node.func)
                if name in forbidden_calls:
                    findings.append(
                        {
                            "path": rel,
                            "line": node.lineno,
                            "kind": "forbidden_call",
                            "symbol": name,
                        }
                    )
    return findings


def _ironbank_guard(project_root: Path) -> dict[str, Any]:
    ironbank = project_root / "tests" / "ironbank"
    files_scanned = len(list(ironbank.glob("test_*.py"))) if ironbank.exists() else 0
    findings = _ironbank_disabled_test_findings(project_root)
    if findings:
        rendered = ", ".join(f"{row['path']}:{row['line']} {row['symbol']}" for row in findings)
        raise RuntimeError(f"Ironbank disabled-test guard failed: {rendered}")
    return {
        "files_scanned": files_scanned,
        "disabled_test_findings": findings,
    }


def _installed_credential_store_guard(home: Path) -> dict[str, Any]:
    bin_dir = home / ".capsem" / "bin"
    capsem_dir = home / ".capsem"
    scan_dirs = [path for path in [bin_dir, *sorted(capsem_dir.glob("bin.backup*"))] if path.exists()]
    if not scan_dirs:
        return {
            "installed_bin_dir": str(bin_dir),
            "files_scanned": 0,
            "forbidden_findings": [],
            "present": False,
        }

    findings: list[dict[str, Any]] = []
    files_scanned = 0
    for directory in scan_dirs:
        candidates: list[Path] = []
        for path in sorted(directory.glob("capsem*")):
            if path.is_file():
                candidates.append(path)
            elif path.name == "capsem-admin-python":
                findings.append(
                    {
                        "path": str(path),
                        "marker": "retired_python_admin_bundle",
                    }
                )
                candidates.extend(sorted(child for child in path.rglob("*") if child.is_file()))
        for path in candidates:
            files_scanned += 1
            try:
                payload = path.read_bytes()
            except OSError as error:
                findings.append(
                    {
                        "path": str(path),
                        "marker": "read_error",
                        "detail": str(error),
                    }
                )
                continue
            for marker in FORBIDDEN_INSTALLED_CREDENTIAL_MARKERS:
                if marker in payload:
                    findings.append(
                        {
                            "path": str(path),
                            "marker": marker.decode("utf-8"),
                        }
                    )

    if findings:
        rendered = ", ".join(f"{row['path']}:{row['marker']}" for row in findings)
        raise RuntimeError(f"Installed Capsem credential store guard failed: {rendered}")
    return {
        "installed_bin_dir": str(bin_dir),
        "files_scanned": files_scanned,
        "forbidden_findings": findings,
        "present": True,
    }


def _git_facts(project_root: Path, run_command: RunCommand) -> dict[str, Any]:
    status = run_command(["git", "status", "--short"], project_root)
    head = run_command(["git", "rev-parse", "HEAD"], project_root)
    branch = run_command(["git", "branch", "--show-current"], project_root)
    return {
        "branch": branch.stdout.strip() if branch.returncode == 0 else None,
        "commit": head.stdout.strip() if head.returncode == 0 else None,
        "dirty": bool(status.stdout.strip()) if status.returncode == 0 else None,
        "commands": {
            "status": asdict(status),
            "head": asdict(head),
            "branch": asdict(branch),
        },
    }


def _manifest_facts(project_root: Path) -> dict[str, Any]:
    manifest = _load_json(project_root / "assets" / "manifest.json") or {}
    assets = manifest.get("assets") if isinstance(manifest.get("assets"), dict) else {}
    binaries = manifest.get("binaries") if isinstance(manifest.get("binaries"), dict) else {}
    return {
        "format": manifest.get("format"),
        "current_binary": binaries.get("current"),
        "current_assets": assets.get("current"),
        "refresh_policy": manifest.get("refresh_policy"),
    }


def _file_inventory(project_root: Path) -> list[dict[str, Any]]:
    roots = [
        project_root / "benchmarks",
        project_root / "assets" / "manifest.json",
        project_root / "CHANGELOG.md",
    ]
    rows: list[dict[str, Any]] = []
    for root in roots:
        paths = [root] if root.is_file() else sorted(root.glob("**/*")) if root.exists() else []
        for path in paths:
            if path.is_file():
                rows.append(
                    {
                        "path": str(path.relative_to(project_root)),
                        "size": path.stat().st_size,
                    }
                )
    return rows


def collect_evidence(
    *,
    project_root: Path,
    output_root: Path,
    timestamp: str | None = None,
    run_command: RunCommand = _run_command,
    home: Path | None = None,
) -> Path:
    stamp = timestamp or _timestamp()
    bundle = output_root if output_root.name == stamp else output_root / stamp
    bundle.mkdir(parents=True, exist_ok=True)

    git = _git_facts(project_root, run_command)
    benchmark_summaries = _benchmark_summaries(project_root)
    manifest = _manifest_facts(project_root)
    files = _file_inventory(project_root)
    ironbank_guard = _ironbank_guard(project_root)
    installed_credential_store_guard = _installed_credential_store_guard(home or Path.home())

    _write_text(bundle / "commands" / "git-status.txt", git["commands"]["status"]["stdout"])
    _write_text(bundle / "commands" / "git-head.txt", git["commands"]["head"]["stdout"])
    _write_text(bundle / "commands" / "git-branch.txt", git["commands"]["branch"]["stdout"])
    _write_text(bundle / "benchmark-summary.md", _benchmark_markdown(benchmark_summaries))
    _write_text(bundle / "manual-gates-pending.md", _manual_gates_markdown())
    _write_text(bundle / "files.json", json.dumps(files, indent=2, sort_keys=True) + "\n")

    manifest_payload = {
        "schema": "capsem.release_evidence.v1",
        "generated_at": stamp,
        "status": "non_manual_green_manual_pending",
        "project_root": str(project_root),
        "git": {
            "branch": git["branch"],
            "commit": git["commit"],
            "dirty": git["dirty"],
        },
        "manifest": manifest,
        "ironbank_guard": ironbank_guard,
        "installed_credential_store_guard": installed_credential_store_guard,
        "benchmark_summaries": benchmark_summaries,
        "manual_gates_pending": MANUAL_GATES_PENDING,
        "files": files,
    }
    _write_text(
        bundle / "manifest.json",
        json.dumps(manifest_payload, indent=2, sort_keys=True) + "\n",
    )
    print(bundle)
    return bundle


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--project-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("test-artifacts") / "release",
    )
    parser.add_argument("--timestamp", help="Deterministic bundle timestamp")
    args = parser.parse_args(argv)

    collect_evidence(
        project_root=args.project_root.resolve(),
        output_root=args.output,
        timestamp=args.timestamp,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

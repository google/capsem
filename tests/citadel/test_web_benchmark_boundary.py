"""Keep web and benchmark ownership provable through the T5 migration."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("web_benchmark_boundary_debt.toml")
SELF = "tests/citadel/test_web_benchmark_boundary.py"
POLICY_PATH = "tests/citadel/web_benchmark_boundary_debt.toml"
TAURI_CONFIG = ROOT / "crates" / "capsem-app" / "tauri.conf.json"

RATIONALE = """\
Web applications, graphics, collectors, and reviewed evidence each have one T5
owner. Old source roots shrink to zero; Tauri and static assets must resolve;
generated recordings never become source; baseline publication is an explicit
reviewed inventory; and lint, coverage, CI, and path callers move with their
surface. See the T5 section of the approved repository cleanup proposal.
"""

OLD_WEB_ROOTS = ("frontend/", "docs/", "site/", "graphics/")
WEB_PROJECT_ROOTS = (
    *OLD_WEB_ROOTS[:3],
    "web/app/",
    "web/docs/",
    "web/marketing/",
)
REFERENCE_PATTERNS = {
    "app": re.compile(r"(?<![\w-])frontend/|['\"]frontend['\"]"),
    "docs": re.compile(r"(?<![\w-])docs/|['\"]docs['\"]"),
    "marketing": re.compile(r"(?<![\w-])site/|['\"]site['\"]"),
    "graphics": re.compile(r"(?<![\w/-])graphics/|['\"]graphics['\"]"),
    "collectors": re.compile(r"(?<![\w-])bench/collectors/|['\"]bench['\"]"),
    "baselines": re.compile(
        r"(?<![\w-])(?<!tests/)benchmarks/|['\"]benchmarks['\"]"
    ),
}
STATIC_LITERAL = re.compile(
    r"['\"](/[^'\"?#]+[.](?:png|svg|ico|wasm|ttf|woff2?))(?:[?#][^'\"]*)?['\"]"
)


@dataclass(frozen=True)
class Observed:
    old_web_sources: tuple[str, ...]
    old_collectors: tuple[str, ...]
    flat_baselines: tuple[str, ...]
    graphic_owners: tuple[str, ...]
    reviewed_baselines: tuple[str, ...]
    lint_mappings: tuple[str, ...]
    coverage_mappings: tuple[str, ...]
    legacy_references: tuple[str, ...]
    asset_problems: tuple[str, ...]
    tracked_recording_problems: tuple[str, ...]


def _git(*args: str) -> list[str]:
    result = subprocess.run(
        ("git", *args),
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr)
    return result.stdout.splitlines()


def _digest(records: tuple[str, ...]) -> str:
    return hashlib.sha256("\0".join(sorted(records)).encode()).hexdigest()


def _text_sources(tracked: tuple[str, ...]) -> dict[str, str]:
    sources: dict[str, str] = {}
    for path in tracked:
        if path in {SELF, POLICY_PATH} or path.startswith(("sprints/", "tmp/")):
            continue
        candidate = ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        raw = candidate.read_bytes()
        if b"\0" in raw:
            continue
        try:
            sources[path] = raw.decode()
        except UnicodeDecodeError:
            continue
    return sources


def _legacy_references(sources: dict[str, str]) -> tuple[str, ...]:
    return tuple(
        sorted(
            json.dumps(
                {
                    "family": family,
                    "path": path,
                    "text": " ".join(line.split()),
                },
                sort_keys=True,
                separators=(",", ":"),
            )
            for path, text in sources.items()
            for line in text.splitlines()
            for family, pattern in REFERENCE_PATTERNS.items()
            if pattern.search(line)
        )
    )


def _lint_mappings() -> tuple[str, ...]:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    required = {"web", "markdown", "json", "shell"}
    return tuple(
        sorted(
            json.dumps(surface, sort_keys=True)
            for surface in config["lint_surfaces"]
            if surface["name"] in required
        )
    )


def _coverage_mappings() -> tuple[str, ...]:
    target_web = re.compile(r"(?<![\w-])web/(?:app|docs|marketing|graphics)/")
    return tuple(
        f"{line_number}:{line.strip()}"
        for line_number, line in enumerate(
            (ROOT / "codecov.yml").read_text(encoding="utf-8").splitlines(), 1
        )
        if target_web.search(line)
        or any(pattern.search(line) for pattern in REFERENCE_PATTERNS.values())
    )


def _tauri_problems(config: dict[str, Any], tracked: frozenset[str]) -> list[str]:
    problems: list[str] = []
    base = Path("crates/capsem-app")
    canonical_icons = Path("web/graphics/tauri")
    frontend = (ROOT / base / config["build"]["frontendDist"]).resolve().relative_to(ROOT).as_posix()
    source_root = str(Path(frontend).parent).rstrip("/") + "/"
    if not any(path.startswith(source_root) for path in tracked):
        problems.append(f"Tauri frontend source is missing: {source_root}")
    entitlements = (
        (ROOT / base / config["bundle"]["macOS"]["entitlements"])
        .resolve()
        .relative_to(ROOT)
        .as_posix()
    )
    if entitlements not in tracked:
        problems.append(f"Tauri entitlements are missing: {entitlements}")
    for icon in config["bundle"]["icon"]:
        resolved = (ROOT / base / icon).resolve().relative_to(ROOT).as_posix()
        if resolved not in tracked:
            problems.append(f"Tauri icon is missing: {resolved}")
        elif not Path(resolved).is_relative_to(canonical_icons):
            problems.append(f"Tauri icon is outside canonical ownership: {resolved}")
    problems.extend(
        f"duplicate crate-local Tauri icon: {path}"
        for path in sorted(tracked)
        if path.startswith("crates/capsem-app/icons/")
    )
    return problems


def _static_asset_problems(
    sources: dict[str, str], tracked: frozenset[str]
) -> list[str]:
    problems: list[str] = []
    for path, text in sources.items():
        project = next(
            (root.rstrip("/") for root in WEB_PROJECT_ROOTS if path.startswith(root)),
            None,
        )
        if project is None or "/src/" not in path:
            continue
        for route in STATIC_LITERAL.findall(text):
            asset = f"{project}/public/{route.lstrip('/')}"
            if asset not in tracked:
                problems.append(f"{path}: missing static asset {asset}")
    return problems


def _observe() -> Observed:
    tracked = tuple(_git("ls-files"))
    tracked_set = frozenset(tracked)
    sources = _text_sources(tracked)
    old_web = tuple(path for path in tracked if path.startswith(OLD_WEB_ROOTS))
    collectors = tuple(path for path in tracked if path.startswith("bench/collectors/"))
    flat = tuple(
        path
        for path in tracked
        if path.startswith("benchmarks/")
        and not path.startswith(("benchmarks/baselines/", "benchmarks/collectors/"))
    )
    graphics = tuple(
        f"{path}->web/graphics/{path.removeprefix('graphics/')}"
        for path in tracked
        if path.startswith("graphics/")
    )
    reviewed = tuple(
        path
        for path in tracked
        if path.startswith("benchmarks/")
        and not path.startswith("benchmarks/collectors/")
    )
    recordings = tuple(
        f"tracked generated recording: {path}"
        for path in tracked
        if path.startswith("cache/target/tests/benchmarks/")
    )
    assets = _tauri_problems(
        json.loads(TAURI_CONFIG.read_text(encoding="utf-8")), tracked_set
    )
    assets.extend(_static_asset_problems(sources, tracked_set))
    return Observed(
        old_web_sources=old_web,
        old_collectors=collectors,
        flat_baselines=flat,
        graphic_owners=graphics,
        reviewed_baselines=reviewed,
        lint_mappings=_lint_mappings(),
        coverage_mappings=_coverage_mappings(),
        legacy_references=_legacy_references(sources),
        asset_problems=tuple(sorted(assets)),
        tracked_recording_problems=recordings,
    )


def _problems(policy: dict[str, Any], observed: Observed) -> list[str]:
    problems: list[str] = []
    for field, records in (
        ("old_web_source", observed.old_web_sources),
        ("old_collector", observed.old_collectors),
        ("flat_baseline", observed.flat_baselines),
        ("graphic_owner", observed.graphic_owners),
        ("reviewed_baseline", observed.reviewed_baselines),
        ("lint_mapping", observed.lint_mappings),
        ("coverage_mapping", observed.coverage_mappings),
        ("legacy_reference", observed.legacy_references),
    ):
        expected_count = policy.get(f"{field}_count")
        expected_digest = policy.get(f"{field}_sha256")
        found_digest = _digest(records)
        if len(records) != expected_count or found_digest != expected_digest:
            problems.append(
                f"{field} debt: expected count={expected_count!r} "
                f"sha256={expected_digest!r}; found count={len(records)} "
                f"sha256={found_digest}"
            )
    problems.extend(observed.asset_problems)
    problems.extend(observed.tracked_recording_problems)
    return problems


def _synthetic(**changes: tuple[str, ...]) -> Observed:
    values = dict.fromkeys(Observed.__dataclass_fields__, ())
    values.update(changes)
    return Observed(**values)


def _empty_policy() -> dict[str, Any]:
    empty = hashlib.sha256(b"").hexdigest()
    return {
        f"{field}_{suffix}": 0 if suffix == "count" else empty
        for field in (
            "old_web_source",
            "old_collector",
            "flat_baseline",
            "graphic_owner",
            "reviewed_baseline",
            "lint_mapping",
            "coverage_mapping",
            "legacy_reference",
        )
        for suffix in ("count", "sha256")
    }


@pytest.mark.parametrize(
    ("observed", "message"),
    [
        (_synthetic(old_web_sources=("frontend/new.ts",)), "old_web_source debt"),
        (_synthetic(old_collectors=("bench/collectors/new",)), "old_collector debt"),
        (_synthetic(flat_baselines=("benchmarks/new.json",)), "flat_baseline debt"),
        (
            _synthetic(graphic_owners=("graphics/new.svg->web/graphics/new.svg",)),
            "graphic_owner debt",
        ),
        (
            _synthetic(reviewed_baselines=("benchmarks/unreviewed.json",)),
            "reviewed_baseline debt",
        ),
        (_synthetic(lint_mappings=("unowned lint",)), "lint_mapping debt"),
        (_synthetic(coverage_mappings=("unowned coverage",)), "coverage_mapping debt"),
        (_synthetic(legacy_references=("caller:frontend/",)), "legacy_reference debt"),
        (_synthetic(asset_problems=("missing static asset",)), "missing static asset"),
        (
            _synthetic(
                tracked_recording_problems=(
                    "tracked generated recording: cache/target/tests/benchmarks/new.json",
                )
            ),
            "tracked generated recording",
        ),
    ],
)
def test_each_web_benchmark_violation_is_observed_red(
    observed: Observed, message: str
) -> None:
    assert any(message in problem for problem in _problems(_empty_policy(), observed)), (
        RATIONALE
    )


def test_missing_policy_fails_closed() -> None:
    assert len(_problems({}, _synthetic())) == 8, RATIONALE


def test_legacy_reference_fingerprint_ignores_line_movement_and_source_order() -> None:
    before = {
        "z.txt": "use frontend/widget here\n",
        "a.txt": "read docs/guide here\n",
    }
    after = {
        "a.txt": "unrelated preface\nread   docs/guide   here\n",
        "z.txt": "\nuse frontend/widget here\n",
    }

    assert _legacy_references(before) == _legacy_references(after)
    assert _digest(_legacy_references(before)) == _digest(_legacy_references(after))


def test_legacy_reference_fingerprint_changes_for_new_debt() -> None:
    before = _legacy_references({"caller.txt": "use frontend/widget here\n"})
    after = _legacy_references(
        {"caller.txt": "use frontend/widget here\nalso read docs/guide\n"}
    )

    assert len(after) == len(before) + 1
    assert _digest(after) != _digest(before)


def test_tauri_asset_resolver_is_observed_red() -> None:
    config = {
        "build": {"frontendDist": "../../frontend/dist"},
        "bundle": {
            "macOS": {"entitlements": "../../entitlements.plist"},
            "icon": ["icons/icon.png"],
        },
    }
    problems = _tauri_problems(config, frozenset())
    assert len(problems) == 3, RATIONALE


def test_tauri_icon_owner_and_duplicate_are_observed_red() -> None:
    config = {
        "build": {"frontendDist": "../../web/app/dist"},
        "bundle": {
            "macOS": {
                "entitlements": "../../build_system/packaging/macos/entitlements.plist"
            },
            "icon": ["icons/icon.png"],
        },
    }
    tracked = frozenset(
        {
            "web/app/src/App.svelte",
            "build_system/packaging/macos/entitlements.plist",
            "crates/capsem-app/icons/icon.png",
        }
    )
    problems = _tauri_problems(config, tracked)
    assert any("outside canonical ownership" in problem for problem in problems), RATIONALE
    assert any("duplicate crate-local Tauri icon" in problem for problem in problems), RATIONALE


def test_current_web_benchmark_boundary_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.get("version") == 1
    problems = _problems(policy, _observe())
    assert not problems, RATIONALE + "\n" + "\n".join(problems)

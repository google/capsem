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

from tests.citadel.source_enum_literals import contract_enums, reference_source
from tests.citadel.vendor_documents import (
    RUST_VENDOR_ROOT,
    pinned_rust_vendor_documents,
)

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
    "docs": re.compile(r"(?<![\w-])(?<!web/)docs/|['\"]docs['\"]"),
    "marketing": re.compile(r"(?<![\w-])site/|['\"]site['\"]"),
    "graphics": re.compile(r"(?<![\w/-])graphics/|['\"]graphics['\"]"),
    "collectors": re.compile(r"(?<![\w-])bench/collectors/|['\"]bench['\"]"),
    "baselines": re.compile(
        r"(?<![\w-])(?<!tests/)benchmarks/|['\"]benchmarks['\"]"
    ),
}
OWNED_SOURCE_PATH = re.compile(
    r"(?<![\w./-])(?:web/(?:app|docs|marketing)|benchmarks/(?:collectors|helpers))/(?:[\w.-]+/)*[\w.-]+"
)
ABSOLUTE_WEB_URL = re.compile(r"(?<![\w./-])https?://[^\s<>\"'`]+", re.IGNORECASE)


STATIC_LITERAL = re.compile(
    r"['\"](/(?!/)[^'\"?#]+[.](?:png|svg|ico|wasm|ttf|woff2?))(?:[?#][^'\"]*)?['\"]"
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


#: Verified generated third-party trees, each with the SHA256SUMS its
#: crate's build verifies. Their upstream comments are not Capsem path
#: callers -- an MDN link reads as `docs/` -- and can never be edited to stop
#: matching, so a file is skipped only while its bytes match its pin; an
#: edited one is scanned like any other source.
VENDORED_TREES = ("crates/capsem-gateway/vendor/xpra-html5",)
BENCHMARK_SOURCE_ROOTS = ("benchmarks/collectors/", "benchmarks/helpers/")


def _pinned_vendor_files(root: Path = ROOT) -> dict[str, str]:
    pinned: dict[str, str] = {}
    for tree in VENDORED_TREES:
        sums = (root / tree / "SHA256SUMS").read_text(encoding="utf-8")
        for line in sums.splitlines():
            digest, path = line.split("  ", 1)
            if path.startswith("www/"):
                pinned[f"{tree}/{path}"] = digest
    # Upstream Markdown documents describe their own repository layout. Keep
    # executable source guarded, and rescan any document changed from its pin.
    pinned.update(pinned_rust_vendor_documents(root))
    return pinned


def _text_sources(
    tracked: tuple[str, ...], root: Path = ROOT, pinned: dict[str, str] | None = None
) -> dict[str, str]:
    pinned = _pinned_vendor_files(root) if pinned is None else pinned
    sources: dict[str, str] = {}
    for path in tracked:
        if path in {SELF, POLICY_PATH} or path.startswith(("sprints/", "tmp/")):
            continue
        candidate = root / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        raw = candidate.read_bytes()
        if b"\0" in raw:
            continue
        if pinned.get(path) == hashlib.sha256(raw).hexdigest():
            continue
        try:
            sources[path] = raw.decode()
        except UnicodeDecodeError:
            continue
    return sources


def _without_owned_paths(line: str) -> str:
    # Canonical docs contain src/content/docs; the second docs is not an old root.
    return OWNED_SOURCE_PATH.sub(
        lambda match: match[0] if ".." in match[0].split("/") else "",
        ABSOLUTE_WEB_URL.sub("", line),
    )


def _legacy_references(sources: dict[str, str]) -> tuple[str, ...]:
    enums = contract_enums(sources)
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
            for line in reference_source(path, text, enums).splitlines()
            for family, pattern in REFERENCE_PATTERNS.items()
            if pattern.search(_without_owned_paths(line))
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


def _coverage_mappings_from(source: str) -> tuple[str, ...]:
    target_web = re.compile(r"(?<![\w-])web/(?:app|docs|marketing|graphics)/")
    return tuple(
        line.strip()
        for line in source.splitlines()
        if target_web.search(line)
        or any(pattern.search(line) for pattern in REFERENCE_PATTERNS.values())
    )


def _coverage_mappings() -> tuple[str, ...]:
    return _coverage_mappings_from(
        (ROOT / "codecov.yml").read_text(encoding="utf-8")
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
        # Test modules deliberately exercise rejected and nonexistent URLs.
        # They remain in legacy-reference checks, but do not deploy assets.
        if "/__tests__/" in path or re.search(r"[.](?:test|spec)[.][cm]?[jt]sx?$", path):
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
        and not path.startswith(("benchmarks/baselines/", *BENCHMARK_SOURCE_ROOTS))
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
        and not path.startswith(BENCHMARK_SOURCE_ROOTS)
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
    values: dict[str, tuple[str, ...]] = dict.fromkeys(Observed.__dataclass_fields__, ())
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


def test_only_vendored_bytes_that_match_their_pin_are_skipped(tmp_path: Path) -> None:
    pinned_text = "// https://developer.mozilla.org/en-US/docs/Web/API\n"
    vendored = "crates/capsem-gateway/vendor/xpra-html5/www/js/Pinned.js"
    edited = "crates/capsem-gateway/vendor/xpra-html5/www/js/Edited.js"
    own = "crates/capsem-gateway/src/caller.rs"
    for path, text in {
        vendored: pinned_text,
        edited: pinned_text + "read docs/guide here\n",
        own: pinned_text + "read docs/guide here\n",
    }.items():
        (tmp_path / path).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / path).write_text(text, encoding="utf-8")
    digest = hashlib.sha256(pinned_text.encode()).hexdigest()
    pinned = {vendored: digest, edited: digest}

    sources = _text_sources((vendored, edited, own), root=tmp_path, pinned=pinned)

    assert set(sources) == {edited, own}, "an edited vendored file loses its skip"
    assert {json.loads(r)["path"] for r in _legacy_references(sources)} == {edited, own}


def test_every_vendored_tree_has_its_pinned_checksums() -> None:
    pinned = _pinned_vendor_files()
    assert pinned and all(re.fullmatch(r"[0-9a-f]{64}", d) for d in pinned.values())
    assert all(path.startswith((*VENDORED_TREES, RUST_VENDOR_ROOT + "/")) for path in pinned)


def test_legacy_reference_fingerprint_changes_for_new_debt() -> None:
    before = _legacy_references({"caller.txt": "use frontend/widget here\n"})
    after = _legacy_references(
        {"caller.txt": "use frontend/widget here\nalso read docs/guide\n"}
    )

    assert len(after) == len(before) + 1
    assert _digest(after) != _digest(before)


@pytest.mark.parametrize(("path", "prefix"), [
    ("command.py", ""), ("collector", "#!/usr/bin/env python3\n"),
    ("collector", "#!/usr/bin/python3\n"),
])
@pytest.mark.parametrize("argv", ['["cargo", "bench", "--locked"]', '("cargo", "bench")'])
def test_exact_python_cargo_bench_argv_is_not_a_legacy_path(path, prefix, argv) -> None:
    assert not _legacy_references({path: prefix + "command = " + argv})
    assert _legacy_references({path: prefix + "command = " + argv + '; other = "bench/collectors/x"'})


@pytest.mark.parametrize("source", [
    'argv = ["other", "bench"]', 'argv = [tool, "bench"]',
    'argv = ["wrapper", "cargo", "bench"]', 'argv = ["cargo", *prefix, "bench"]',
    'argv = ["cargo", "bench/collectors/../../outside"]', 'value = "bench"',
    'argv = ["cargo", "bench"',
])
def test_cargo_wording_does_not_exempt_actual_or_dynamic_paths(source) -> None:
    assert _legacy_references({"command.py": source})


@pytest.mark.parametrize("prefix", ["", "#!/bin/sh\n", "# python3\n"])
def test_non_python_suffixless_content_does_not_gain_an_argv_exemption(prefix) -> None:
    assert _legacy_references({"untyped": prefix + 'argv = ["cargo", "bench"]'})


def test_external_documentation_urls_do_not_hide_adjacent_repository_callers() -> None:
    url = "https://github.com/upstream/tool/blob/main/docs/guide.md"
    assert not _legacy_references({"settings.txt": f"# {url}"}), RATIONALE
    assert _legacy_references({"source.py": f"url = '{url}'; path = 'docs/retired.md'"}), RATIONALE
    assert _legacy_references({"source.py": "prefix/https://upstream/docs/retired.md"}), RATIONALE


def test_only_byte_identical_upstream_rust_documentation_is_skipped(tmp_path: Path) -> None:
    vendor = tmp_path / VENDORED_TREES[0]
    vendor.mkdir(parents=True)
    (vendor / "SHA256SUMS").write_text("")
    tree = tmp_path / "third_party/rust/example"
    (tree / "src").mkdir(parents=True)
    doc, source = tree / "README.md", tree / "src/lib.rs"
    for path in (doc, source):
        path.write_text("docs/relative.md")
    (tree / "UPSTREAM.json").write_text(json.dumps({"files": {
        path.relative_to(tree).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in (doc, source)
    }}))
    paths = tuple(path.relative_to(tmp_path).as_posix() for path in (doc, source))
    observed = _text_sources(paths, root=tmp_path)
    assert set(observed) == {paths[1]}, "actual source remains guarded even if upstream pinned"
    doc.write_text("docs/edited.md")
    assert set(_text_sources(paths, root=tmp_path)) == set(paths), "edited documentation loses its skip"


def test_openapi_wire_enums_are_not_repository_paths() -> None:
    document: dict[str, Any] = {"openapi": "3.1.0", "components": {"schemas": {
        "Source": {"type": "string", "enum": ["frontend"]},
    }}}
    assert not _legacy_references({"contract.json": json.dumps(document)})
    document["example"] = "frontend/missing.ts"
    assert _legacy_references({"contract.json": json.dumps(document)})


@pytest.mark.parametrize(("path", "source"), [
    ("model.py", 'from enum import StrEnum\nclass Source(StrEnum):\n    FRONTEND = "frontend"\n'),
    ("model.ts", 'export enum Source {\n  FRONTEND = "frontend",\n}\n'),
])
def test_typed_wire_enums_keep_surrounding_path_checks(path: str, source: str) -> None:
    contract = {"contract.json": json.dumps({"openapi": "3.1.0", "components": {
        "schemas": {"Source": {"type": "string", "enum": ["frontend"]}},
    }})}
    assert _legacy_references({path: source})  # Ordinary path enums remain visible.
    assert not _legacy_references({**contract, path: source})
    assert _legacy_references({**contract, path: source + '\nlegacy_path = "frontend"\n'})
    assert _legacy_references({**contract, path: source.replace('"frontend"', '"frontend" # docs/guide')})


def test_coverage_mapping_fingerprint_ignores_unrelated_line_movement() -> None:
    before = "    - web/app/src/**\n    - web/docs/src/**\n"
    after = "# unrelated component\n" + before

    assert _coverage_mappings_from(before) == _coverage_mappings_from(after)


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


def test_canonical_docs_paths_are_owned_and_retired_roots_still_fail() -> None:
    assert not _legacy_references({"example.md": "web/docs/src/content/guide.md"}), RATIONALE
    canonical = "web/docs/src/content/docs/architecture/service-architecture.md"
    assert not _legacy_references({"example.md": canonical}), RATIONALE
    assert _legacy_references({"example.md": canonical + " docs/retired.md"}), RATIONALE
    assert _legacy_references({"example.md": "archive/" + canonical}), RATIONALE
    assert _legacy_references({"example.md": "web/docs/../../docs/retired.md"}), RATIONALE
    assert _legacy_references({"example.md": "docs/guide.md"}), RATIONALE
    assert _legacy_references({"example.md": "../docs/guide.md"}), RATIONALE


def test_current_web_benchmark_boundary_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.get("version") == 1
    problems = _problems(policy, _observe())
    assert not problems, RATIONALE + "\n" + "\n".join(problems)


def test_canonical_benchmark_sources_are_owned_but_misdirected_paths_fail() -> None:
    for path in ("benchmarks/collectors/stream", "benchmarks/helpers/stream_measurements.py"):
        assert not _legacy_references({"example.md": path}), RATIONALE
        assert _legacy_references({"example.md": "archive/" + path}), RATIONALE
    assert _legacy_references({"example.md": "benchmarks/collectors/../../unowned.json"}), RATIONALE
    assert _legacy_references({"example.md": "benchmarks/flat-recording.json"}), RATIONALE


def test_only_unchanged_upstream_markdown_is_outside_first_party_paths(tmp_path) -> None:
    tree = tmp_path / "third_party/rust/engine"
    tree.mkdir(parents=True)
    readme = tree / "README.md"
    readme.write_text("upstream ./docs/features.md")
    digest = hashlib.sha256(readme.read_bytes()).hexdigest()
    (tree / "UPSTREAM.json").write_text(json.dumps({"files": {
        "README.md": digest, "src/lib.rs": digest,
        "../../../AGENTS.md": digest, "/AGENTS.md": digest,
    }}))
    relative = readme.relative_to(tmp_path).as_posix()
    pins = pinned_rust_vendor_documents(tmp_path)
    assert pins == {relative: digest}
    assert _text_sources((relative,), tmp_path, pins) == {}
    readme.write_text("locally changed ./docs/features.md")
    assert relative in _text_sources((relative,), tmp_path, pins)


@pytest.mark.parametrize("path", [
    "web/app/src/lib/models/__tests__/image-metadata.test.ts",
    "web/app/src/lib/models/image-metadata.test.ts",
    "web/docs/src/icons.spec.tsx",
])
def test_asset_existence_does_not_interpret_explicit_test_module_fixtures(path: str) -> None:
    source = '"/icons/missing.svg"; "/icons/../secret.svg"; "//example.com/icon.svg"'
    assert _static_asset_problems({path: source}, frozenset()) == []
    # Test classification is narrow: these same literals in deployed modules
    # still require real assets; traversal is never resolved outside ownership.
    production = path.replace("/__tests__", "").replace(".test.ts", ".ts").replace(".spec.tsx", ".tsx")
    problems = _static_asset_problems({production: source}, frozenset())
    assert len(problems) == 2, problems
    assert any("/icons/missing.svg" in problem for problem in problems)
    assert any("/icons/../secret.svg" in problem for problem in problems)


def test_network_relative_urls_are_not_bundled_local_static_assets() -> None:
    assert not _static_asset_problems({
        "web/app/src/lib/Card.svelte": '<img src="//cdn.example.test/icon.svg">',
    }, frozenset())

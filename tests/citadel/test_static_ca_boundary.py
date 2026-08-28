"""Keep Capsem's static interception CA byte-identical under one owner."""

from __future__ import annotations

import hashlib
import re
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("static_ca_boundary_debt.toml")
SELF = "tests/citadel/test_static_ca_boundary.py"
POLICY_PATH = "tests/citadel/static_ca_boundary_debt.toml"

RATIONALE = """\
The committed interception CA is intentional product source, not an ambient
credential or a test fixture. Its certificate and key must keep their reviewed
bytes, have exactly one owner, and move with every compile/build consumer to
crates/capsem-core/resources/ca/. See T4 of the approved repository cleanup.
"""

FINAL_ROOT = "crates/capsem-core/resources/ca"
RESOURCE_NAMES = ("capsem-ca.crt", "capsem-ca.key")
CA_NAME = re.compile(r"capsem-ca[.](?:crt|key)")
LEGACY_CA = re.compile(r"security/keys/capsem-ca[.](?:crt|key)")
AMBIENT = re.compile(
    r"(?:fallback|default|alternate|exists\s*\(|is_file\s*\(|"
    r"unwrap_or|or_else|getenv|var_os)",
    re.IGNORECASE,
)
PRODUCTION_ROOTS = ("src/", "scripts/", "build_system/", "config/", ".github/")


@dataclass(frozen=True)
class Resource:
    path: str
    name: str
    sha256: str
    size: int


@dataclass(frozen=True)
class Observed:
    resources: tuple[Resource, ...]
    legacy_callers: tuple[str, ...]
    ambient_fallbacks: tuple[str, ...]


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


def _tracked_text_sources(tracked: tuple[str, ...]) -> dict[str, str]:
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


def _resource_candidates(
    tracked: tuple[str, ...], trusted_hashes: frozenset[str]
) -> tuple[Resource, ...]:
    resources: list[Resource] = []
    for path in tracked:
        candidate = ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        raw = candidate.read_bytes()
        sha256 = hashlib.sha256(raw).hexdigest()
        if candidate.name in RESOURCE_NAMES or sha256 in trusted_hashes:
            resources.append(
                Resource(
                    path=path,
                    name=candidate.name,
                    sha256=sha256,
                    size=len(raw),
                )
            )
    return tuple(sorted(resources, key=lambda resource: resource.path))


def _legacy_callers(sources: dict[str, str]) -> tuple[str, ...]:
    return tuple(
        sorted(
            f"{path}:{line_number}:{line.strip()}"
            for path, text in sources.items()
            for line_number, line in enumerate(text.splitlines(), 1)
            if LEGACY_CA.search(line)
        )
    )


def _ambient_fallbacks(sources: dict[str, str]) -> tuple[str, ...]:
    return tuple(
        sorted(
            f"{path}:{line_number}:{line.strip()}"
            for path, text in sources.items()
            if path.startswith(PRODUCTION_ROOTS)
            if not path.startswith("build_system/tests/")
            for line_number, line in enumerate(text.splitlines(), 1)
            if CA_NAME.search(line) and AMBIENT.search(line)
        )
    )


def _observe(policy: dict[str, Any]) -> Observed:
    tracked = tuple(_git("ls-files"))
    sources = _tracked_text_sources(tracked)
    trusted = frozenset(
        resource["sha256"] for resource in policy.get("resources", {}).values()
    )
    return Observed(
        resources=_resource_candidates(tracked, trusted),
        legacy_callers=_legacy_callers(sources),
        ambient_fallbacks=_ambient_fallbacks(sources),
    )


def _inventory_problems(policy: dict[str, Any], observed: Observed) -> list[str]:
    problems: list[str] = []
    expected = policy.get("resources", {})
    observed_by_path = {resource.path: resource for resource in observed.resources}
    expected_paths = {
        details.get("path") for details in expected.values() if details.get("path")
    }

    for name in RESOURCE_NAMES:
        details = expected.get(name)
        if not details:
            problems.append(f"missing resource policy: {name}")
            continue
        path = details.get("path")
        resource = observed_by_path.get(path)
        if resource is None:
            problems.append(f"missing core-owned static CA resource: {path}")
            continue
        if resource.name != name:
            problems.append(f"wrong static CA name at {path}: {resource.name}")
        if resource.sha256 != details.get("sha256"):
            problems.append(f"altered static CA bytes: {path}")
        if resource.size != details.get("size"):
            problems.append(f"altered static CA size: {path}")

    unexpected = sorted(
        resource.path
        for resource in observed.resources
        if resource.path not in expected_paths
    )
    if unexpected:
        problems.append(f"unexpected static CA owner or duplicate: {unexpected}")

    for name in RESOURCE_NAMES:
        trusted_hash = expected.get(name, {}).get("sha256")
        owners = sorted(
            resource.path
            for resource in observed.resources
            if resource.name == name or resource.sha256 == trusted_hash
        )
        if len(owners) > 1:
            problems.append(f"duplicate static CA resource {name}: {owners}")
    return problems


def _debt_problems(policy: dict[str, Any], observed: Observed) -> list[str]:
    problems: list[str] = []
    for field, records in (
        ("legacy_caller", observed.legacy_callers),
        ("ambient_fallback", observed.ambient_fallbacks),
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
    return problems


def _problems(policy: dict[str, Any], observed: Observed) -> list[str]:
    problems = _inventory_problems(policy, observed)
    problems.extend(_debt_problems(policy, observed))
    return problems


def _final_policy() -> dict[str, Any]:
    empty = hashlib.sha256(b"").hexdigest()
    return {
        "resources": {
            "capsem-ca.crt": {
                "path": f"{FINAL_ROOT}/capsem-ca.crt",
                "sha256": "fdedfba5141babd1f62d38f7a512ede47aac7a2f688ec643f55976af7cafcf1b",
                "size": 615,
            },
            "capsem-ca.key": {
                "path": f"{FINAL_ROOT}/capsem-ca.key",
                "sha256": "56189a9fd3149947a5169470cb124c08d58781bd354b5c22eae7b93d3e5cc36f",
                "size": 241,
            },
        },
        "legacy_caller_count": 0,
        "legacy_caller_sha256": empty,
        "ambient_fallback_count": 0,
        "ambient_fallback_sha256": empty,
    }


def _resource(path: str, name: str, sha256: str, size: int) -> Resource:
    return Resource(path=path, name=name, sha256=sha256, size=size)


def test_duplicate_static_ca_copy_is_observed_red() -> None:
    policy = _final_policy()
    cert = policy["resources"]["capsem-ca.crt"]
    observed = Observed(
        resources=(
            _resource(cert["path"], "capsem-ca.crt", cert["sha256"], cert["size"]),
            _resource("copy/cert.pem", "cert.pem", cert["sha256"], cert["size"]),
        ),
        legacy_callers=(),
        ambient_fallbacks=(),
    )
    assert any("duplicate static CA" in problem for problem in _problems(policy, observed)), (
        RATIONALE
    )


def test_missing_core_owned_resource_is_observed_red() -> None:
    observed = Observed(resources=(), legacy_callers=(), ambient_fallbacks=())
    assert any(
        "missing core-owned" in problem
        for problem in _problems(_final_policy(), observed)
    ), RATIONALE


def test_stale_legacy_caller_is_observed_red() -> None:
    observed = Observed(
        resources=(),
        legacy_callers=('src/build.py:1:"security/keys/capsem-ca.crt"',),
        ambient_fallbacks=(),
    )
    assert any(
        "legacy_caller debt" in problem
        for problem in _problems(_final_policy(), observed)
    ), RATIONALE


def test_changed_ca_bytes_are_observed_red() -> None:
    policy = _final_policy()
    cert = policy["resources"]["capsem-ca.crt"]
    observed = Observed(
        resources=(
            _resource(cert["path"], "capsem-ca.crt", "0" * 64, cert["size"]),
        ),
        legacy_callers=(),
        ambient_fallbacks=(),
    )
    assert any("altered static CA bytes" in problem for problem in _problems(policy, observed)), (
        RATIONALE
    )


def test_ambient_fallback_is_observed_red() -> None:
    observed = Observed(
        resources=(),
        legacy_callers=(),
        ambient_fallbacks=("src/build.py:1:fallback capsem-ca.crt",),
    )
    assert any(
        "ambient_fallback debt" in problem
        for problem in _problems(_final_policy(), observed)
    ), RATIONALE


def test_test_assertions_are_not_production_fallbacks() -> None:
    records = _ambient_fallbacks(
        {
            "build_system/tests/image/test_docker.py": (
                'assert (context_dir / "capsem-ca.crt").is_file()'
            ),
            "build_system/builder/image/doctor.py": (
                'fallback = root / "capsem-ca.crt" if alternate.exists() else primary'
            ),
        }
    )
    assert records == (
        (
            'build_system/builder/image/doctor.py:1:fallback = root / "capsem-ca.crt" '
            "if alternate.exists() else primary"
        ),
    )


def test_missing_policy_fails_closed() -> None:
    observed = Observed(resources=(), legacy_callers=(), ambient_fallbacks=())
    assert len(_problems({}, observed)) >= 4, RATIONALE


def test_current_static_ca_boundary_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.get("version") == 1, RATIONALE
    assert policy.get("transition_item") == "S05-006", RATIONALE
    problems = _problems(policy, _observe(policy))
    assert not problems, RATIONALE + "\n" + "\n".join(problems)

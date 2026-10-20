"""Citadel guard: VM profiles stay removed.

A VM profile was a named bundle of rootfs, packages, policy and MCP config.
It was deleted outright (google/capsem#289): policy now comes from built-in
defaults, settings.toml and corp.toml; the runtime is one minimal rootfs;
applications come from OCI images. Nothing was aliased or migrated, so the
concept's vocabulary reappearing anywhere is the concept growing back.

"Profile" alone is not banned: it also names the security-rule tier
(`profiles.rules.*`, `SecurityRuleProfile`), Seatbelt sandbox profiles, cargo
and Nextest build profiles and `/etc/profile`. The guard bans the spellings
only a VM profile ever used.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

NO_VM_PROFILES_RATIONALE = """\
VM profile reintroduction.

VM profiles were removed as a clean break (google/capsem#289): no profile
ledgers or catalog, no profile_id in the API, no /profiles routes, no
--profile flag on capsem or capsem-admin, no profile-owned rootfs inputs, no
profile plumbing in the gate or the tests. Policy is built-in defaults +
settings.toml + corp.toml, written per VM to vm/active_policy.toml; the
runtime is one rootfs (`just build-assets`, released by `just release-assets`);
applications come from OCI images.

Do not reintroduce the concept. If a file must name it to refuse or clean up
after it, add the file to REMOVAL_SITES with that reason.
"""

# Spellings that only ever meant a VM profile. Each is matched as written.
VM_PROFILE_VOCABULARY: tuple[tuple[str, str], ...] = (
    (r"\bprofile_?[iI]d\b|\bprofileId\b", "a request or record naming a profile"),
    (r"\bdefault_profile", "the per-runtime default profile"),
    (
        r"\bProfile(Catalog|Config|ConfigFile|Runtime|Availability|Asset\w*|Obom\w*|Skills|VmDefaults|FileDescriptor)\b",
        "profile ledger types",
    ),
    (r"profile[-_]catalog", "the profile catalog"),
    (r"config/profiles\b|\bprofiles/(code|co-work)\b|\bprofile\.toml\b", "profile ledger files"),
    (r"\bactive_profile\b|\bActiveProfile", "the per-VM profile file, now active_policy"),
    (r"CAPSEM_(PROFILES_DIR|TEST_PROFILE|ACTIVE_PROFILE)|CODE_PROFILE_ID", "profile plumbing in the environment"),
    (
        r"\bmaterialize(d)?_(test_)?profiles?\b|\bprofiles?_dir\b|\bprofiles_subdir\b|\bprofiles_glob\b",
        "profile materialization",
    ),
    (r"\bprofiles_(ready|total)\b|Profiles: +\d", "profile readiness"),
    (r"[\"'`]/profiles\b", "the /profiles HTTP routes"),
    (r"\brelease[-_]profile\b", "the per-profile release lane"),
    (r"capsem-admin[\"', ]+profile\b|\bprofile (validate|check|materialize)\b", "the capsem-admin profile commands"),
    (r"\bprofile_(revision|payload_hash|root|content)\b|\bprofile-root\b", "profile pins and seeds"),
    (r"/opt/ai-clis", "the profile-installed AI CLI prefix"),
    (r"(capsem|run|create|assets|mcp)[\"', ]+(\w+[\"', ]+)?--profile\b", "the removed --profile flag"),
)
FORBIDDEN = re.compile("|".join(f"(?:{pattern})" for pattern, _ in VM_PROFILE_VOCABULARY))

# History is not product: release notes, the changelog and recorded benchmark
# evidence (JSON under the benchmarks root) describe what was.
HISTORY = re.compile(r"^(CHANGELOG\.md|LATEST_RELEASE\.md|bench[^/]*/.*\.json)$")

# Files that must name the removed concept to refuse it or clean up after it.
REMOVAL_SITES: dict[str, str] = {
    "tests/citadel/test_no_vm_profiles.py": "this guard",
    "crates/capsem-service/src/registry.rs": (
        "reads the `profile_id` an entry written before the removal carries, so resume can refuse it by name"
    ),
    "crates/capsem-service/src/registry/tests.rs": "proves such an entry keeps its marker through a save",
    "crates/capsem/src/update.rs": "removes the retired ~/.capsem/profiles catalog on `capsem update --yes`",
    "crates/capsem/src/update/verified_update.rs": "the no-follow removal of the retired catalog",
    "crates/capsem/src/update/runtime_contract_tests.rs": "proves that removal never follows a link",
}


def tracked() -> list[str]:
    output = subprocess.run(["git", "ls-files"], cwd=PROJECT_ROOT, check=True, capture_output=True, text=True).stdout
    return [path for path in output.splitlines() if (PROJECT_ROOT / path).is_file()]


def violations(path: str, text: str) -> list[str]:
    return [
        f"{path}:{number}: {line.strip()}"
        for number, line in enumerate(text.splitlines(), start=1)
        if FORBIDDEN.search(line)
    ]


def _read(path: str) -> str | None:
    try:
        return (PROJECT_ROOT / path).read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return None


def test_no_profile_inputs_are_tracked() -> None:
    offenders = [p for p in tracked() if p.startswith("config/profiles/") or p == "config/profile-catalog.toml"]
    assert offenders == [], NO_VM_PROFILES_RATIONALE + "\n" + "\n".join(offenders)


def test_the_repository_does_not_name_vm_profiles() -> None:
    offenders: list[str] = []
    for path in tracked():
        if HISTORY.match(path) or path in REMOVAL_SITES:
            continue
        text = _read(path)
        if text is not None:
            offenders.extend(violations(path, text))
    assert offenders == [], NO_VM_PROFILES_RATIONALE + "\n" + "\n".join(offenders)


def test_removal_sites_still_exist_and_still_need_the_exemption() -> None:
    # An exemption outliving its reason is an exemption for whatever comes next.
    stale = [
        path for path in REMOVAL_SITES if not (PROJECT_ROOT / path).is_file() or not violations(path, _read(path) or "")
    ]
    assert stale == [], "REMOVAL_SITES entries no longer needed:\n" + "\n".join(stale)


def test_no_profile_routes_in_the_public_surface() -> None:
    surface = (PROJECT_ROOT / "config/public-surface.toml").read_text(encoding="utf-8")
    assert "/profiles" not in surface, NO_VM_PROFILES_RATIONALE


def test_no_clap_profile_argument() -> None:
    # A `profile` field on a clap struct is `--profile` without the literal.
    offenders = []
    for path in tracked():
        if path.startswith(("crates/capsem/src/", "crates/capsem-admin/src/")) and path.endswith(".rs"):
            for number, line in enumerate((_read(path) or "").splitlines(), start=1):
                if re.search(r"^\s*(pub(\(crate\))?\s+)?profile:\s", line):
                    offenders.append(f"{path}:{number}: {line.strip()}")
    assert offenders == [], NO_VM_PROFILES_RATIONALE + "\n" + "\n".join(offenders)


def test_no_profile_recipe() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    assert not re.search(r"(?m)^_?[\w-]*profile[\w-]*\b[^:\n]*:", justfile), NO_VM_PROFILES_RATIONALE


@pytest.mark.parametrize(
    "line",
    [
        "    pub profile_id: String,",
        'body = {"profileId": "code"}',
        "let catalog = ProfileCatalog::load_default();",
        'include_str!("../../config/profiles/code/profile.toml")',
        'path = vm_dir.join("active_profile.toml")',
        "CAPSEM_PROFILES_DIR=/tmp/p",
        "service.profiles_dir = materialize_test_profiles(tmp)",
        'export PATH="/opt/ai-clis/bin:$PATH"',
        "Profiles:  2/2 ready",
        'client.get("/profiles/list")',
        "just release-profile nightly code HEAD",
        '["capsem-admin", "profile", "materialize"]',
        '["capsem", "run", "--profile", "code", "true"]',
        'run(["create", "--profile", CODE])',
    ],
)
def test_reintroductions_are_caught(line: str) -> None:
    assert violations("x", line), line


@pytest.mark.parametrize(
    "line",
    [
        "[profiles.rules.block_secret_host]",
        "let profile = SecurityRuleProfile::parse_toml(text)?;",
        "use capsem_config::{SecurityRuleProfile, ProviderRuleProfile};",
        "cargo build --profile release",
        "[profile.release]",
        "cat > /newroot/etc/profile.d/capsem.sh << 'PROFILE'",
        '"LD_PROFILE",',
        'profile_name = "gate.sb"',
        'result["io_profile"] = io_profile_bench(path)',
        'rust_test_profile_variable = "NEXTEST_PROFILE"',
    ],
)
def test_unrelated_meanings_pass(line: str) -> None:
    assert not violations("x", line), violations("x", line)

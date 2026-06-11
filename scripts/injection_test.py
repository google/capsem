#!/usr/bin/env python3
"""End-to-end boot-config test for non-secret settings materialization.

Each scenario writes a temporary settings.toml (and optionally corp.toml), boots the VM
with `capsem-doctor -k injection`, and checks the exit code. The in-VM tests read
/tmp/capsem-injection-manifest.json to verify the emitted boot env/files are
well-formed.

Usage:
    python3 scripts/injection_test.py              # uses target/debug/capsem
    python3 scripts/injection_test.py --binary ./capsem --assets ./assets
"""

import argparse
import os
import subprocess
import sys
import tempfile

BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RESET = "\033[0m"


class Results:
    """Accumulates pass/fail results for a clean summary."""

    def __init__(self):
        self.passed: list[str] = []
        self.failed: list[str] = []

    def ok(self, msg: str):
        self.passed.append(msg)
        print(f"  {GREEN}PASS{RESET}  {msg}")

    def fail(self, msg: str):
        self.failed.append(msg)
        print(f"  {RED}FAIL{RESET}  {msg}")

    def check(self, cond: bool, pass_msg: str, fail_msg: str):
        if cond:
            self.ok(pass_msg)
        else:
            self.fail(fail_msg)

    @property
    def success(self) -> bool:
        return len(self.failed) == 0


# -- Scenario definitions --
# Each scenario is a dict with:
#   name: human-readable label
#   settings_toml: TOML string for <CAPSEM_HOME>/settings.toml
#   corp_toml: optional TOML string for CAPSEM_CORP_CONFIG (None = no corp override)
#
# Runtime AI credentials are intentionally absent here. Provider access and
# credential brokerage now flow through profile/corp security rules plus plugins,
# not settings-owned AI toggles or static boot-time secret injection.

SCENARIOS = [
    {
        "name": "git_identity",
        "description": "Non-secret git identity and repository toggles materialize cleanly",
        "settings_toml": """\
[settings]
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.git.identity.author_name" = { value = "Test User", modified = "2026-01-01T00:00:00Z" }
"repository.git.identity.author_email" = { value = "test@example.com", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "broker_refs_not_boot_secrets",
        "description": "Brokered repository credential references are accepted but not materialized as raw boot secrets",
        "settings_toml": """\
[settings]
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.token" = { value = "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "empty_tokens",
        "description": "Repository providers on with empty tokens -- no credential file should be emitted",
        "settings_toml": """\
[settings]
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.token" = { value = "", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "corp_rule_file",
        "description": "Corp rule config loads without resurrecting settings-owned AI provider toggles",
        "settings_toml": """\
[settings]
"repository.git.identity.author_name" = { value = "Corp Test User", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": """\
[corp.rules.block_example_invalid]
name = "block_example_invalid"
action = "block"
priority = -100
detection_level = "high"
reason = "Integration proof that corp rules own enforcement."
match = 'http.host == "example.invalid"'
""",
    },
]


def run_scenario(
    binary: str,
    assets_dir: str,
    scenario: dict,
    results: Results,
) -> None:
    """Write temp config(s), boot VM with capsem-doctor -k injection, check exit code."""
    name = scenario["name"]
    print(f"\n{BOLD}--- Scenario: {name} ---{RESET}")
    print(f"  {DIM}{scenario['description']}{RESET}")

    # Write temporary settings.toml inside an isolated Capsem home.
    capsem_home = tempfile.TemporaryDirectory(prefix=f"capsem-injection-{name}-home-")
    settings_path = os.path.join(capsem_home.name, "settings.toml")
    with open(settings_path, "w") as settings_file:
        settings_file.write(scenario["settings_toml"])

    # Write temporary corp.toml if specified.
    corp_path = None
    if scenario.get("corp_toml"):
        corp_file = tempfile.NamedTemporaryFile(
            mode="w", suffix=".toml", prefix=f"capsem-injection-{name}-corp-", delete=False,
        )
        corp_file.write(scenario["corp_toml"])
        corp_file.close()
        corp_path = corp_file.name

    env = {
        **os.environ,
        "CAPSEM_ASSETS_DIR": assets_dir,
        "RUST_LOG": "capsem=warn",
        "CAPSEM_HOME": capsem_home.name,
    }
    if corp_path:
        env["CAPSEM_CORP_CONFIG"] = corp_path
    else:
        # Ensure no stale corp config leaks through.
        env.pop("CAPSEM_CORP_CONFIG", None)

    vm_command = "capsem-doctor -k injection"
    try:
        proc = subprocess.run(
            [binary, "run", vm_command],
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )
        exit_code = proc.returncode
        stdout = proc.stdout.strip()
        stderr = proc.stderr.strip()

        if exit_code == 0:
            results.ok(f"{name}: all injection tests passed")
        else:
            results.fail(f"{name}: injection tests failed (exit {exit_code})")
            # Show full output so failures are easy to diagnose.
            if stdout:
                print(f"    {CYAN}--- stdout ---{RESET}")
                for line in stdout.splitlines():
                    color = RED if ("FAILED" in line or "AssertionError" in line) else ""
                    end = RESET if color else ""
                    print(f"    {color}{line}{end}")
            if stderr:
                print(f"    {YELLOW}--- stderr ---{RESET}")
                for line in stderr.splitlines():
                    print(f"    {line}")
    except subprocess.TimeoutExpired:
        results.fail(f"{name}: VM timed out after 120s")
    finally:
        # Clean up temp files.
        os.unlink(user_file.name)
        if corp_path:
            os.unlink(corp_path)


def main():
    parser = argparse.ArgumentParser(
        description="End-to-end non-secret boot config test.",
    )
    parser.add_argument(
        "--binary",
        default="target/debug/capsem",
        help="Path to the capsem binary (default: target/debug/capsem)",
    )
    parser.add_argument(
        "--assets",
        default="assets",
        help="Path to VM assets directory (default: assets)",
    )
    parser.add_argument(
        "--scenario",
        default=None,
        help="Run only this scenario (by name). Default: run all.",
    )
    args = parser.parse_args()

    print(f"{BOLD}=== Capsem Injection Test ==={RESET}")
    print(f"  binary: {args.binary}")
    print(f"  assets: {args.assets}")

    results = Results()

    scenarios = SCENARIOS
    if args.scenario:
        scenarios = [s for s in SCENARIOS if s["name"] == args.scenario]
        if not scenarios:
            names = ", ".join(s["name"] for s in SCENARIOS)
            print(f"{RED}Unknown scenario: {args.scenario}. Available: {names}{RESET}")
            sys.exit(1)

    for scenario in scenarios:
        run_scenario(args.binary, args.assets, scenario, results)

    # Summary.
    print(f"\n{BOLD}{'=' * 60}{RESET}")
    total = len(results.passed) + len(results.failed)
    print(
        f"  {GREEN}{len(results.passed)} passed{RESET}"
        f"  {RED}{len(results.failed)} failed{RESET}"
        f"  ({total} scenarios)"
    )
    if results.success:
        print(f"  {GREEN}{BOLD}INJECTION TEST PASSED{RESET}\n")
    else:
        print(f"  {RED}{BOLD}INJECTION TEST FAILED{RESET}\n")
    sys.exit(0 if results.success else 1)


if __name__ == "__main__":
    main()

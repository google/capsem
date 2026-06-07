#!/usr/bin/env python3
"""End-to-end injection test: generate configs, boot VMs, verify all injection paths.

Each scenario writes a temporary user.toml (and optionally corp.toml), boots the VM
with `capsem-doctor -k injection`, and checks the exit code. The in-VM tests read
/tmp/capsem-injection-manifest.json to verify every env var and file arrived.

Usage:
    python3 scripts/injection_test.py              # uses target/debug/capsem
    python3 scripts/injection_test.py --binary ./capsem --assets ./assets
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

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
#   user_toml: TOML string for CAPSEM_USER_CONFIG
#   corp_toml: optional TOML string for CAPSEM_CORP_CONFIG (None = no corp override)

SCENARIOS = [
    {
        "name": "all_enabled",
        "description": "All AI providers on, both repo tokens set, git identity set",
        "user_toml": """\
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.google.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.openai.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-test-key-injection", modified = "2026-01-01T00:00:00Z" }
"ai.google.api_key" = { value = "AIzaSy_test_key_injection", modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "ghp_test_token_injection", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.token" = { value = "glpat-test_token_injection", modified = "2026-01-01T00:00:00Z" }
"repository.git.identity.author_name" = { value = "Test User", modified = "2026-01-01T00:00:00Z" }
"repository.git.identity.author_email" = { value = "test@example.com", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "partial",
        "description": "Only Google enabled, only GitHub token, no git identity",
        "user_toml": """\
[settings]
"ai.anthropic.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"ai.google.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.openai.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"ai.google.api_key" = { value = "AIzaSy_partial_key", modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "ghp_partial_token", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "all_disabled",
        "description": "All providers off, tokens set but allow=false -- .git-credentials must NOT exist",
        "user_toml": """\
[settings]
"ai.anthropic.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"ai.google.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"ai.openai.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "ghp_should_not_appear", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.token" = { value = "glpat-should_not_appear", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "empty_tokens",
        "description": "Providers on but tokens empty -- .git-credentials must NOT exist",
        "user_toml": """\
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.google.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.openai.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.github.token" = { value = "", modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"repository.providers.gitlab.token" = { value = "", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": None,
    },
    {
        "name": "corp_override",
        "description": "User enables all, corp blocks Anthropic -- CAPSEM_ANTHROPIC_ALLOWED=0",
        "user_toml": """\
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.google.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.openai.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-corp-test-key", modified = "2026-01-01T00:00:00Z" }
"ai.google.api_key" = { value = "AIzaSy_corp_test_key", modified = "2026-01-01T00:00:00Z" }
""",
        "corp_toml": """\
[settings]
"ai.anthropic.allow" = { value = false, modified = "2026-01-01T00:00:00Z" }
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

    # Write temporary user.toml.
    user_file = tempfile.NamedTemporaryFile(
        mode="w", suffix=".toml", prefix=f"capsem-injection-{name}-user-", delete=False,
    )
    user_file.write(scenario["user_toml"])
    user_file.close()

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
        "CAPSEM_USER_CONFIG": user_file.name,
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
        description="End-to-end injection test for capsem boot config.",
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

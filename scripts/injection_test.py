#!/usr/bin/env python3
"""End-to-end injection test: generate Profile V2 state, boot VMs, verify injection paths.

Each scenario writes temporary `service.toml` and profile TOML under an isolated
CAPSEM_HOME, boots the VM with `capsem-doctor -k injection`, and checks the exit
code. The in-VM tests read /tmp/capsem-injection-manifest.json to verify every
env var and file arrived.

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


def _provider_sections(anthropic: bool, google: bool, openai: bool) -> str:
    return f"""\
[ai.providers.anthropic]
enabled = {str(anthropic).lower()}
credential_refs = ["anthropic-api-key"]

[ai.providers.google]
enabled = {str(google).lower()}
credential_refs = ["google-api-key"]

[ai.providers.openai]
enabled = {str(openai).lower()}
credential_refs = ["openai-api-key"]
"""


def _profile_toml(profile_id: str, provider_sections: str) -> str:
    return f"""\
version = 1
id = "{profile_id}"
name = "Injection {profile_id}"
best_for = "Injection diagnostics."
profile_type = "coding"
extends_profile_id = "everyday-work"

{provider_sections}
"""


def _service_toml(profile_id: str, profile_dir: Path) -> str:
    return f"""\
version = 1

[profiles]
user_dirs = ["{profile_dir}"]
default_profile = "{profile_id}"

[credentials.items.anthropic-api-key]
value = "sk-ant-test-key-injection"

[credentials.items.google-api-key]
value = "AIzaSy_test_key_injection"

[credentials.items.openai-api-key]
value = "sk-test-key-injection"

[credentials.items.github-token]
value = "ghp_test_token_injection"
"""


# -- Scenario definitions --
# Each scenario selects a temporary Profile V2 profile.

SCENARIOS = [
    {
        "name": "all_enabled",
        "description": "All AI providers on through Profile V2",
        "providers": (True, True, True),
    },
    {
        "name": "partial",
        "description": "Only Google enabled through Profile V2",
        "providers": (False, True, False),
    },
    {
        "name": "all_disabled",
        "description": "All providers off through Profile V2",
        "providers": (False, False, False),
    },
]


def run_scenario(
    binary: str,
    assets_dir: str,
    scenario: dict,
    results: Results,
) -> None:
    """Write temporary Profile V2 state, boot VM, and check the doctor exit code."""
    name = scenario["name"]
    print(f"\n{BOLD}--- Scenario: {name} ---{RESET}")
    print(f"  {DIM}{scenario['description']}{RESET}")

    try:
        with tempfile.TemporaryDirectory(prefix=f"capsem-injection-{name}-") as capsem_home:
            capsem_home_path = Path(capsem_home)
            profile_dir = capsem_home_path / "profiles"
            profile_dir.mkdir(parents=True, exist_ok=True)
            profile_id = f"injection-{name.replace('_', '-')}"
            profile_path = profile_dir / f"{profile_id}.toml"
            profile_path.write_text(
                _profile_toml(profile_id, _provider_sections(*scenario["providers"])),
                encoding="utf-8",
            )
            (capsem_home_path / "service.toml").write_text(
                _service_toml(profile_id, profile_dir),
                encoding="utf-8",
            )

            env = {
                **os.environ,
                "CAPSEM_ASSETS_DIR": assets_dir,
                "CAPSEM_HOME": str(capsem_home_path),
                "CAPSEM_RUN_DIR": str(capsem_home_path / "run"),
                "RUST_LOG": "capsem=warn",
            }

            vm_command = "capsem-doctor -k injection"
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

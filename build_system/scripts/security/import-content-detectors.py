"""Reproduce the reviewed native detector subset from pinned clean checkouts.

Developer-only generator: no network, imports or execution of upstream code.
Run through run-bounded-command.py with --presidio and --gitleaks checkout paths.
"""

from __future__ import annotations

import argparse
import ast
import json
import subprocess
import tomllib
from pathlib import Path

PRESIDIO = "4f0e236e31a57767958a3eec2de3fc58dffc3034"
GITLEAKS = "b58d3f102cf3a2c84cb7f923d05c25c9b1aed84b"
SELECTED = {
    "github-pat",
    "github-fine-grained-pat",
    "github-oauth",
    "github-app-token",
    "anthropic-api-key",
    "openai-api-key",
    "aws-access-token",
    "slack-bot-token",
    "private-key",
}


def checked(path: Path, commit: str) -> Path:
    head = subprocess.check_output(["git", "-C", str(path), "rev-parse", "HEAD"], text=True).strip()
    dirty = subprocess.check_output(["git", "-C", str(path), "status", "--porcelain"], text=True)
    if head != commit or dirty:
        raise ValueError(f"expected clean pinned checkout {commit}: {path}")
    return path


def patterns(path: Path) -> list[str]:
    # Parse Python syntax as data; never import recognizer modules or install NLP.
    tree = ast.parse(path.read_text())
    return [
        ast.literal_eval(node.args[1])
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "Pattern"
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--presidio", type=Path, required=True)
    parser.add_argument("--gitleaks", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    recognizers = (
        checked(args.presidio, PRESIDIO)
        / "presidio-analyzer/presidio_analyzer/predefined_recognizers"
    )
    rules = tomllib.loads((checked(args.gitleaks, GITLEAKS) / "config/gitleaks.toml").read_text())[
        "rules"
    ]
    detectors = [
        ("email", patterns(recognizers / "generic/email_recognizer.py")[0], 0.0),
        ("ssn", patterns(recognizers / "country_specific/us/us_ssn_recognizer.py")[-1], 0.0),
        # Rust regex has no lookaround; native code performs Luhn/SSN checks.
        ("card", r"\b[13456][0-9]{3}(?:[- ]?[0-9]){9,15}\b", 0.0),
        # Deliberately narrower than Presidio's phonenumbers-based recognizer.
        ("phone", r"\b(?:1[- .]?)?[2-9][0-9]{2}[- .][2-9][0-9]{2}[- .][0-9]{4}\b", 0.0),
    ]
    imported = [rule for rule in rules if rule["id"] in SELECTED]
    if {rule["id"] for rule in imported} != SELECTED:
        raise ValueError("reviewed Gitleaks rule inventory changed")
    detectors.extend(
        (rule["id"], rule["regex"], float(rule.get("entropy", 0))) for rule in imported
    )
    detectors.append(("broker-reference", r"credential:blake3:[0-9a-fA-F]{64}", 0.0))
    lines = ["# Reviewed subset; see DETECTOR_SOURCES.md. No runtime downloads."]
    for name, pattern, entropy in detectors:
        # JSON string quoting is compatible with TOML basic strings. This is
        # source generation, never a runtime payload conversion or sizing pass.
        lines.extend(
            [
                "[[detectors]]",
                "id = " + json.dumps(name),
                "pattern = " + json.dumps(pattern),
                "entropy = " + str(entropy),
                "",
            ]
        )
    args.output.write_text("\n".join(lines))


if __name__ == "__main__":
    main()

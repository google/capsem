"""Profile payload contracts that must hold before image materialization."""

from __future__ import annotations

import tomllib
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "config" / "profiles"


def _profile_payload(profile_dir: Path) -> tuple[dict, Path, Path]:
    profile_path = profile_dir / "profile.toml"
    profile = tomllib.loads(profile_path.read_text())
    build_path = PROJECT_ROOT / "config" / profile["files"]["build"]["path"]
    requirements_path = PROJECT_ROOT / "config" / profile["files"]["python_requirements"]["path"]
    return profile, build_path, requirements_path


def test_profiles_ship_ollama_without_cuda_payload_bloat() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile, build_path, requirements_path = _profile_payload(profile_dir)
        build_script = build_path.read_text()
        requirements = {
            line.strip()
            for line in requirements_path.read_text().splitlines()
            if line.strip() and not line.startswith("#")
        }

        profile_id = profile["id"]
        if "https://ollama.com/install.sh" not in build_script:
            failures.append(f"{profile_id}: build script does not install Ollama")
        if "rm -rf /usr/local/lib/ollama/cuda_*" not in build_script:
            failures.append(f"{profile_id}: build script does not prune Ollama CUDA libraries")
        if "ollama" not in requirements:
            failures.append(f"{profile_id}: python requirements do not include the Ollama SDK")

    assert not failures, "invalid profile payload contract:\n" + "\n".join(failures)

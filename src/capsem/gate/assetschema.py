"""Typed boot-asset and evidence authority from ``config/gate.toml``."""

from __future__ import annotations

from pydantic import model_validator

from .configschema import Strict


class ArtifactsConfig(Strict):
    """The three files a bootable per-architecture asset tree is made of."""

    kernel: str
    initrd: str
    rootfs: str

    @property
    def bootable(self) -> tuple[str, ...]:
        """What must exist for a tree to boot, in build order."""
        return (self.kernel, self.initrd, self.rootfs)


class AssetsConfig(Strict):
    test_root: str
    profiles_glob: str
    evidence_artifacts: tuple[str, ...]
    obom_artifact: str
    failure_tail_lines: int
    shell_proof_timeout_seconds: int
    run_dir_template: str
    admin_command: tuple[str, ...]
    capsem_binary: str
    hash_assets_script: str
    shell_proof_script: str
    container_cleanup_script: str
    cross_platform_probe_command: str
    cross_platform_probe_network: str
    merged_assets_dir: str
    merged_config_dir: str
    profile_home_dir: str
    failure_evidence_dir: str
    materialized_profiles_dir: str
    current_link: str
    evidence_suffixes: tuple[str, ...]
    evidence_prune_dirs: tuple[str, ...]

    @model_validator(mode="after")
    def semantic_evidence_is_declared(self) -> AssetsConfig:
        if self.obom_artifact not in self.evidence_artifacts:
            raise ValueError("assets.obom_artifact must be one of assets.evidence_artifacts")
        return self

"""Finite installed proof inputs; no implicit artifact selection or build."""

from pathlib import PurePosixPath

from pydantic import Field, PositiveInt, field_validator

from .configschema import Strict


class InstallBenchmarkConfig(Strict):
    inputs: str
    work_root: str
    macos_owner: str
    owner_timeout_seconds: PositiveInt = Field(le=7200)
    cleanup_timeout_seconds: PositiveInt = Field(le=600)
    max_input_bytes: PositiveInt = Field(le=10485760)
    max_receipt_bytes: PositiveInt = Field(le=67108864)
    max_package_bytes: PositiveInt = Field(le=2147483648)

    @field_validator("inputs", "work_root")
    @classmethod
    def generated_path(cls, value: str) -> str:
        path = PurePosixPath(value)
        if path.is_absolute() or ".." in path.parts or path.parts[:2] != ("cache", "target"):
            raise ValueError("Install benchmark inputs/output must be cache/target owned")
        return value

    @field_validator("macos_owner")
    @classmethod
    def existing_owner(cls, value: str) -> str:
        if value != "build_system/packaging/macos/macos_tart_glowup.py":
            raise ValueError("Install uses the existing Tart owner only")
        return value

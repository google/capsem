"""Canonical pure Dockerfile helpers; no host discovery or engine authority.

Source provenance: Pierre Tholoniat, bb61fc82d44bc42c978c4acf5435a1975600c75e.
File loading, grants, build execution and ownership belong to their controllers.
"""

from .dockerfile_ca import (
    _CA_ENV,
    _CA_READY,
    _CA_STAGE_LINES,
    _CA_UPDATE_LINE,
    _IMAGE_CA,
    _IMAGE_CA_BUNDLE,
    _extract_ca_fingerprint,
    _stage_has_any_run,
    _stage_runs_shell,
    is_root_user_spec,
    patch_dockerfile_for_capsem_ca,
)
from .dockerfile_metadata import (
    _DOCKERFILE_VAR_RE,
    _expand_dockerfile_vars,
    _extract_dockerfile_metadata,
    _strip_optional_quotes,
    _update_dockerfile_vars,
)
from .dockerfile_scan import (
    _ESCAPE_DIRECTIVE_RE,
    _HEREDOC_TOKEN_RE,
    _dockerfile_instructions,
    _find_heredoc_openers_detailed,
    _scan_dockerfile_instructions,
)

__all__ = [
    "_CA_ENV",
    "_CA_READY",
    "_CA_STAGE_LINES",
    "_CA_UPDATE_LINE",
    "_DOCKERFILE_VAR_RE",
    "_ESCAPE_DIRECTIVE_RE",
    "_HEREDOC_TOKEN_RE",
    "_IMAGE_CA",
    "_IMAGE_CA_BUNDLE",
    "_dockerfile_instructions",
    "_expand_dockerfile_vars",
    "_extract_ca_fingerprint",
    "_extract_dockerfile_metadata",
    "_find_heredoc_openers_detailed",
    "_scan_dockerfile_instructions",
    "_stage_has_any_run",
    "_stage_runs_shell",
    "_strip_optional_quotes",
    "_update_dockerfile_vars",
    "is_root_user_spec",
    "patch_dockerfile_for_capsem_ca",
]

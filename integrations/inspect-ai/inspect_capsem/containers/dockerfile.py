"""Canonical pure Dockerfile helpers; no host discovery or engine authority.

Source provenance: Pierre Tholoniat, bb61fc82d44bc42c978c4acf5435a1975600c75e.
File loading, grants, build execution and ownership belong to their controllers.
"""

from .dockerfile_ca import _CA_ENV as _CA_ENV
from .dockerfile_ca import _CA_READY as _CA_READY
from .dockerfile_ca import _CA_STAGE_LINES as _CA_STAGE_LINES
from .dockerfile_ca import _CA_UPDATE_LINE as _CA_UPDATE_LINE
from .dockerfile_ca import _IMAGE_CA as _IMAGE_CA
from .dockerfile_ca import _IMAGE_CA_BUNDLE as _IMAGE_CA_BUNDLE
from .dockerfile_ca import _extract_ca_fingerprint as _extract_ca_fingerprint
from .dockerfile_ca import _stage_has_any_run as _stage_has_any_run
from .dockerfile_ca import _stage_runs_shell as _stage_runs_shell
from .dockerfile_ca import is_root_user_spec as is_root_user_spec
from .dockerfile_ca import patch_dockerfile_for_capsem_ca as patch_dockerfile_for_capsem_ca
from .dockerfile_heredoc import _escape_unquoted_heredoc_line as _escape_unquoted_heredoc_line
from .dockerfile_heredoc import _format_printf_heredoc_cmd as _format_printf_heredoc_cmd
from .dockerfile_heredoc import (
    _lower_single_instruction_heredocs as _lower_single_instruction_heredocs,
)
from .dockerfile_heredoc import _shell_single_quote as _shell_single_quote
from .dockerfile_heredoc import lower_dockerfile_heredocs as lower_dockerfile_heredocs
from .dockerfile_metadata import _DOCKERFILE_VAR_RE as _DOCKERFILE_VAR_RE
from .dockerfile_metadata import _expand_dockerfile_vars as _expand_dockerfile_vars
from .dockerfile_metadata import _extract_dockerfile_metadata as _extract_dockerfile_metadata
from .dockerfile_metadata import _strip_optional_quotes as _strip_optional_quotes
from .dockerfile_metadata import _update_dockerfile_vars as _update_dockerfile_vars
from .dockerfile_scan import _ESCAPE_DIRECTIVE_RE as _ESCAPE_DIRECTIVE_RE
from .dockerfile_scan import _HEREDOC_TOKEN_RE as _HEREDOC_TOKEN_RE
from .dockerfile_scan import _dockerfile_instructions as _dockerfile_instructions
from .dockerfile_scan import _find_heredoc_openers_detailed as _find_heredoc_openers_detailed
from .dockerfile_scan import _scan_dockerfile_instructions as _scan_dockerfile_instructions

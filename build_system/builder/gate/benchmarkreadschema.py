"""Bounded read-only route cases and independently seeded corpus."""
from typing import Literal

from pydantic import Field, PositiveInt

from .benchmarknetworkschema import DiagnosticCorpusConfig
from .configschema import Strict


class RouteCaseConfig(Strict):
    """One successful HTTP method, template and distinct fixture case."""

    method: Literal["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"]
    path: str
    case: str
    parameters: dict[str, str] = Field(default_factory=dict)
    query: dict[str, str | int] = Field(default_factory=dict)
    response: Literal["json", "text", "binary", "warc", "health", "openapi", "html", "javascript", "diagnostics"] = "json"
    target_scope: Literal["both", "gateway"] = "both"
    samples: PositiveInt | None = None
    load_requests: PositiveInt | None = None
    concurrency: PositiveInt | None = None


class RouteDebtConfig(Strict):
    """An uncovered operation names its existing fixture owner and gap."""

    owner: str = Field(min_length=1)
    reason: str = Field(min_length=20)


class RouteCorpusConfig(Strict):
    """Exact bounded read corpus, retained in the benchmark config cohort."""

    file_sizes: tuple[PositiveInt, ...]
    archive_facts_max_bytes: PositiveInt
    archive_inflated_max_bytes: PositiveInt
    archive_max_records: PositiveInt
    warc_info_max_bytes: PositiveInt
    workspace_files: PositiveInt
    log_bytes: PositiveInt
    log_line_bytes: PositiveInt
    log_tail: PositiveInt
    log_read_bytes: PositiveInt
    file_depth: PositiveInt
    mcp_tools: tuple[str, ...]
    network_name: str
    diagnostics: DiagnosticCorpusConfig

"""Capsem settings schema -- Pydantic models as canonical source of truth.

Two node types:
  - GroupNode: container with children (kind="group")
  - SettingNode: everything else (kind="setting")
    - Regular settings: setting_type in (text, number, bool, kv_map, ...)
    - Actions: setting_type="action", metadata.action=ActionKind
    - MCP tools: setting_type="mcp_tool", metadata.origin=McpToolOrigin

MCP servers are GroupNodes containing server config settings and mcp_tool
SettingNodes. Tool categories (snapshots, network) are nested sub-groups.

JSON Schema is generated from SettingsRoot.model_json_schema().
"""

from __future__ import annotations

from enum import Enum
from typing import Annotated, Any, Literal

from pydantic import BaseModel, ConfigDict, Discriminator, Field, Tag


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class SettingType(str, Enum):
    """Data type of a setting (drives UI rendering)."""

    TEXT = "text"
    NUMBER = "number"
    URL = "url"
    EMAIL = "email"
    APIKEY = "apikey"
    BOOL = "bool"
    FILE = "file"
    KV_MAP = "kv_map"
    STRING_LIST = "string_list"
    INT_LIST = "int_list"
    FLOAT_LIST = "float_list"
    ACTION = "action"
    MCP_TOOL = "mcp_tool"


class Widget(str, Enum):
    """Explicit UI widget override."""

    TOGGLE = "toggle"
    TEXT_INPUT = "text_input"
    NUMBER_INPUT = "number_input"
    PASSWORD_INPUT = "password_input"
    SELECT = "select"
    FILE_EDITOR = "file_editor"
    DOMAIN_CHIPS = "domain_chips"
    STRING_CHIPS = "string_chips"
    SLIDER = "slider"
    KV_EDITOR = "kv_editor"


class SideEffect(str, Enum):
    """Frontend side effect triggered on value change."""

    TOGGLE_THEME = "toggle_theme"


class ActionKind(str, Enum):
    """Action identifier for action-type settings."""

    CHECK_UPDATE = "check_update"
    PRESET_SELECT = "preset_select"
    RERUN_WIZARD = "rerun_wizard"


class McpTransport(str, Enum):
    """MCP server transport protocol."""

    STDIO = "stdio"
    SSE = "sse"


class McpToolOrigin(str, Enum):
    """Where an MCP tool runs."""

    BUILTIN = "builtin"
    REMOTE = "remote"
    IN_VM = "in_vm"


class PolicySource(str, Enum):
    """Where a setting's effective value came from."""

    DEFAULT = "default"
    USER = "user"
    CORP = "corp"


# ---------------------------------------------------------------------------
# Value types
# ---------------------------------------------------------------------------


class FileValue(BaseModel):
    """A file to write to a guest path. Value variant for type=file settings."""

    model_config = ConfigDict(frozen=True)

    path: str
    content: str


# SettingValue is a loose union -- consumers check setting_type to interpret.
# Order matters for Pydantic union parsing (bool before int, etc.).
# dict[str, str] (kv_map) comes after FileValue (which is a BaseModel with
# specific fields) so Pydantic tries the structured type first.
SettingValue = (
    bool | int | float | FileValue | dict[str, str]
    | list[str] | list[int] | list[float] | str
)


# ---------------------------------------------------------------------------
# HTTP method permissions
# ---------------------------------------------------------------------------


class HttpMethodPermissions(BaseModel):
    """Per-rule HTTP method permissions."""

    model_config = ConfigDict(frozen=True)

    domains: list[str] = Field(default_factory=list)
    path: str | None = None
    get: bool = False
    post: bool = False
    put: bool = False
    delete: bool = False
    other: bool = False


# ---------------------------------------------------------------------------
# Setting metadata
# ---------------------------------------------------------------------------


class SettingMetadata(BaseModel):
    """Structured metadata for a setting.

    Contains fields for all setting types:
    - Common: domains, choices, min, max, rules, env_vars, mask, validator, etc.
    - Action-specific: action (ActionKind)
    - MCP tool-specific: origin (McpToolOrigin)
    - MCP server-specific (legacy): transport, command, url, args, env, headers
    """

    # -- Common fields (from Rust SettingMetadata) --
    domains: list[str] = Field(default_factory=list)
    choices: list[str] = Field(default_factory=list)
    min: int | None = None
    max: int | None = None
    rules: dict[str, HttpMethodPermissions] = Field(default_factory=dict)
    env_vars: list[str] = Field(default_factory=list)
    collapsed: bool = False
    format: str | None = None
    docs_url: str | None = None
    prefix: str | None = None
    filetype: str | None = None
    widget: Widget | None = None
    side_effect: SideEffect | None = None
    hidden: bool = False
    builtin: bool = False
    mask: bool = False
    validator: str | None = None

    # -- Action-specific --
    action: ActionKind | None = None

    # -- MCP tool-specific --
    origin: McpToolOrigin | None = None

    # -- MCP server-specific (legacy, kept for backward compat) --
    transport: McpTransport | None = None
    command: str | None = None
    url: str | None = None
    args: list[str] = Field(default_factory=list)
    env: dict[str, str] = Field(default_factory=dict)
    headers: dict[str, str] = Field(default_factory=dict)


# ---------------------------------------------------------------------------
# History
# ---------------------------------------------------------------------------


class HistoryEntry(BaseModel):
    """A single value change record for audit trail."""

    model_config = ConfigDict(frozen=True)

    timestamp: str
    value: Any
    source: PolicySource


# ---------------------------------------------------------------------------
# Node types
# ---------------------------------------------------------------------------


class SettingNode(BaseModel):
    """A setting node (kind="setting").

    Covers regular settings, actions, and MCP tools.
    Consumers check setting_type to know which fields are relevant.
    """

    kind: Literal["setting"] = "setting"

    # -- Required core --
    key: str
    name: str
    description: str
    setting_type: SettingType

    # -- Value fields (populated for regular settings) --
    default_value: Any = None
    effective_value: Any = None

    # -- Policy fields --
    source: PolicySource = PolicySource.DEFAULT
    modified: str | None = None
    corp_locked: bool = False
    enabled_by: str | None = None
    enabled: bool = True
    collapsed: bool = False

    # -- Metadata (always present, defaults to empty) --
    metadata: SettingMetadata = Field(default_factory=SettingMetadata)

    # -- Value history (audit trail, populated at runtime) --
    history: list[HistoryEntry] = Field(default_factory=list)


class GroupNode(BaseModel):
    """A group node (kind="group"). Container with children."""

    kind: Literal["group"] = "group"

    key: str
    name: str
    description: str | None = None
    enabled_by: str | None = None
    enabled: bool = True
    collapsed: bool
    children: list[SettingsNode]


# Discriminated union on "kind" field
SettingsNode = Annotated[
    Annotated[GroupNode, Tag("group")] | Annotated[SettingNode, Tag("setting")],
    Discriminator("kind"),
]

# Update forward refs now that SettingsNode is defined
GroupNode.model_rebuild()


# ---------------------------------------------------------------------------
# Root model
# ---------------------------------------------------------------------------


class SettingsRoot(BaseModel):
    """Top-level settings document."""

    settings: list[SettingsNode]


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def validate_settings(json_str: str) -> SettingsRoot:
    """Parse and validate a JSON string as a SettingsRoot."""
    return SettingsRoot.model_validate_json(json_str)


def to_json(root: SettingsRoot) -> str:
    """Serialize a SettingsRoot to JSON."""
    return root.model_dump_json(indent=2)


def export_json_schema() -> dict:
    """Generate JSON Schema from the Pydantic models."""
    return SettingsRoot.model_json_schema()


def extract_settings(nodes: list[SettingsNode]) -> list[SettingNode]:
    """Recursively walk the tree and collect all SettingNode instances."""
    result: list[SettingNode] = []
    for node in nodes:
        if isinstance(node, SettingNode):
            result.append(node)
        elif isinstance(node, GroupNode):
            result.extend(extract_settings(node.children))
    return result


def count_by_type(nodes: list[SettingsNode]) -> dict[str, int]:
    """Count settings by setting_type across the entire tree."""
    settings = extract_settings(nodes)
    counts: dict[str, int] = {}
    for s in settings:
        key = s.setting_type.value
        counts[key] = counts.get(key, 0) + 1
    return counts

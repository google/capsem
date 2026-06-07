"""Tests for the Capsem settings schema (Pydantic models).

TDD: these tests define the expected behavior of the schema module.
Covers enums, value variants, node types, roundtrips, metadata,
and golden fixture conformance.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from capsem.builder.schema import (
    ActionKind,
    FileValue,
    GroupNode,
    HistoryEntry,
    HttpMethodPermissions,
    McpToolOrigin,
    McpTransport,
    PolicySource,
    SettingMetadata,
    SettingNode,
    SettingType,
    SettingsRoot,
    SideEffect,
    Widget,
    count_by_type,
    export_json_schema,
    extract_settings,
    to_json,
    validate_settings,
)

FIXTURE_DIR = Path(__file__).parent / "settings_spec"


# ---------------------------------------------------------------------------
# Enum value tests
# ---------------------------------------------------------------------------


class TestSettingType:
    """SettingType enum has 13 values (11 value types + action + mcp_tool)."""

    EXPECTED_VALUES = [
        "text",
        "number",
        "url",
        "email",
        "apikey",
        "bool",
        "file",
        "kv_map",
        "string_list",
        "int_list",
        "float_list",
        "action",
        "mcp_tool",
    ]

    def test_all_values_present(self):
        actual = sorted(e.value for e in SettingType)
        assert sorted(self.EXPECTED_VALUES) == actual

    @pytest.mark.parametrize("value", EXPECTED_VALUES)
    def test_value_roundtrip(self, value):
        assert SettingType(value).value == value


class TestWidget:
    EXPECTED = [
        "toggle",
        "text_input",
        "number_input",
        "password_input",
        "select",
        "file_editor",
        "domain_chips",
        "string_chips",
        "slider",
        "kv_editor",
    ]

    def test_all_values_present(self):
        actual = sorted(e.value for e in Widget)
        assert sorted(self.EXPECTED) == actual


class TestSideEffect:
    def test_toggle_theme(self):
        assert SideEffect.TOGGLE_THEME.value == "toggle_theme"

    def test_count(self):
        assert len(SideEffect) == 1


class TestActionKind:
    EXPECTED = ["check_update", "preset_select", "rerun_wizard"]

    def test_all_values_present(self):
        actual = sorted(e.value for e in ActionKind)
        assert sorted(self.EXPECTED) == actual


class TestMcpTransport:
    def test_stdio(self):
        assert McpTransport.STDIO.value == "stdio"

    def test_sse(self):
        assert McpTransport.SSE.value == "sse"

    def test_count(self):
        assert len(McpTransport) == 2


class TestPolicySource:
    EXPECTED = ["default", "user", "corp"]

    def test_all_values_present(self):
        actual = sorted(e.value for e in PolicySource)
        assert sorted(self.EXPECTED) == actual


class TestMcpToolOrigin:
    EXPECTED = ["builtin", "remote", "in_vm"]

    def test_all_values_present(self):
        actual = sorted(e.value for e in McpToolOrigin)
        assert sorted(self.EXPECTED) == actual

    def test_count(self):
        assert len(McpToolOrigin) == 3


# ---------------------------------------------------------------------------
# SettingValue variant tests
# ---------------------------------------------------------------------------


class TestFileValue:
    def test_construction(self):
        fv = FileValue(path="/root/.bashrc", content="PS1='$ '")
        assert fv.path == "/root/.bashrc"
        assert fv.content == "PS1='$ '"

    def test_roundtrip(self):
        fv = FileValue(path="/etc/conf", content="key=val")
        data = fv.model_dump()
        assert data == {"path": "/etc/conf", "content": "key=val"}
        fv2 = FileValue.model_validate(data)
        assert fv == fv2


class TestHistoryEntry:
    def test_construction(self):
        h = HistoryEntry(
            timestamp="2025-06-01T12:00:00Z",
            value="test",
            source=PolicySource.USER,
        )
        assert h.timestamp == "2025-06-01T12:00:00Z"
        assert h.value == "test"
        assert h.source == PolicySource.USER

    def test_frozen(self):
        h = HistoryEntry(
            timestamp="2025-06-01T12:00:00Z",
            value=42,
            source=PolicySource.DEFAULT,
        )
        with pytest.raises(Exception):
            h.timestamp = "changed"

    def test_roundtrip(self):
        h = HistoryEntry(
            timestamp="2025-06-01T12:00:00Z",
            value={"key": "val"},
            source=PolicySource.CORP,
        )
        data = h.model_dump()
        h2 = HistoryEntry.model_validate(data)
        assert h == h2


# ---------------------------------------------------------------------------
# HttpMethodPermissions tests
# ---------------------------------------------------------------------------


class TestHttpMethodPermissions:
    def test_defaults(self):
        perms = HttpMethodPermissions()
        assert perms.domains == []
        assert perms.path is None
        assert perms.get is False
        assert perms.post is False
        assert perms.put is False
        assert perms.delete is False
        assert perms.other is False

    def test_full_construction(self):
        perms = HttpMethodPermissions(
            domains=["api.example.com"],
            path="/v1/*",
            get=True,
            post=True,
            put=False,
            delete=False,
            other=False,
        )
        assert perms.domains == ["api.example.com"]
        assert perms.path == "/v1/*"
        assert perms.get is True
        assert perms.post is True

    def test_roundtrip(self):
        perms = HttpMethodPermissions(get=True, post=True)
        data = perms.model_dump()
        perms2 = HttpMethodPermissions.model_validate(data)
        assert perms == perms2


# ---------------------------------------------------------------------------
# SettingMetadata tests
# ---------------------------------------------------------------------------


class TestSettingMetadata:
    def test_defaults(self):
        meta = SettingMetadata()
        assert meta.domains == []
        assert meta.choices == []
        assert meta.min is None
        assert meta.max is None
        assert meta.rules == {}
        assert meta.env_vars == []
        assert meta.collapsed is False
        assert meta.format is None
        assert meta.docs_url is None
        assert meta.prefix is None
        assert meta.filetype is None
        assert meta.widget is None
        assert meta.side_effect is None
        assert meta.hidden is False
        assert meta.builtin is False
        assert meta.mask is False
        assert meta.validator is None
        # Action-specific
        assert meta.action is None
        # MCP tool-specific
        assert meta.origin is None
        # MCP server-specific (legacy)
        assert meta.transport is None
        assert meta.command is None
        assert meta.url is None
        assert meta.args == []
        assert meta.env == {}
        assert meta.headers == {}

    def test_all_fields_settable(self):
        meta = SettingMetadata(
            domains=["*.example.com"],
            choices=["a", "b"],
            min=0,
            max=100,
            rules={"default": HttpMethodPermissions(get=True)},
            env_vars=["MY_VAR"],
            collapsed=True,
            format="domain_list",
            docs_url="https://docs.example.com",
            prefix="sk-",
            filetype="json",
            widget=Widget.SLIDER,
            side_effect=SideEffect.TOGGLE_THEME,
            hidden=True,
            builtin=True,
            mask=True,
            validator="^[a-z]+$",
            action=ActionKind.CHECK_UPDATE,
            origin=McpToolOrigin.REMOTE,
            transport=McpTransport.STDIO,
            command="/usr/bin/server",
            url="https://mcp.example.com",
            args=["--verbose"],
            env={"DEBUG": "1"},
            headers={"Authorization": "Bearer token"},
        )
        assert meta.domains == ["*.example.com"]
        assert meta.min == 0
        assert meta.max == 100
        assert meta.widget == Widget.SLIDER
        assert meta.mask is True
        assert meta.validator == "^[a-z]+$"
        assert meta.action == ActionKind.CHECK_UPDATE
        assert meta.origin == McpToolOrigin.REMOTE
        assert meta.transport == McpTransport.STDIO

    def test_roundtrip(self):
        meta = SettingMetadata(
            domains=["x.com"],
            env_vars=["KEY"],
            widget=Widget.TOGGLE,
            action=ActionKind.PRESET_SELECT,
        )
        data = meta.model_dump()
        meta2 = SettingMetadata.model_validate(data)
        assert meta == meta2


# ---------------------------------------------------------------------------
# Node tests
# ---------------------------------------------------------------------------


class TestGroupNode:
    def test_kind_is_group(self):
        g = GroupNode(key="app", name="App", collapsed=False, children=[])
        assert g.kind == "group"

    def test_required_fields(self):
        g = GroupNode(key="app", name="App", collapsed=False, children=[])
        assert g.key == "app"
        assert g.name == "App"
        assert g.collapsed is False
        assert g.children == []

    def test_optional_fields_default(self):
        g = GroupNode(key="app", name="App", collapsed=False, children=[])
        assert g.description is None
        assert g.enabled_by is None
        assert g.enabled is True

    def test_optional_fields_set(self):
        g = GroupNode(
            key="provider",
            name="Provider",
            description="A provider group",
            enabled_by="provider.allow",
            enabled=False,
            collapsed=True,
            children=[],
        )
        assert g.description == "A provider group"
        assert g.enabled_by == "provider.allow"
        assert g.enabled is False
        assert g.collapsed is True

    def test_nested_children(self):
        child = SettingNode(
            key="app.toggle",
            name="Toggle",
            description="A toggle",
            setting_type=SettingType.BOOL,
            default_value=True,
        )
        g = GroupNode(
            key="app", name="App", collapsed=False, children=[child]
        )
        assert len(g.children) == 1

    def test_serialization_has_kind(self):
        g = GroupNode(key="app", name="App", collapsed=False, children=[])
        data = g.model_dump()
        assert data["kind"] == "group"


class TestSettingNode:
    def test_kind_is_setting(self):
        s = SettingNode(
            key="app.toggle",
            name="Toggle",
            description="desc",
            setting_type=SettingType.BOOL,
        )
        assert s.kind == "setting"

    def test_required_core(self):
        s = SettingNode(
            key="app.name",
            name="Name",
            description="desc",
            setting_type=SettingType.TEXT,
        )
        assert s.key == "app.name"
        assert s.name == "Name"
        assert s.setting_type == SettingType.TEXT

    def test_optional_defaults(self):
        s = SettingNode(
            key="x",
            name="X",
            description="d",
            setting_type=SettingType.TEXT,
        )
        assert s.default_value is None
        assert s.effective_value is None
        assert s.source == PolicySource.DEFAULT
        assert s.modified is None
        assert s.corp_locked is False
        assert s.enabled_by is None
        assert s.enabled is True
        assert s.collapsed is False
        assert s.metadata is not None

    def test_regular_setting_with_values(self):
        s = SettingNode(
            key="app.cpu",
            name="CPU Count",
            description="Number of CPUs",
            setting_type=SettingType.NUMBER,
            default_value=4,
            effective_value=8,
            source=PolicySource.USER,
            modified="2025-01-01T00:00:00Z",
            corp_locked=False,
            enabled=True,
            metadata=SettingMetadata(min=1, max=16),
        )
        assert s.default_value == 4
        assert s.effective_value == 8
        assert s.source == PolicySource.USER

    def test_action_setting(self):
        s = SettingNode(
            key="app.check_update",
            name="Check for updates",
            description="Manually check",
            setting_type=SettingType.ACTION,
            metadata=SettingMetadata(action=ActionKind.CHECK_UPDATE),
        )
        assert s.setting_type == SettingType.ACTION
        assert s.metadata.action == ActionKind.CHECK_UPDATE
        assert s.default_value is None

    def test_mcp_tool_setting(self):
        s = SettingNode(
            key="mcp.capsem.tools.snapshot_create",
            name="snapshot_create",
            description="Create a VM snapshot",
            setting_type=SettingType.MCP_TOOL,
            metadata=SettingMetadata(
                origin=McpToolOrigin.BUILTIN,
                builtin=True,
            ),
        )
        assert s.setting_type == SettingType.MCP_TOOL
        assert s.metadata.origin == McpToolOrigin.BUILTIN
        assert s.metadata.builtin is True

    def test_kv_map_setting(self):
        s = SettingNode(
            key="test.env",
            name="Environment",
            description="Key-value pairs",
            setting_type=SettingType.KV_MAP,
            default_value={"KEY": "value"},
            effective_value={"KEY": "value"},
            metadata=SettingMetadata(widget=Widget.KV_EDITOR),
        )
        assert s.setting_type == SettingType.KV_MAP
        assert s.default_value == {"KEY": "value"}
        assert s.metadata.widget == Widget.KV_EDITOR

    def test_history_default_empty(self):
        s = SettingNode(
            key="x", name="X", description="d",
            setting_type=SettingType.TEXT,
        )
        assert s.history == []

    def test_history_with_entries(self):
        entries = [
            HistoryEntry(
                timestamp="2025-06-01T12:00:00Z",
                value="old",
                source=PolicySource.DEFAULT,
            ),
            HistoryEntry(
                timestamp="2025-06-02T12:00:00Z",
                value="new",
                source=PolicySource.USER,
            ),
        ]
        s = SettingNode(
            key="x", name="X", description="d",
            setting_type=SettingType.TEXT,
            history=entries,
        )
        assert len(s.history) == 2
        assert s.history[0].value == "old"
        assert s.history[1].source == PolicySource.USER

    def test_file_setting_value(self):
        fv = FileValue(path="/root/.bashrc", content="PS1='$ '")
        s = SettingNode(
            key="vm.bashrc",
            name="Bashrc",
            description="Shell config",
            setting_type=SettingType.FILE,
            default_value=fv.model_dump(),
        )
        assert s.default_value == {"path": "/root/.bashrc", "content": "PS1='$ '"}

    def test_serialization_has_kind(self):
        s = SettingNode(
            key="x",
            name="X",
            description="d",
            setting_type=SettingType.BOOL,
        )
        data = s.model_dump()
        assert data["kind"] == "setting"


# ---------------------------------------------------------------------------
# SettingsRoot + roundtrip tests
# ---------------------------------------------------------------------------


class TestSettingsRoot:
    def test_empty(self):
        root = SettingsRoot(settings=[])
        assert root.settings == []

    def test_with_group_and_settings(self):
        setting = SettingNode(
            key="app.toggle",
            name="Toggle",
            description="A toggle",
            setting_type=SettingType.BOOL,
            default_value=True,
        )
        group = GroupNode(
            key="app",
            name="App",
            collapsed=False,
            children=[setting],
        )
        root = SettingsRoot(settings=[group])
        assert len(root.settings) == 1

    def test_roundtrip_json(self):
        setting = SettingNode(
            key="app.name",
            name="App Name",
            description="The name",
            setting_type=SettingType.TEXT,
            default_value="Capsem",
            effective_value="Capsem",
        )
        action = SettingNode(
            key="app.check",
            name="Check",
            description="Check now",
            setting_type=SettingType.ACTION,
            metadata=SettingMetadata(action=ActionKind.CHECK_UPDATE),
        )
        mcp = SettingNode(
            key="mcp.test.tools.test_tool",
            name="test_tool",
            description="A test MCP tool",
            setting_type=SettingType.MCP_TOOL,
            metadata=SettingMetadata(
                origin=McpToolOrigin.BUILTIN,
                builtin=True,
            ),
        )
        group = GroupNode(
            key="app",
            name="App",
            collapsed=False,
            children=[setting, action],
        )
        mcp_group = GroupNode(
            key="mcp",
            name="MCP",
            collapsed=False,
            children=[mcp],
        )
        root = SettingsRoot(settings=[group, mcp_group])
        json_str = to_json(root)
        root2 = validate_settings(json_str)
        assert len(root2.settings) == 2

    def test_nested_groups_roundtrip(self):
        inner = SettingNode(
            key="a.b.toggle",
            name="Toggle",
            description="Inner toggle",
            setting_type=SettingType.BOOL,
            default_value=True,
        )
        inner_group = GroupNode(
            key="a.b",
            name="B",
            collapsed=False,
            children=[inner],
        )
        outer_group = GroupNode(
            key="a",
            name="A",
            collapsed=False,
            children=[inner_group],
        )
        root = SettingsRoot(settings=[outer_group])
        json_str = to_json(root)
        root2 = validate_settings(json_str)
        # Walk to inner setting
        outer = root2.settings[0]
        assert outer.kind == "group"
        inner_g = outer.children[0]
        assert inner_g.kind == "group"
        leaf = inner_g.children[0]
        assert leaf.kind == "setting"
        assert leaf.key == "a.b.toggle"


# ---------------------------------------------------------------------------
# Schema generation tests
# ---------------------------------------------------------------------------


class TestJsonSchema:
    def test_export_returns_dict(self):
        schema = export_json_schema()
        assert isinstance(schema, dict)

    def test_has_defs(self):
        schema = export_json_schema()
        assert "$defs" in schema

    def test_has_properties(self):
        schema = export_json_schema()
        assert "properties" in schema

    def test_is_valid_json(self):
        schema = export_json_schema()
        json_str = json.dumps(schema)
        parsed = json.loads(json_str)
        assert parsed == schema

    def test_settings_property_exists(self):
        schema = export_json_schema()
        assert "settings" in schema["properties"]


# ---------------------------------------------------------------------------
# Golden fixture conformance tests
# ---------------------------------------------------------------------------


def _load_golden() -> SettingsRoot:
    return validate_settings((FIXTURE_DIR / "golden.json").read_text())


def _load_expected() -> dict:
    return json.loads((FIXTURE_DIR / "expected.json").read_text())


def _count_groups(nodes) -> int:
    """Recursively count group nodes."""
    count = 0
    for node in nodes:
        if isinstance(node, GroupNode):
            count += 1
            count += _count_groups(node.children)
    return count


class TestGoldenFixture:
    """Comprehensive conformance tests against golden.json."""

    def test_golden_fixture_parses(self):
        root = _load_golden()
        assert len(root.settings) > 0

    def test_golden_total_settings(self):
        root = _load_golden()
        expected = _load_expected()
        settings = extract_settings(root.settings)
        assert len(settings) == expected["total_settings"]

    def test_golden_settings_by_type(self):
        root = _load_golden()
        expected = _load_expected()
        counts = count_by_type(root.settings)
        assert counts == expected["by_type"]

    def test_golden_group_count(self):
        root = _load_golden()
        expected = _load_expected()
        assert _count_groups(root.settings) == expected["group_count"]

    def test_golden_setting_fields(self):
        root = _load_golden()
        expected = _load_expected()
        settings = extract_settings(root.settings)
        settings_by_key = {s.key: s for s in settings}
        for exp in expected["settings"]:
            actual = settings_by_key.get(exp["key"])
            assert actual is not None, f"Missing setting: {exp['key']}"
            assert actual.name == exp["name"], f"Name mismatch for {exp['key']}"
            assert actual.setting_type.value == exp["setting_type"], (
                f"Type mismatch for {exp['key']}"
            )
            assert (actual.enabled_by or None) == exp["enabled_by"], (
                f"enabled_by mismatch for {exp['key']}"
            )

    def test_all_setting_types_present(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        present = {s.setting_type for s in settings}
        for st in SettingType:
            assert st in present, f"Missing setting_type: {st.value}"

    def test_all_metadata_fields_exercised(self):
        """Key SettingMetadata fields are non-default in at least one setting."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        defaults = SettingMetadata()

        fields_to_check = [
            "domains", "choices", "min", "max", "rules", "env_vars",
            "format", "docs_url", "prefix", "filetype", "widget",
            "side_effect", "hidden", "builtin", "mask", "validator",
            "action", "origin",
        ]
        exercised = set()
        for s in settings:
            for field in fields_to_check:
                val = getattr(s.metadata, field)
                default_val = getattr(defaults, field)
                if val != default_val:
                    exercised.add(field)

        missing = set(fields_to_check) - exercised
        assert not missing, f"Metadata fields never exercised: {missing}"

    def test_action_settings_have_action_kind(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        actions = [s for s in settings if s.setting_type == SettingType.ACTION]
        assert len(actions) >= 1
        for a in actions:
            assert a.metadata.action is not None, (
                f"Action {a.key} missing metadata.action"
            )

    def test_mcp_tool_settings_have_origin(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        tools = [s for s in settings if s.setting_type == SettingType.MCP_TOOL]
        assert len(tools) >= 1
        for t in tools:
            assert t.metadata.origin is not None, (
                f"MCP tool {t.key} missing metadata.origin"
            )

    def test_mask_field_exercised(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        masked = [s for s in settings if s.metadata.mask]
        assert len(masked) >= 1

    def test_kv_map_setting_has_dict_value(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        kvs = [s for s in settings if s.setting_type == SettingType.KV_MAP]
        assert len(kvs) >= 1
        for k in kvs:
            assert isinstance(k.default_value, dict), (
                f"kv_map setting {k.key} default_value should be a dict"
            )

    def test_validator_field_exercised(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        with_validator = [s for s in settings if s.metadata.validator]
        assert len(with_validator) >= 1

    def test_mcp_server_is_group_with_tools(self):
        """MCP server 'capsem' is a GroupNode containing a tools sub-group."""
        root = _load_golden()
        mcp_group = None
        for node in root.settings:
            if isinstance(node, GroupNode) and node.key == "mcp":
                mcp_group = node
                break
        assert mcp_group is not None
        capsem_group = None
        for child in mcp_group.children:
            if isinstance(child, GroupNode) and child.key == "mcp.capsem":
                capsem_group = child
                break
        assert capsem_group is not None
        tools_group = None
        for child in capsem_group.children:
            if isinstance(child, GroupNode) and child.key == "mcp.capsem.tools":
                tools_group = child
                break
        assert tools_group is not None
        assert len(tools_group.children) >= 1

    def test_enabled_by_chain(self):
        """The provider group's enabled_by points to a valid bool setting."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        settings_by_key = {s.key: s for s in settings}
        # Find settings with enabled_by
        with_parent = [s for s in settings if s.enabled_by]
        assert len(with_parent) >= 1
        for s in with_parent:
            parent = settings_by_key.get(s.enabled_by)
            assert parent is not None, (
                f"{s.key} has enabled_by={s.enabled_by} but that setting doesn't exist"
            )
            assert parent.setting_type == SettingType.BOOL, (
                f"{s.key}'s enabled_by target {s.enabled_by} is not a bool"
            )

    def test_file_setting_has_path_content(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        files = [s for s in settings if s.setting_type == SettingType.FILE]
        assert len(files) >= 1
        for f in files:
            assert isinstance(f.default_value, dict), (
                f"File setting {f.key} default_value should be a dict"
            )
            assert "path" in f.default_value
            assert "content" in f.default_value

    def test_hidden_setting_exists(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        hidden = [s for s in settings if s.metadata.hidden]
        assert len(hidden) >= 1

    def test_builtin_setting_exists(self):
        root = _load_golden()
        settings = extract_settings(root.settings)
        builtins = [s for s in settings if s.metadata.builtin]
        assert len(builtins) >= 1

    def test_nested_group_depth(self):
        """test_ai.provider is nested 2 levels deep."""
        root = _load_golden()
        # Find test_ai group
        ai_group = None
        for node in root.settings:
            if isinstance(node, GroupNode) and node.key == "test_ai":
                ai_group = node
                break
        assert ai_group is not None
        # Find provider group inside
        provider = None
        for child in ai_group.children:
            if isinstance(child, GroupNode) and child.key == "test_ai.provider":
                provider = child
                break
        assert provider is not None
        assert len(provider.children) >= 1

    def test_roundtrip_golden(self):
        """Parse golden -> serialize -> parse again -> identical structure."""
        root1 = _load_golden()
        json_str = to_json(root1)
        root2 = validate_settings(json_str)
        settings1 = extract_settings(root1.settings)
        settings2 = extract_settings(root2.settings)
        assert len(settings1) == len(settings2)
        for s1, s2 in zip(settings1, settings2):
            assert s1.key == s2.key
            assert s1.setting_type == s2.setting_type
            assert s1.name == s2.name

    def test_user_modified_setting(self):
        """test_appearance.theme has source=user and modified timestamp."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        theme = next(s for s in settings if s.key == "test_appearance.theme")
        assert theme.source == PolicySource.USER
        assert theme.modified is not None
        assert theme.default_value != theme.effective_value

    def test_collapsed_group_exists(self):
        """At least one group has collapsed=true."""
        root = _load_golden()

        def find_collapsed(nodes):
            for node in nodes:
                if isinstance(node, GroupNode):
                    if node.collapsed:
                        return True
                    if find_collapsed(node.children):
                        return True
            return False

        assert find_collapsed(root.settings)

    def test_collapsed_setting_exists(self):
        """At least one setting has collapsed=true."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        collapsed = [s for s in settings if s.collapsed]
        assert len(collapsed) >= 1

    def test_choices_field_exercised(self):
        """At least one setting has non-empty choices."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        with_choices = [s for s in settings if s.metadata.choices]
        assert len(with_choices) >= 1

    def test_select_widget_with_choices(self):
        """A setting with widget=select also has choices."""
        root = _load_golden()
        settings = extract_settings(root.settings)
        selects = [s for s in settings if s.metadata.widget == Widget.SELECT]
        assert len(selects) >= 1
        for s in selects:
            assert len(s.metadata.choices) >= 2, (
                f"Select widget {s.key} needs choices"
            )

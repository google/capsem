"""Compose parsing from evaluator-supplied values, without host discovery.

Field/interpolation provenance: Pierre Tholoniat, bb61fc82d44bc42c978c4acf5435a1975600c75e.
These inputs do not authorize engine execution or host access. Controllers
must obtain explicit grants before supplying environment, file or directory data.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from pathlib import Path, PurePath
from typing import Any

import yaml

from .compose_inputs import (
    DEFAULT_COMPOSE_LIMITS,
    EMPTY_INPUTS,
    InterpolationBudget,
    build_host_compose_inputs,
)
from .compose_inputs import ComposeInputs as ComposeInputs
from .compose_inputs import ComposeLimits as ComposeLimits
from .compose_interpolation import _interpolate_compose_str, _load_dotenv
from .compose_service import _COMPOSE_PASSTHROUGH_FIELDS as _COMPOSE_PASSTHROUGH_FIELDS
from .compose_service import _extract_service_fields as _extract_service_fields
from .compose_service import extract_compose_fields as extract_compose_fields
from .compose_values import _is_bind_mount_source as _is_bind_mount_source
from .compose_values import _normalize_ports as _normalize_ports
from .compose_values import _normalize_volumes as _normalize_volumes


class _BoundedSafeLoader(yaml.SafeLoader):
    def __init__(self, stream: str, limits: ComposeLimits):
        super().__init__(stream)
        self.mapping_sizes: dict[int, int] = {}
        self.counted_mappings: set[int] = set()
        self.remaining_mapping_pairs = limits.maximum_nodes
        self.maximum_mapping_pairs = limits.maximum_nodes

    def _merge_size(self, node: Any, active: set[int]) -> int:
        identity = id(node)
        if identity in active:
            raise ValueError("Compose YAML contains a cyclic merge")
        if identity in self.mapping_sizes:
            return self.mapping_sizes[identity]
        active.add(identity)
        size = 0
        for key, value in node.value:
            if key.tag != "tag:yaml.org,2002:merge":
                size += 1
            elif isinstance(value, yaml.nodes.MappingNode):
                size += self._merge_size(value, active)
            elif isinstance(value, yaml.nodes.SequenceNode):
                for child in value.value:
                    if not isinstance(child, yaml.nodes.MappingNode):
                        raise ValueError("Invalid Compose YAML merge")
                    size += self._merge_size(child, active)
                    if size > self.maximum_mapping_pairs:
                        raise ValueError("Compose expanded node limit exceeded")
            else:
                raise ValueError("Invalid Compose YAML merge")
            if size > self.maximum_mapping_pairs:
                raise ValueError("Compose expanded node limit exceeded")
        active.remove(identity)
        self.mapping_sizes[identity] = size
        return size

    def flatten_mapping(self, node: Any) -> None:
        identity = id(node)
        if identity not in self.counted_mappings:
            self.remaining_mapping_pairs -= self._merge_size(node, set())
            if self.remaining_mapping_pairs < 0:
                raise ValueError("Compose expanded node limit exceeded")
            self.counted_mappings.add(identity)
        # Preserve SafeLoader's merge precedence after proving that its
        # intermediate pair lists fit, before any recursive list expansion.
        super().flatten_mapping(node)


def _interpolate_compose_tree(
    node: Any,
    environment: dict[str, str],
    budget: InterpolationBudget,
    depth: int = 0,
    active: set[int] | None = None,
) -> Any:
    budget.node(depth)
    if isinstance(node, str):
        return _interpolate_compose_str(node, environment, budget)
    if not isinstance(node, dict | list | tuple):
        return node
    if active is None:
        active = set()
    identity = id(node)
    if identity in active:
        raise ValueError("Compose YAML contains a cyclic alias")
    active.add(identity)
    try:
        if isinstance(node, dict):
            return {
                key: _interpolate_compose_tree(value, environment, budget, depth + 1, active)
                for key, value in node.items()
            }
        result = [
            _interpolate_compose_tree(value, environment, budget, depth + 1, active)
            for value in node
        ]
        return tuple(result) if isinstance(node, tuple) else result
    finally:
        active.remove(identity)


def _bounded_yaml(text: str, limits: ComposeLimits) -> Any:
    # Stream events before constructing objects. Aliases count as input nodes;
    # expanded alias visits are independently bounded during interpolation.
    depth = nodes = 0
    try:
        for event in yaml.parse(text, Loader=yaml.SafeLoader):
            if isinstance(event, yaml.events.MappingStartEvent | yaml.events.SequenceStartEvent):
                depth += 1
                if depth > limits.maximum_depth:
                    raise ValueError("Compose depth limit exceeded")
            if isinstance(event, yaml.events.MappingEndEvent | yaml.events.SequenceEndEvent):
                depth -= 1
            if isinstance(
                event,
                yaml.events.MappingStartEvent
                | yaml.events.SequenceStartEvent
                | yaml.events.ScalarEvent
                | yaml.events.AliasEvent,
            ):
                nodes += 1
                if nodes > limits.maximum_nodes:
                    raise ValueError("Compose node limit exceeded")
        loader = _BoundedSafeLoader(text, limits)
        try:
            return loader.get_single_data()
        finally:
            loader.dispose()
    except (yaml.YAMLError, RecursionError):
        raise ValueError("Invalid Compose YAML") from None


def parse_compose_yaml(
    text: str,
    *,
    limits: ComposeLimits = DEFAULT_COMPOSE_LIMITS,
    inputs: ComposeInputs = EMPTY_INPUTS,
    dotenv: str = "",
) -> dict[str, Any]:
    if len(text.encode("utf-8")) + len(dotenv.encode("utf-8")) > limits.maximum_bytes:
        raise ValueError("Compose input byte limit exceeded")
    budget = InterpolationBudget(limits)
    dotenv_variables = _load_dotenv(dotenv, inputs.environment, budget)
    budget.blocked_environment = inputs.blocked_environment - set(dotenv_variables)
    budget.warned_blocked = inputs.warned_blocked
    raw = _bounded_yaml(text, limits)
    if not isinstance(raw, dict):
        return {}
    try:
        return _interpolate_compose_tree(raw, {**dotenv_variables, **inputs.environment}, budget)
    except RecursionError:
        raise ValueError("Compose depth limit exceeded") from None


def parse_compose_yaml_file(
    compose_path: PurePath,
    *,
    inputs: ComposeInputs,
    limits: ComposeLimits = DEFAULT_COMPOSE_LIMITS,
) -> dict[str, Any]:
    """Parse already-supplied file text; never opens or probes its pathname."""
    text = inputs.files.get(inputs.resolve(compose_path))
    if text is None:
        raise PermissionError("Compose file text was not supplied by the evaluator")
    dotenv = inputs.files.get(inputs.resolve(compose_path.parent / ".env"), "")
    return parse_compose_yaml(text, inputs=inputs, dotenv=dotenv, limits=limits)


def parse_host_compose_yaml_file(
    compose_path: Path,
    *,
    allowed_host_env: Sequence[str] = (),
    sample_metadata: Mapping[str, Any] | None = None,
    limits: ComposeLimits = DEFAULT_COMPOSE_LIMITS,
) -> dict[str, Any]:
    """Parse `compose_path` from host disk with bounded YAML, `.env`, and allowlisted host env."""
    inputs = build_host_compose_inputs(
        compose_path,
        allowed_host_env=allowed_host_env,
        sample_metadata=sample_metadata,
        limits=limits,
    )
    return parse_compose_yaml_file(compose_path, inputs=inputs, limits=limits)

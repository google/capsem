"""Inspect task metadata must fit the service's UTF-8 label contract."""

from __future__ import annotations

import pytest
from capsem_integrations._controller import truncate_label_value
from inspect_capsem import CapsemSandboxConfig
from inspect_capsem.sandbox import _sample_vm_labels, _start_sample_vm

from .helpers import Scripted


@pytest.mark.parametrize("task", ["é" * 200, "🙂" * 100, "ascii" * 100])
@pytest.mark.parametrize("stage", ["helper", "sample"])
async def test_task_labels_fit_the_byte_limit_without_splitting_unicode(
    task: str, stage: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CAPSEM_VM_PREFIX", "🙂" * 100)
    if stage == "helper":
        labels = _sample_vm_labels(task_name=task)
        assert labels is not None
    else:
        controller = Scripted()
        await _start_sample_vm(controller, CapsemSandboxConfig(), task_name=task)
        labels = controller.started[0]["labels"]
    value = labels["inspect-capsem-task"]
    assert value == truncate_label_value(task)
    assert len(value.encode("utf-8")) <= 255
    assert task.startswith(value)
    assert len((value + task[len(value)]).encode("utf-8")) > 255
    prefix = labels["capsem-vm-prefix"]
    assert len(prefix.encode("utf-8")) <= 255
    monkeypatch.delenv("CAPSEM_VM_PREFIX", raising=False)
    assert _sample_vm_labels(None) is None

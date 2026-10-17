"""Inspect task metadata must fit the service's UTF-8 label contract."""

import pytest
from inspect_capsem import CapsemSandboxConfig
from inspect_capsem._controller import _managed_vm_labels
from inspect_capsem._lifecycle import _init_sample_vm

from .helpers import Scripted


@pytest.mark.parametrize("task", ["é" * 200, "🙂" * 100, "ascii" * 100])
@pytest.mark.parametrize("stage", ["controller", "sample"])
async def test_task_labels_fit_the_byte_limit_without_splitting_unicode(task: str, stage: str) -> None:
    if stage == "controller":
        labels = _managed_vm_labels(task_name=task)
    else:
        controller = Scripted()
        await _init_sample_vm(controller, CapsemSandboxConfig(), task)
        labels = controller.started[0]["labels"]
    value = labels["inspect-capsem-task"]
    assert len(value.encode("utf-8")) <= 255
    assert task.startswith(value)
    assert len((value + task[len(value)]).encode("utf-8")) > 255

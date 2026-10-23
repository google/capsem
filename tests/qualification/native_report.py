"""Opt-in structured actual pytest outcomes for native CI artifact handoff."""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

from tests.fixtures.oci import native_candidates


class NativeReport:
    def __init__(self, config, destination: Path):
        self.config = config
        self.destination = destination
        self.started = time.time()
        self.cases = {}
        self.groups = {}

    def pytest_collection_finish(self, session):
        self.cases = {item.nodeid: {} for item in session.items}
        for item in session.items:
            marker = item.get_closest_marker("capability")
            group = marker.args[0] if marker else "mandatory"
            self.groups[group] = self.groups.get(group, 0) + 1

    def pytest_runtest_logreport(self, report):
        if report.nodeid in self.cases:
            self.cases[report.nodeid][report.when] = report.outcome

    @pytest.hookimpl(trylast=True)
    def pytest_sessionfinish(self, session, exitstatus):
        from tests.qualification.candidate import resolve

        candidate = resolve()
        settings = self.config.functional.native_images
        expected = candidate.layout / settings.report_name
        if self.destination != expected or self.destination.is_symlink():
            raise ValueError("native report destination is not selected candidate")
        if native_candidates.host_platform() != settings.platform:
            raise ValueError("native report requires the configured native Linux host")
        native_candidates.contained(candidate.layout, self.destination)
        value = {
            "source": native_candidates.source(self.config.root).model_dump(),
            "platform": settings.platform,
            "image": candidate.name,
            "subject": candidate.digest,
            "capabilities": sorted(candidate.capabilities),
            "started_at": self.started,
            "completed_at": time.time(),
            "exitstatus": int(session.exitstatus),
            "cases": self.cases,
            "groups": self.groups,
        }
        data = json.dumps(value, sort_keys=True).encode()
        if len(data) > settings.receipt_max_bytes:
            raise ValueError("native report budget exceeded")
        temporary = self.destination.with_suffix(".part")
        native_candidates.contained(candidate.layout, temporary)
        with temporary.open("xb") as stream:
            stream.write(data)
        temporary.chmod(0o600)
        temporary.replace(self.destination)

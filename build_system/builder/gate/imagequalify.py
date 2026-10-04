"""Qualify one image by booting it under Capsem: `capsem-gate image-qualify`.

The same harness the main gate runs on the reference image
(`[functional.qualification]`, tests/qualification): the mandatory groups every
image passes, plus the groups its `images/<name>/qualify.toml` capabilities add.
`--image NAME --layout PATH` names a candidate -- an OCI layout as it was
built or pulled by digest, never repacked; without them it qualifies the
reference image. Locally and in the image workflow it is the same command, so
a publication is never the first time an image boots under Capsem.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from . import pytestsuite, referenceimage, runtimeprepare
from .command import GateCommand
from .errors import GateError
from .plan import Plan
from .testmodules import InWorkspace


class ImageQualifyModule(
    InWorkspace,
    GateCommand,
    name="image-qualify",
    help="boot one image under Capsem and run its qualification groups",
):
    uses_qualification = True
    outside_egress = True

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("--image", help="the candidate's catalog name (with --layout)")
        parser.add_argument("--layout", type=Path, help="the candidate's OCI layout (with --image)")

    def plan(self) -> Plan:
        image, layout = getattr(self._args, "image", None), getattr(self._args, "layout", None)
        if (image is None) != (layout is None):
            raise GateError("image-qualify takes --image and --layout together, or neither")
        settings = self._config.functional.qualification
        plan = Plan(self.name)
        ready = () if self.qualification.pulled else (
            runtimeprepare.prepare(plan, self._config, permission=self.rebuild_permission).ready,
        )
        phase = plan.phase("qualification")
        if image is None or layout is None:
            ready = (phase.add(referenceimage.prepare(self._config), after=ready),)
            variables: tuple[tuple[str, str], ...] = ()
        else:
            variables = (
                (settings.image_variable, image),
                (settings.layout_variable, str(Path(layout).resolve())),
            )
        phase.add(
            pytestsuite.Suite(
                label="pytest.qualification",
                paths=(settings.suite_path,),
                contends=pytestsuite.sharing(self._config),
                variables=variables,
            ).as_step(self._config),
            after=ready,
        )
        return plan

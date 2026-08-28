"""The run-time question `functional` asks before any profile lane runs.

Split out of `module_functional`, which had reached its ceiling -- and along
the seam this gate is built on rather than at a convenient line. Everything
left there *plans*: it names steps and declares what they follow, with the
machine sealed. This *acts*: it reads a tree that exists only once the build
has run. `plan()` may not depend on build output, which is precisely why this
check could not stay where the decision it guards is made, and had to become a
step that runs.
"""

from __future__ import annotations

from pathlib import Path

from . import profiles
from .actions import Action
from .context import Context


class AxisAgrees(Action, name="axis-agrees"):
    """Check the materialized profiles are the ones the plan was built for.

    `selected()` reads checked-in `config/profiles/`; this reads what the
    build actually materialized, and refuses when they differ. A materialized
    catalog that does not match means the gate would prove a pairing nobody is
    shipping -- which is why the check did not go away when the plan stopped
    reading build output, it moved to where it can run.
    """

    def __init__(self, assets: Path | None = None, profiles_dir: Path | None = None) -> None:
        self._assets = assets
        self._profiles_dir = profiles_dir

    def render(self) -> str:
        """Name what is being checked, not just that a check happens.

        Two lanes ask this question of two different trees -- the layout the
        build left behind, and a cohort staged the way a release stages one.
        A fixed string made those indistinguishable in the run log, so a
        failure did not say which tree it had read.
        """
        where = self._profiles_dir or self._assets
        return "check the materialized profiles match the checked-in axis" + (
            f" in {where}" if where is not None else ""
        )

    def perform(self, context: Context) -> None:
        profiles.agree(
            context.config,
            profiles_dir=self._profiles_dir,
            manifest=(
                self._assets / context.config.install.manifest_name
                if self._assets is not None
                else None
            ),
        )
        context.journal.note(f"profile axis {', '.join(profiles.selected(context.config))}")

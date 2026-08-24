"""The one command each release lane runs to qualify what it is publishing.

One verb per artifact family, and the only `just` recipes any workflow needs.

Before these existed, `release-assets.yaml` and `release.yaml` each assembled
their lane out of private `_test-*` recipes: three or four steps, in an order
restated in YAML, with the deferred-profile branch expressed as a step-level
`if:`. The bodies were shared but the sequence was not, which is how the asset
lane grew a `_test-profile-artifacts` branch that the binary lane never got.
Private primitives had become the integration surface, so the integration had
nowhere to live except the workflow files, twice.

`release-binaries` and `release-profile` cannot be reused here, and that is not
an oversight. Those are the operator's dispatchers: they accept a qualified
commit, publish the immutable source ref, and dispatch these very workflows. A
workflow calling one would dispatch itself.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .command import GateCommand
from .config import GateConfig
from .content import ProfileContent
from .module_artifacts import artifacts, pulled_artifacts
from .module_functional import functional
from .module_glowup import glowup
from .plan import Plan
from .qualification import Qualification
from .sourcestate import record_step, verify_step
from .testmodules import InWorkspace


def _pairing(
    plan: Plan,
    config: GateConfig,
    qualification: Qualification,
    *,
    staged: ProfileContent | None = None,
) -> Plan:
    """Artifacts, then every VM suite, then the installed-package proof.

    The order is the dependency: the functional suites boot what the artifact
    module produced, and the glow-up installs the package those suites proved.
    """
    recorded = plan.shared(record_step(config))
    built = artifacts(plan, config, qualification=qualification, after=(recorded,))
    proved = functional(
        plan,
        config,
        qualification=qualification,
        after=(built,),
        staged=staged,
    )
    completed = glowup(
        plan,
        config,
        qualification=qualification,
        after=(proved,),
        staged=staged,
    )
    plan.add(verify_step(), after=(completed,))
    return plan


class QualifyBinariesModule(
    InWorkspace,
    GateCommand,
    name="qualify-binaries",
    help="qualify the candidate packages against the manifest-selected profiles",
):
    """The binary lane's whole proof, as one step a workflow can call.

    The staged roots arrive as arguments for the same reason `qualify-assets`
    takes its input directory as one: this runs from a private prefix that
    carries only tracked files, so anything the lane staged into the workspace
    has to be named absolutely or it cannot be found at all.
    """

    uses_qualification = True

    # Its glow-up installs a system package. Bubblewrap sets
    # `PR_SET_NO_NEW_PRIVS`, which stops sudo dead, so `glowup.package`
    # declares `outside_sandbox=True` -- and that declaration only means
    # anything if this command holds the egress resource to honour it.
    # Without it the step ran inside the sandbox anyway and the lane failed two
    # hours in with `sudo: /etc/sudo.conf is owned by uid 65534`, which names
    # neither the step nor the sandbox. The local `candidate` lane holds the
    # same resource for the same reason; this lane is the one no local run
    # reaches.
    outside_egress = True

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("workspace_root", type=Path)

    def plan(self) -> Plan:
        return _pairing(
            Plan(self.name),
            self._config,
            self.qualification,
            staged=ProfileContent.staged(self._config, self._args.workspace_root),
        )


class QualifyAssetsModule(
    InWorkspace,
    GateCommand,
    name="qualify-assets",
    help="qualify one profile's built assets against the selected binary",
):
    """The asset lane's whole proof, including the branch the YAML used to own.

    A channel with no package cohort yet can publish a profile immutably but
    inactive. That is not a lesser pairing, it is a different one: there is no
    binary to pair against, so the functional and glow-up modules have nothing
    to run and the artifact proof stands alone. The lane decides that here,
    from the flag the authoring job computed, rather than in two `if:`
    expressions a reader has to reassemble.
    """

    # Same reason as the binary lane: when a binary exists to pair against,
    # this composes the glow-up that installs a package outside the sandbox.
    outside_egress = True

    uses_qualification = True

    @classmethod
    def add_arguments(cls, parser: argparse.ArgumentParser) -> None:
        parser.add_argument("input_dir", type=Path)
        parser.add_argument("profile")
        parser.add_argument("workspace_root", type=Path)
        parser.add_argument(
            "--activation-ready",
            required=True,
            choices=("true", "false"),
            help="whether a compatible package cohort exists to pair against",
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self._args.activation_ready != "true":
            recorded = plan.shared(record_step(self._config))
            proved = pulled_artifacts(
                plan,
                self._config,
                input_dir=self._args.input_dir,
                profile=self._args.profile,
                after=(recorded,),
            )
            plan.add(verify_step(), after=(proved,))
            return plan
        # The activation-ready pairing stages into the workspace exactly as the
        # binary lane does, and qualifies from the same kind of private prefix,
        # so it needs the same absolute anchor. Taken unconditionally: a root
        # supplied only on the branch that happens to be exercised is a root
        # nobody notices is missing.
        return _pairing(
            plan,
            self._config,
            self.qualification,
            staged=ProfileContent.staged(self._config, self._args.workspace_root),
        )

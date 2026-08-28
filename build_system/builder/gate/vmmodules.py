"""The modules that need built artifacts, a VM, or both.

Split from `testmodules`, which holds the ones provable from a bare checkout.
The seam is what a module needs before it can start: these three cannot run
against source alone, and the others cannot tell you anything source does not
already contain.

Each phase now owns a file -- `module_artifacts`, `module_functional`,
`module_glowup` -- because three independently composed release phases in one
module is three reasons for it to change. This re-exports them so composition
and the command registry keep one import site.
"""

from __future__ import annotations

from .module_artifacts import ArtifactsModule, artifacts
from .module_functional import FunctionalModule, functional
from .module_glowup import GlowupModule, glowup

__all__ = [
    "ArtifactsModule",
    "FunctionalModule",
    "GlowupModule",
    "artifacts",
    "functional",
    "glowup",
]

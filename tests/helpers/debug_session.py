"""Sessions of capsem-debug, the test-tooling image.

The VM runtime carries no test tooling. A test that needs a test runner, a
network tool, a package manager, a model SDK or an agent CLI opens one of
these sessions and execs into its workload; `"target": "vm"` still reaches
the VM. The image is served by the digest `config/gate.toml` pins, from a
hermetic registry on loopback (tests/fixtures/oci/README.md).

Product behavior -- what a user's own workload does -- belongs on the
reference image (`helpers.image_session.image_session`); capsem-debug is for
tests that need tooling the product image does not ship.
"""

from __future__ import annotations

import functools

from helpers.image_session import WORKSPACE, image_session
from tests.fixtures.oci.pinned_image import debug_image

__all__ = ["WORKSPACE", "debug_session"]

#: A named session whose workload is capsem-debug; yields its id once the
#: workload runs, and deletes it afterwards.
debug_session = functools.partial(image_session, image=debug_image)

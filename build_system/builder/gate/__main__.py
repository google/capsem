"""`python -m capsem_builder.gate`, for callers without the console script on PATH.

Through the launcher rather than straight to `cli`, so this spelling gets the
same isolated bytecode cache the console script does -- and so the launcher's
own re-exec, which lands here with the cache already isolated, dispatches
instead of looping.
"""

from __future__ import annotations

from ..gatelaunch import main

raise SystemExit(main())

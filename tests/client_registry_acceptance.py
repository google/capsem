"""Private post-publication driver using the real service lifecycle fixture."""

from __future__ import annotations

import argparse
import os
import tempfile
import time
from pathlib import Path

from capsem_builder.gate.config import load
from capsem_builder.release.tools.client_consumers import verify


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary-dir", type=Path, required=True)
    arguments = parser.parse_args()
    binaries = arguments.binary_dir.resolve()
    for name in ("capsem-service", "capsem-gateway", "capsem-process", "capsem-tray", "capsem-ledger"):
        if not (binaries / name).is_file():
            raise ValueError(f"missing native release binary: {name}")
    os.environ.update(CAPSEM_RELEASE_BIN_DIR=str(binaries), CAPSEM_TRAY_HEADLESS="1")
    # Fixture constants must resolve the selected native cohort, never the
    # development target. No VM is needed to prove the polled service API.
    from helpers.service import ServiceInstance

    with tempfile.TemporaryDirectory(prefix="capsem-client-assets-") as directory:
        service = ServiceInstance(assets_dir=Path(directory), sign_binaries=False)
        try:
            service.start()
            deadline = time.monotonic() + 15
            while not all((service.tmp_dir / name).is_file() for name in ("gateway.port", "gateway.token")):
                if time.monotonic() >= deadline:
                    raise RuntimeError("release gateway did not become ready")
                time.sleep(0.1)
            environment = dict(os.environ, CAPSEM_RUN_DIR=str(service.tmp_dir))
            verify(load(Path.cwd()), environment)
        finally:
            service.stop()


if __name__ == "__main__":
    main()

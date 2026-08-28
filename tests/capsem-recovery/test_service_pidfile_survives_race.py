"""The pidfile a harness reaps by must name whoever is actually serving.

`$run_dir/service.pid` is the only handle anything has on a detached service:
the asset gate's `stop_gate_pidfile`, `_ensure-service`, and every abort path
reap by it. `stop_gate_pidfile` on a missing file silently succeeds, so a
pidfile that disappears is indistinguishable from a service that was reaped.

Startup is deliberately self-idempotent -- a second starter that finds a
compatible peer already serving exits 0. That loser used to write the pidfile
before the race resolved, so its shutdown guard deleted the *winner's* pid on
the way out. The winner then survived every subsequent cleanup, reparented to
launchd with its gateway and tray, while the gate reported success.

`build_system/tests/scripts/test_pidfile_cleanup_is_wired.py` proves the reaping is wired to a
pidfile something writes. This proves the pidfile still names the live service
once the startup race has been run.
"""

import shutil

import pytest
from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_losing_starter_leaves_the_winners_pidfile_intact():
    """A peer that exits 0 must not take the serving pid with it."""
    winner = ServiceInstance()
    winner.start()
    pidfile = winner.tmp_dir / "service.pid"

    try:
        assert pidfile.read_text().strip() == str(winner.proc.pid), (
            "the serving service must record its own pid for the reaper to find"
        )

        # The asset gate's shape: one run dir, one socket, a second starter
        # probing it. Sharing the run dir is the whole point -- both services
        # resolve the same `service.pid`.
        loser = ServiceInstance()
        shutil.rmtree(loser.home_dir, ignore_errors=True)
        loser.home_dir = winner.home_dir
        loser.tmp_dir = winner.tmp_dir
        loser.uds_path = winner.uds_path

        loser.start()
        assert loser.proc.wait(timeout=30) == 0, (
            "a starter that finds a compatible peer must exit 0"
        )
        loser.stop(cleanup=False)

        assert winner.proc.poll() is None, "the winner must still be serving"
        assert pidfile.exists(), (
            "the losing starter erased the pidfile; every later reap now finds "
            "nothing, reports success, and leaves the winner running"
        )
        assert pidfile.read_text().strip() == str(winner.proc.pid), (
            "the pidfile must name the service that is actually serving"
        )
    finally:
        winner.stop()

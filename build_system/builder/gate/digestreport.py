"""Rendering the digest, and putting it where it will actually be read.

Separate from `rundigest`, which decides what is true. This decides what is
worth saying, and the constraint that shapes it is unusual: this document is
injected into an agent's context on every session, so every line is paid for on
work that has nothing to do with the gate. A digest that grows without bound
gets skimmed, then ignored, then removed by whoever is trying to save tokens.

So it is deliberately short and ordered by what changes a decision: whether the
last run passed, what to do about it, and only then the tables the conclusions
came from. Bounded by `recent_runs` and `hotspots`, both config-owned.
"""

from __future__ import annotations

from pathlib import Path

from .actions import Action
from .config import GateConfig
from .context import Context
from .digestschema import DigestConfig
from .filesystem import write_text
from .rundigest import Analysis, advice, analyse
from .runledger import LedgerRow, rows, sync
from .runlogschema import FAILED, OK, SKIPPED
from .timing import clock


def render(analysis: Analysis, settings: DigestConfig, *, ledger_rows: int) -> str:
    """The whole document. Markdown, because its readers are models."""
    lines = ["# Gate status", ""]
    latest = analysis.latest
    if latest is None:
        return "\n".join(
            [
                *lines,
                "No completed gate runs recorded yet. `just test` seeds this.",
                "",
            ]
        )

    verdict = "FAILED" if latest.status != OK else "ok"
    lines += [
        f"**Last run** `{latest.run_id}` -- {verdict} -- {clock(latest.total_ms)}"
        f" -- head `{latest.head[:12] or 'unknown'}`",
        "",
        "## Act on this",
        "",
    ]
    lines += [f"- {line}" for line in advice(analysis, settings)]
    lines += [
        "",
        "## Recent runs",
        "",
        "| run | command | elapsed | ok | failed | skipped |",
        "|---|---|---|---|---|---|",
    ]
    for row in analysis.recent:
        counts = _counts(row)
        lines.append(
            f"| `{row.run_id[:22]}` | {row.command} | {clock(row.total_ms)} "
            f"| {counts[OK]} | {counts[FAILED]} | {counts[SKIPPED]} |"
        )

    if analysis.hotspots:
        window = analysis.hotspots[0].window
        lines += [
            "",
            f"## Where the time goes ({window} comparable runs)",
            "",
            "| step | median | on critical path | queueing |",
            "|---|---|---|---|",
        ]
        for hotspot in analysis.hotspots:
            lines.append(
                f"| `{hotspot.label}` | {clock(hotspot.median_ms)} "
                f"| {hotspot.residency}/{hotspot.window} | {hotspot.wait_share:.0%} |"
            )

    lines += [
        "",
        f"_{ledger_rows} runs in the ledger. `capsem-gate runs trend` for the full history,"
        " `capsem-gate runs show <id>` for one run._",
        "",
    ]
    return "\n".join(lines)


def _counts(row: LedgerRow) -> dict[str, int]:
    counted = {OK: 0, FAILED: 0, SKIPPED: 0}
    for step in row.steps.values():
        if step.status in counted:
            counted[step.status] += 1
    return counted


def build(config: GateConfig) -> str:
    """Read the ledger, work out what it means, and render it."""
    settings = config.runlog.digest
    # Heals first: a directory still on disk but absent from the ledger is
    # history the digest would otherwise report as never having happened.
    sync(config, config.runlog)
    history = rows(config)
    return render(analyse(history, settings), settings, ledger_rows=len(history))


def write(config: GateConfig) -> Path:
    """Regenerate the digest in place. Returns where it went."""
    target = config.path(config.runlog.digest.path)
    write_text(target, build(config))
    return target


class RefreshDigest(Action, name="run-digest"):
    """Rebuild the overview early, as a step that is allowed to fail.

    `RunLog.close` also writes it, but best-effort: close runs on the failure
    path and may not raise there, or it would replace the failure somebody
    needs to read. That leaves a silent degradation, and this is the other
    half -- an ordinary step, in the fast phase, whose failure is a failure.

    First in the phase deliberately. The digest is the one artifact a person
    or an agent wants *before* deciding what to do with the run they just
    started, and one produced at the end is one produced too late.
    """

    def render(self) -> str:
        return "regenerate the cross-run digest from the ledger"

    def perform(self, context: Context) -> None:
        # Interrogating a plan must not write to the checkout. `runs --dry-run`
        # and every contract that builds a real plan pass `observing`, and the
        # last primitive to forget overwrote the running gate's own source
        # state and failed `source.verify` forty minutes later.
        if context.observing:
            return
        context.journal.note(f"digest refreshed at {write(context.config)}")

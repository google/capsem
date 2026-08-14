"""Citadel guard: a verdict may be thrown away, but not silently.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one records the cheapest possible bug with the most expensive
consequence.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

from capsem.gate import config as gate_config
from capsem.gate.exclusions import canonical, reconcile
from capsem.gate.shellnodes import suppressed
from capsem.gate.shellparse import parse
from capsem.gate.shellsurfaces import run_instructions, workflow_bodies

ROOT = Path(__file__).resolve().parents[2]
LEDGER_ENTRIES = gate_config.load(ROOT).boundary.discarded_verdicts

SUPPRESSION_RATIONALE = """\
`command || true` runs a command and throws its exit status away.

Most uses in this tree are right: clearing state that may not be set, killing a
process that may not be running, printing diagnostics after the failure they
describe. Two are deliberate cache prewarms. None is a bug today.

They are ledgered anyway, because this is the shape that costs the most for the
least. `test "$X" = success || true` satisfied a release contract while branch
protection was switched off. The check ran. It failed. The step passed. Nothing
in the run said so, and nothing could -- an exit status thrown away leaves no
trace anywhere.

Two wrong shapes were tried first and are worth naming, because both look
reasonable:

  - **A count per file.** It fails when somebody adds a harmless `pkill`, and
    passes when somebody turns one of the existing nine into a real check that
    is now ignored. The number is orthogonal to the risk.
  - **A list of tolerant program names.** `launchctl bootout` on an unloaded
    service is expected to fail; `launchctl` elsewhere is a real check. The
    program says nothing about whether the verdict mattered. Four of the five
    findings that rule produced were its own misclassification.

So the ledger pins the exact command, by a hash of its *parsed argv*. Requote
it, reflow it, move it to another file: same decision, same hash, no churn.
Change what is actually suppressed and it is a new decision, which has to be
stated as one.

Read with a parser, because the question is grammatical. `|| true` inside a
quoted argument suppresses nothing, `a || b` is a fallback rather than a
suppression, and `true && risky` is not a suppression at all.

See src/capsem/gate/exclusions.py and skills/dev-gate/SKILL.md.
"""

LEDGER = "[[boundary.discarded_verdicts]] in config/gate.toml"


def tracked(*patterns: str) -> list[str]:
    return subprocess.run(
        ["git", "ls-files", *patterns], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.split()


def shell_surfaces() -> list[tuple[str, str]]:
    """Every body in the repository that bash will execute.

    All three surfaces, because the bug does not care which file it lives in
    and a guard that covered only `scripts/` would have missed the nine in
    `release.yaml`.
    """
    found = [(name, (ROOT / name).read_text(encoding="utf-8")) for name in tracked("*.sh")]
    for name in tracked("Dockerfile*", "*/Dockerfile*"):
        body = (ROOT / name).read_text(encoding="utf-8")
        found += [(name, run) for run in run_instructions(body)]
    found += [
        (key.split(":")[0], body)
        for key, body in workflow_bodies(ROOT / ".github" / "workflows").items()
    ]
    return found


def discarded() -> dict[str, str]:
    """Digest to rendering, for every command whose verdict is thrown away."""
    found: dict[str, str] = {}
    for where, body in shell_surfaces():
        for command in suppressed(parse(body)):
            found[canonical(command.argv)] = f"{where}: {' '.join(command.argv)}"
    return found


def test_every_discarded_verdict_is_declared_with_a_reason() -> None:
    found = discarded()
    assert found, "found no suppressions at all -- the extractor or parser regressed"

    ledger = LEDGER_ENTRIES
    outcome = reconcile(found, [entry.digest for entry in ledger])
    assert outcome.clean, SUPPRESSION_RATIONALE + "\n" + outcome.report(add=LEDGER)


def test_the_ledger_describes_what_it_excuses() -> None:
    """An entry's rendering must still match the command it was granted for.

    The digest pins identity, but a reader reconciles by reading. A `subject`
    that has drifted from the command is how a ledger stops being reviewable
    while every automated check still passes.
    """
    found = discarded()
    ledger = LEDGER_ENTRIES
    wrong = [
        f"{entry.digest}: ledger says {entry.subject!r}, tree has {found[entry.digest].split(': ', 1)[1]!r}"
        for entry in ledger
        if entry.digest in found and found[entry.digest].split(": ", 1)[1] != entry.subject
    ]
    assert not wrong, SUPPRESSION_RATIONALE + "\n" + "\n".join(wrong)


def test_the_guard_reads_grammar_rather_than_text() -> None:
    """The distinctions that make this worth parsing for."""
    assert [c.program for c in suppressed(parse("risky || true"))] == ["risky"]
    assert [c.program for c in suppressed(parse("risky || :"))] == ["risky"]
    assert suppressed(parse("risky || fallback")) == [], "a real fallback is not suppression"
    assert suppressed(parse("true && risky")) == [], "only the right of || discards"
    assert suppressed(parse('echo "risky || true"')) == [], "a quoted mention suppresses nothing"
    assert [c.program for c in suppressed(parse("f() { risky || true; }"))] == ["risky"]


def test_the_digest_survives_formatting_but_not_meaning() -> None:
    """Why the hash is over parsed argv and not over source text.

    A ledger keyed on text churns on every reflow, and a ledger that churns
    gets regenerated wholesale instead of reviewed -- at which point it excuses
    whatever is currently there, which is no ledger at all.
    """
    plain = suppressed(parse("cargo build --workspace || true"))
    requoted = suppressed(parse('cargo   "build"   --workspace  || true'))
    continued = suppressed(parse("cargo build \\\n  --workspace || true"))
    assert canonical(plain[0].argv) == canonical(requoted[0].argv) == canonical(continued[0].argv)

    changed = suppressed(parse("cargo build --workspace --all-targets || true"))
    assert canonical(changed[0].argv) != canonical(plain[0].argv), (
        "a different command must be a different decision"
    )

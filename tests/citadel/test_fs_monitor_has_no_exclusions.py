"""Citadel guard: the host file monitor may not blind itself to any path.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one records a security review finding: `fs_monitor.rs` carried an
`EXCLUDED_DIRS` list (`.git`, `node_modules`, `__pycache__`, `.cache`, `target`,
`.venv`, `.swapfile`) and a `should_exclude` predicate applied both to the
notify event path and to the workspace snapshot walk, so every change under
those directories was dropped before it could become a ledger row or reach a
file security rule.

Those are precisely the persistence and supply-chain paths an incident
responder looks at first: `.git/hooks/*` and `.git/config` (`core.hooksPath`,
`core.fsmonitor`), `node_modules/<pkg>/package.json` install scripts,
`.venv/bin/activate`, and build scripts in the guest's build output directory.
A ledger that silently
omits them is not merely incomplete, it is a forensic lie: it reports a clean
session for a workspace that was backdoored.

The list existed only to reduce event volume. Volume is a cost question and is
answered by the monitor's adaptive poll interval, never by deciding in advance
which attacks are not worth recording. There is no config knob for this on
purpose: an exclusion a user can switch on is an exclusion an attacker can ask
for.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
FS_MONITOR = PROJECT_ROOT / "crates/capsem-core/src/fs_monitor.rs"
FS_MONITOR_DIR = PROJECT_ROOT / "crates/capsem-core/src/fs_monitor"

FS_MONITOR_RATIONALE = """\
The host file monitor must record every path under the workspace.

A security review found that fs_monitor.rs dropped every event whose path had a
`.git`, `node_modules`, `__pycache__`, `.cache`, `target`, `.venv` or
`.swapfile` component -- hiding `.git/hooks/*`, `.git/config`, npm install
scripts, `.venv/bin/activate` and build-output scripts from the ledger and
from the file security rules. Do not reintroduce an exclusion list, a skip
predicate, a `filter_entry` on the snapshot walk, or a config knob for either.
Cost is answered by the adaptive poll interval, not by blindness.

See skills/dev-session-debug/SKILL.md and CLAUDE.md 'Logger DB Boundary'.
"""

# Identifier fragments that only appear when someone is deciding, inside the
# monitor, that some paths are not worth watching.
FORBIDDEN_IDENTIFIERS: tuple[tuple[str, str], ...] = (
    ("EXCLUD", "an exclusion list under any spelling"),
    ("IGNORED_DIRS", "an exclusion list under another name"),
    ("SKIP_DIRS", "an exclusion list under another name"),
    ("DENY_DIRS", "an exclusion list under another name"),
    ("PRUNE_DIRS", "an exclusion list under another name"),
    ("should_exclude", "an exclusion predicate"),
    ("should_skip", "an exclusion predicate under another name"),
    ("should_ignore", "an exclusion predicate under another name"),
    ("is_noisy", "an exclusion predicate wearing a cost argument"),
    ("filter_entry(", "a pruned snapshot walk"),
    ("filter_entry (", "a pruned snapshot walk, spaced past the check above"),
    ("follow_links(true", "a walk that descends a guest symlink"),
    ("follow_links (true", "a walk that descends a guest symlink, spaced"),
    ("PollWatcher", "notify's poller, which follows links and cannot be told not to"),
)

# Path literals whose only use in the monitor was to name what not to record.
#
# The check is deliberately quote-scoped: it matches the Rust string literal,
# not the bare word, so prose in this file and comments in the monitor can name
# the paths the finding is about. The cost of that scoping is that a doc
# comment in the monitor must not write these paths in straight double quotes
# -- spell them in backticks or bare, as the module doc does -- or this guard
# will read the comment as the list coming back. That is the right trade: a
# guard that cannot be explained is a guard nobody keeps.
#
# The identifiers above are matched bare, so the same discipline is stricter
# for them: the monitor's own prose must not spell `PollWatcher` or the
# link-following setting even to explain why they are refused. The module doc
# says so and points here.
FORBIDDEN_LITERALS: tuple[tuple[str, str], ...] = (
    ('"node_modules"', "a hardcoded supply-chain path"),
    ('".git"', "a hardcoded persistence path"),
)


def fs_monitor_findings(path: str, text: str) -> list[str]:
    """Pure predicate: what makes this monitor source blind, if anything."""
    findings: list[str] = []
    for needle, reason in FORBIDDEN_IDENTIFIERS + FORBIDDEN_LITERALS:
        if needle in text:
            findings.append(f"{path} contains `{needle}` ({reason})")
    return findings


def monitor_sources() -> list[Path]:
    """Every non-test source of the monitor."""
    sources = [FS_MONITOR]
    if FS_MONITOR_DIR.is_dir():
        sources.extend(
            path
            for path in sorted(FS_MONITOR_DIR.rglob("*.rs"))
            if path.name != "tests.rs" and "tests" not in path.relative_to(FS_MONITOR_DIR).parts
        )
    return sources


def test_the_monitor_source_exists() -> None:
    """A guard over a file nobody has asserts nothing."""
    assert FS_MONITOR.is_file(), f"{FS_MONITOR} is missing; this guard is vacuous"


def test_the_monitor_records_every_path() -> None:
    findings: list[str] = []
    for source in monitor_sources():
        findings.extend(
            fs_monitor_findings(str(source.relative_to(PROJECT_ROOT)), source.read_text())
        )
    assert not findings, FS_MONITOR_RATIONALE + "\n" + "\n".join(findings)


def test_the_predicate_catches_the_shapes_the_review_found() -> None:
    """The adversarial case: every spelling the exclusion could come back as."""
    adversarial = """
    const EXCLUDED_DIRS: &[&str] = &[".git", "node_modules"];
    const IGNORED_DIRS: &[&str] = &[];
    const SKIP_DIRS: &[&str] = &[];
    const DENY_DIRS: &[&str] = &[];
    const PRUNE_DIRS: &[&str] = &[];
    fn should_exclude(path: &Path) -> bool { true }
    fn should_skip(path: &Path) -> bool { true }
    fn should_ignore(path: &Path) -> bool { true }
    fn is_noisy(path: &Path) -> bool { true }
    fn walk() { WalkDir::new(dir).into_iter().filter_entry(|e| true); }
    fn spaced() { WalkDir::new(dir).into_iter().filter_entry (|e| true); }
    fn follows() { WalkDir::new(dir).follow_links(true).into_iter(); }
    fn follows_spaced() { WalkDir::new(dir).follow_links (true).into_iter(); }
    fn delegated() { let w = PollWatcher::new(cb, config)?; }
    """
    findings = fs_monitor_findings("adversarial.rs", adversarial)
    assert len(findings) == len(FORBIDDEN_IDENTIFIERS) + len(FORBIDDEN_LITERALS), findings


def test_the_predicate_allows_an_honest_monitor() -> None:
    """The legitimate shape: watch everything, pay for it with the interval."""
    honest = """
    const POLL_INTERVAL_MIN_MS: u64 = 500;
    fn poll_interval_for_scan(scan: Duration) -> Duration { scan * 10 }
    fn workspace_snapshot(dir: &Path) {
        WalkDir::new(dir).min_depth(1).follow_links(false).into_iter();
    }
    """
    assert fs_monitor_findings("honest.rs", honest) == []

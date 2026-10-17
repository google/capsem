"""Citadel guard: nothing agent-facing says a release can skip `just test`.

A release dispatches only a source whose complete local `just test` passed on
this machine (RELEASE.md 8.1). The mechanism refuses; this guard keeps the
words from contradicting it, because agents follow the words first.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

LOCAL_PROOF_RATIONALE = """\
The repository told agents a release does not need `just test`.

AGENTS.md, RELEASE.md, the release skills and the gate's own refusal text
said each release command was "sufficient on its own", that its
hosted lane "self-qualifies", and that a complete local run was "optional".
Agents believed it. Four consecutive stable release attempts then each failed
in the hosted lane after about two and a half hours, on a defect -- the
installed-package glow-up checking one version's binary names against the
previous package -- that the local glow-up and functional lanes find in
minutes. That cost a day.

The rule is now: nothing runs in CI that `just test` has not run locally
first. Both release commands refuse a source without a passing journal
(`qualificationflow.decide`), and the hosted lane still qualifies what it
publishes. Say that, not the old model. The only exemption is the unattended
nightly scheduler, `[release].unattended_channels` in config/gate.toml.
"""

#: Agent-facing sources: the contract, the release spec, every skill and its
#: references, the recipe surface, and the gate's own operator messages.
SURFACES = (
    "AGENTS.md",
    "RELEASE.md",
    "justfile",
    "skills/**/*.md",
    "build_system/builder/gate/**/*.py",
    "build_system/builder/cache/**/*.py",
)

#: The spellings the old model used. Matched case-insensitively over text with
#: every run of whitespace, Markdown emphasis, and backticks collapsed, so a
#: phrase wrapped across lines or around `just test` still matches.
FORBIDDEN = (
    r"self[- ]qualif",
    r"sufficient on (?:its|their) own",
    r"sufficient by itself",
    r"optional (?:complete|reusable|whole)[- ](?:local|world|system)",
    r"local full run is optional",
    r"optional before (?:a )?(?:release|publication)",
    r"not required before (?:a )?(?:release|publication)",
    r"candidate produces optional local qualification",
    r"just test (?:<[^>]*> )?is optional",
    r"(?:is )?not a (?:release )?prerequisite",
    (
        r"(?:must not|do not|does not|never) (?:require or )?(?:consume|require) "
        r"(?:a |the |this |that )?(?:developer-machine |machine-local |optional |local )*"
        r"(?:just test|journal|complete run)"
    ),
    # The hosted lane qualifies artifacts independently; the public dispatcher
    # accepts local proof. Scope the singular wording to that dispatcher.
    (
        r"release (?:command|dispatcher) never consumes "
        r"(?:a |the |this |that )?(?:developer-machine |machine-local |local )*"
        r"(?:just test|journal|complete run)"
    ),
    r"ignore machine-local (?:candidate )?journals",
)
PATTERN = re.compile("|".join(f"(?:{item})" for item in FORBIDDEN), re.IGNORECASE)

#: Places where the words are true: installing the product by hand genuinely
#: is not a release prerequisite. Each entry is a phrase, not a file, so the
#: rest of that file is still held.
ALLOWED = ("just install", "local install", "hands-on")


def _normalized(text: str) -> str:
    return re.sub(r"\s+", " ", re.sub(r"[`*_]", "", text))


def violations(text: str) -> list[str]:
    """Every forbidden phrase in ``text``, with a little context."""
    flat = _normalized(text)
    found = []
    for match in PATTERN.finditer(flat):
        context = flat[max(0, match.start() - 80) : match.end() + 40]
        if any(allowed in context.lower() for allowed in ALLOWED):
            continue
        found.append(context.strip())
    return found


def _sources() -> list[Path]:
    paths = {path for pattern in SURFACES for path in PROJECT_ROOT.glob(pattern)}
    return sorted(path for path in paths if path.is_file() and "node_modules" not in path.parts)


def test_the_guard_reads_every_surface() -> None:
    """A glob that silently matches nothing proves nothing."""
    sources = {path.relative_to(PROJECT_ROOT).as_posix() for path in _sources()}
    for required in (
        "AGENTS.md",
        "RELEASE.md",
        "justfile",
        "skills/release-process/SKILL.md",
        "skills/release-process/references/qualification-and-test-composition.md",
        "build_system/builder/gate/testadmission.py",
        "build_system/builder/gate/qualificationevidence.py",
    ):
        assert required in sources, f"the local-proof guard no longer reads {required}"


def test_no_agent_facing_text_says_a_release_can_skip_just_test() -> None:
    offenders = [
        f"{path.relative_to(PROJECT_ROOT)}: ...{context}..."
        for path in _sources()
        for context in violations(path.read_text(encoding="utf-8"))
    ]

    assert not offenders, LOCAL_PROOF_RATIONALE + "\nStill saying it:\n  " + "\n  ".join(
        offenders
    )


@pytest.mark.parametrize(
    "text",
    (
        "Each release command is sufficient on its own because its hosted lane",
        "Release dispatchers are sufficient\non their own; hosted lanes self-qualify",
        "Release commands self-qualify; a local full run is optional.",
        "just test \"$c\" # Optional complete local proof; exact repeats reuse it",
        "`just test <source-commit>` is optional reusable local whole-system",
        "so `just test` is not a\n   prerequisite.",
        "It is optional before publication.",
        "Release dispatch MUST NOT require or consume a developer-machine\n`just test` journal.",
        "Release commands self-qualify and do not consume a local\n`just test` prerequisite.",
        "- [ ] Both release commands ignore machine-local candidate journals",
        "hosted lanes and never require this optional local journal.",
        "Release commands MUST NOT\nconsume that machine-local journal.",
        "It is diagnostic evidence, not a\nprerequisite consumed by either release dispatcher.",
        "dispatches its **self-qualifying** hosted lane",
        (
            "It is not required before release: the hosted release lane owns "
            "qualification and never consumes the local journal."
        ),
        (
            "Candidate produces optional local qualification; release commands dispatch "
            "hosted qualification."
        ),
        "The release command never consumes the local journal.",
    ),
)
def test_every_old_spelling_is_caught(text: str) -> None:
    """The phrases that were actually in the tree, wrapped the way they were."""
    assert violations(text), f"the guard misses: {text!r}"


@pytest.mark.parametrize(
    "text",
    (
        "Optional hands-on local testing; never a release prerequisite\njust install",
        (
            "A passing `just test <source-commit>` for the exact commit is required "
            "before either release command."
        ),
        "Release commands require it but never run it.",
        "Nightly consumes no local journal: its scheduler runs unattended.",
        "`--force` never waives the `just test` journal.",
        (
            "Hosted release qualification cannot consume local proof and MUST run "
            "its own complete artifact-family pairing."
        ),
        (
            "The hosted release lane never consumes the local journal; "
            "the public release command requires it before dispatch."
        ),
    ),
)
def test_the_new_model_is_not_flagged(text: str) -> None:
    assert not violations(text), f"the guard flags correct wording: {text!r}"

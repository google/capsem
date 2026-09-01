"""Every surface in the repository that carries shell, extracted once.

Three of them: tracked `*.sh`, every workflow `run:` body, and every Dockerfile
`RUN` body -- including the `.j2` templates, rendered rather than masked,
because the rendered output is what runs.

One extractor because two consumers ask the same question of the same text: the
shell audit lints these bodies, and the Citadel's shape guard measures them. A
second extractor is a second set of parsing bugs, and this one already had
four -- flattened continuations swallowing comments, comments stripped after
continuations rather than before, `${{ }}` masked to a constant, Jinja masked
instead of rendered.

Paths arrive as arguments. Nothing here spells a directory.
"""

from __future__ import annotations

import re
import shlex
from pathlib import Path

import yaml

#: Tokens that end one command and begin the next. `shlex` in
#: `punctuation_chars` mode emits each as a token of its own, which is the
#: whole reason to lex instead of matching: `cargo` in a filename, in a
#: comment, or on the left of an assignment is a different token from `cargo`
#: in command position, and a regex that tells those apart has become a lexer
#: with none of the testing.
SEPARATORS = frozenset({";", "|", "||", "&", "&&", "(", ")", "{", "}", "!"})

#: Words that run another command. The argv worth reporting is the one after
#: them, so they are stepped over rather than reported as the invocation.
WRAPPERS = frozenset(
    {"env", "sudo", "time", "exec", "caffeinate", "nice", "command", "then", "do", "!"}
)

#: A GitHub expression. The runner substitutes each as one value before bash
#: sees it, so it masks to a variable reference: masking to a literal makes
#: `[ "$X" = "y" ]` a constant comparison and ShellCheck rightly says SC2050.
EXPRESSION = re.compile(r"\$\{\{\s*.*?\s*\}\}", re.S)
EXPRESSION_PLACEHOLDER = "${CAPSEM_GH_EXPRESSION}"

RUN_INSTRUCTION = re.compile(r"^RUN\s+((?:.*\\\n)*.*)", re.MULTILINE)
RUN_OPTION = re.compile(r"--[A-Za-z][A-Za-z0-9-]*(?:=[^\\\s]+)?[ \t]*(?:\\\n[ \t]*)?")

#: A command continued onto the next line. Joined before lexing, so a five-line
#: invocation is one command rather than five fragments.
CONTINUATION = re.compile(r"\\\n\s*")


def executable_lines(body: str) -> list[str]:
    """The lines of `body` that do something: no blanks, no comments."""
    return [
        line for line in body.splitlines() if line.strip() and not line.lstrip().startswith("#")
    ]


def _without_comments(text: str) -> str:
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


def commands(body: str) -> list[tuple[str, ...]]:
    """Every command in `body`, as argv, lexed rather than matched.

    `shlex` handles the parts that defeat pattern matching one at a time and
    silently: quoting, escapes, `#` starting a comment only in word position,
    and operators that abut their arguments. Line continuations are joined
    first so a command split across five lines is one command, which is the
    form every interesting invocation in this repository takes.

    Assignment prefixes (`FOO=bar cmd`) and wrappers (`env`, `sudo`) are
    stepped over, so the argv reported is the command that actually runs.

    A lexer, not a parser: `$(...)` and `${...}` come back as opaque tokens and
    control words are commands like any other. That is enough to answer "what
    does this invoke", which is the only question asked of it.
    """
    found: list[tuple[str, ...]] = []
    for line in CONTINUATION.sub(" ", _without_comments(body)).splitlines():
        if not line.strip():
            continue
        lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        current: list[str] = []
        try:
            tokens = list(lexer)
        except ValueError:
            # An unbalanced quote, usually a fragment of a heredoc. Skipping
            # the line is right: reporting a guess about text we could not read
            # is worse than reporting nothing, and the caller is looking for
            # what is definitely there.
            continue
        for token in [*tokens, ";"]:
            if token in SEPARATORS or all(character in ";|&" for character in token):
                if current:
                    found.append(tuple(current))
                current = []
                continue
            if not current and ("=" in token.split("/")[0][:64] and not token.startswith("-")):
                continue  # FOO=bar prefix
            if not current and token in WRAPPERS:
                continue
            current.append(token)
    return found


def invocations(body: str, program: str) -> list[tuple[str, ...]]:
    """Every argv in `body` whose command is `program`.

    Full argv rather than a `(command, subcommand)` pair. The pair is wrong the
    moment a flag precedes the subcommand -- `pnpm --dir web/app run build`
    reads as `("pnpm", "--dir")` -- and which flags take a value is knowledge
    this module has no business holding. The caller knows what it is asking
    about, so it gets the tokens and asks.
    """
    return [argv for argv in commands(body) if argv and argv[0] == program]


def run_instructions(dockerfile: str) -> list[str]:
    """Every RUN body in a Dockerfile, the way Docker parses one.

    Comments are removed *before* continuations are joined, which is the order
    Docker uses. Reversing it leaves the `\\` preceding a comment line dangling
    and reports a syntax error in a RUN that builds correctly.

    Docker consumes leading instruction options such as BuildKit cache mounts;
    they are not arguments passed to `/bin/sh -c` and are stripped here.
    Continuations in the shell body are otherwise left intact: ShellCheck
    reads them, and flattening pulls a trailing comment into the middle of a
    logical line.
    """
    return [
        _without_run_options(body)
        for body in RUN_INSTRUCTION.findall(_without_comments(dockerfile))
    ]


def _without_run_options(body: str) -> str:
    """Remove only Docker's leading `RUN --option` prefix."""
    position = 0
    while found := RUN_OPTION.match(body, position):
        position = found.end()
    return body[position:]


def workflow_bodies(workflows: Path) -> dict[str, str]:
    """Every `run:` body under `workflows`, keyed by workflow, job and step."""
    bodies: dict[str, str] = {}
    for path in sorted(workflows.glob("*.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for job, definition in (document.get("jobs") or {}).items():
            for index, step in enumerate(definition.get("steps") or []):
                if not isinstance(step, dict) or not step.get("run"):
                    continue
                # The index is in the key because step names are not unique
                # within a job. Keying on the name alone silently dropped five
                # of 184 steps into a dict collision -- a linter quietly not
                # reading five release steps, which is the exact fail-open
                # shape this suite exists to catch.
                name = f"{path.name}:{job}:{index}:{step.get('name', '<unnamed>')}"
                bodies[name] = EXPRESSION.sub(EXPRESSION_PLACEHOLDER, str(step["run"]))
    return bodies


def dockerfile_bodies(checked_in: Path, templates: Path, render) -> dict[str, str]:
    """Every `RUN` body from the Dockerfiles that are actually built.

    `render` is the same entry point the image build uses, called per template
    and architecture. Templates are rendered, never masked: a masked
    `{{ apt_packages | join }}` is a fiction, and linting a fiction reports on
    a file nobody builds.
    """
    bodies: dict[str, str] = {}
    for path in sorted(checked_in.glob("Dockerfile*")):
        for index, run in enumerate(run_instructions(path.read_text(encoding="utf-8"))):
            bodies[f"{path.name}:RUN[{index}]"] = EXPRESSION.sub(EXPRESSION_PLACEHOLDER, run)

    for template, arch, rendered in render(templates):
        for index, run in enumerate(run_instructions(rendered)):
            bodies[f"{template}:{arch}:RUN[{index}]"] = run
    return bodies


def rendered_templates(templates: Path, guest_config: Path):
    """Yield `(template, architecture, rendered)` for every guest template."""
    from ..image.config import load_guest_config
    from ..image.docker import render_dockerfile

    guest = load_guest_config(guest_config)
    for template in sorted(path.name for path in templates.glob("*.j2")):
        for arch in guest.build.architectures:
            yield template, arch, render_dockerfile(template, guest, arch)

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
from pathlib import Path

import yaml

#: A GitHub expression. The runner substitutes each as one value before bash
#: sees it, so it masks to a variable reference: masking to a literal makes
#: `[ "$X" = "y" ]` a constant comparison and ShellCheck rightly says SC2050.
EXPRESSION = re.compile(r"\$\{\{\s*.*?\s*\}\}", re.S)
EXPRESSION_PLACEHOLDER = "${CAPSEM_GH_EXPRESSION}"

RUN_INSTRUCTION = re.compile(r"^RUN\s+((?:.*\\\n)*.*)", re.MULTILINE)


def executable_lines(body: str) -> list[str]:
    """The lines of `body` that do something: no blanks, no comments."""
    return [line for line in body.splitlines() if line.strip() and not line.lstrip().startswith("#")]


def _without_comments(text: str) -> str:
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


def run_instructions(dockerfile: str) -> list[str]:
    """Every RUN body in a Dockerfile, the way Docker parses one.

    Comments are removed *before* continuations are joined, which is the order
    Docker uses. Reversing it leaves the `\\` preceding a comment line dangling
    and reports a syntax error in a RUN that builds correctly.

    Continuations are otherwise left intact: ShellCheck reads them, and
    flattening pulls a trailing comment into the middle of a logical line.
    """
    return RUN_INSTRUCTION.findall(_without_comments(dockerfile))


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
    from capsem.builder.config import load_guest_config
    from capsem.builder.docker import render_dockerfile

    guest = load_guest_config(guest_config)
    for template in sorted(path.name for path in templates.glob("*.j2")):
        for arch in guest.build.architectures:
            yield template, arch, render_dockerfile(template, guest, arch)

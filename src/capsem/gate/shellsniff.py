"""Whether the text handed to the shell tools is shell at all.

Its own module because deciding what something *is* is not lexing it, and
because the mistake it catches is made by callers rather than by the lexer: a
raw `.j2` template, a whole Dockerfile, a workflow. Each lexes without error
and yields confident nonsense, which is the worst failure mode a tool like this
has -- worse than an exception, because the guard reading the result reports a
clean tree.
"""

from __future__ import annotations

import re

#: Markers that say the text handed to the lexer is not shell.
#:
#: Deliberately narrow. Every one of these is unambiguous, because a sniffer
#: that guesses is worse than none: `docker inspect --format '{{range
#: .Mounts}}'` is a Go template inside perfectly good shell, so `{{` cannot be
#: a Jinja signal. `{%` can.
FOREIGN = (
    ("jinja", re.compile(r"\{%-?\s*(?:if|for|set|block|macro|endif|endfor)\b")),
    ("dockerfile", re.compile(r"^FROM\s+\S+", re.MULTILINE)),
    # No Python detector. Eight tracked scripts embed a Python heredoc, so
    # `import x` at line start is shell here -- the sniffer cannot see heredoc
    # boundaries because it runs before lexing. A signal that fires on eight
    # correct files is a signal people turn off.
    ("yaml", re.compile(r"^(?:jobs|steps|runs-on|on):\s*$", re.MULTILINE)),
)


class ForeignSourceWarning(UserWarning):
    """The lexer was handed something that is not shell."""


def sniff(source: str) -> str | None:
    """The non-shell language this text appears to be, if it obviously is one.

    Cheap, and only confident cases. Its whole job is to catch the mistake of
    passing a *container* of shell where a shell body was wanted -- a raw `.j2`
    template instead of its rendered output, a whole Dockerfile instead of one
    `RUN` body, a workflow instead of a `run:` block. Each of those lexes
    without error and produces confident nonsense, which is the worst failure
    mode a tool like this has.
    """
    for name, pattern in FOREIGN:
        if pattern.search(source):
            return name
    return None

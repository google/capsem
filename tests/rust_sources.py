"""Reading Rust sources that follow the sibling-`tests.rs` layout.

`CLAUDE.md` requires every `#[test]` to live in a sibling `tests.rs` that the
parent declares with `#[cfg(test)] mod tests;`. Source-contract tests that
assert a Rust test *exists* must therefore read that sibling, not the
production file it was moved out of.

The two sources stay separate on purpose. Several contracts assert that a
symbol is *absent* from production code -- `Removed` is not a `Status`
variant, `pub package: String` is not a field. A test module legitimately
names the thing it proves is rejected, so concatenating the two would let a
test fixture falsify a claim about production code. `production()` answers
"what does the shipped code say", `sibling_tests()` answers "what does its
test module prove", and no assertion has to guess which it got.

Neither public name may begin with `test`: pytest's default collection prefix
is `test*`, so a helper called `tests_of` gets collected and reported as a
failing test in every module that imports it by name.
"""

from __future__ import annotations

import re
from pathlib import Path

# Files that are themselves a module root, so `mod tests;` resolves beside
# them rather than in a subdirectory named after them.
_MODULE_ROOTS = {"main.rs", "lib.rs", "mod.rs"}


def production(path: Path) -> str:
    """The production source alone, with no test module mixed in."""
    return path.read_text(encoding="utf-8")


def sibling_tests_path(path: Path) -> Path:
    """Where `path`'s `mod tests;` resolves, following Rust's own rules."""
    if path.name in _MODULE_ROOTS:
        return path.parent / "tests.rs"
    return path.parent / path.stem / "tests.rs"


def sibling_tests(path: Path) -> str:
    """The source of `path`'s sibling test module.

    Fails loudly rather than returning an empty string: a contract asserting
    that some Rust test exists must not quietly pass because the module it
    should have searched was missing.
    """
    module = sibling_tests_path(path)
    if not module.is_file():
        raise AssertionError(
            f"{path} declares tests that should live in {module}, "
            f"but that file does not exist -- see the sibling-tests.rs "
            f"layout rule in CLAUDE.md"
        )
    return module.read_text(encoding="utf-8")


def masked_code(source: str) -> str:
    """Mask Rust comments and literals, preserving source offsets and newlines."""
    start = re.compile(
        r'//|/\*|(?:br|r)(?P<hashes>\#*)"|b?"|'
        r"b?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\\'])'",
        re.S,
    )
    chars = list(source)
    offset = 0
    while match := start.search(source, offset):
        token = match.group()
        if token == "//":
            end = source.find("\n", match.end())
            end = len(source) if end < 0 else end
        elif token == "/*":
            depth, end = 1, match.end()
            while depth:
                nested = re.search(r"/\*|\*/", source[end:])
                if nested is None:
                    raise ValueError("unclosed Rust comment")
                depth += 1 if nested.group() == "/*" else -1
                end += nested.end()
        elif match.group("hashes") is not None:
            closing = '"' + match.group("hashes")
            end = source.find(closing, match.end())
            if end < 0:
                raise ValueError("unclosed Rust raw string")
            end += len(closing)
        elif token.endswith('"'):
            closing = re.match(r'(?:\\.|[^"\\])*"', source[match.end() :], re.S)
            if closing is None:
                raise ValueError("unclosed Rust string")
            end = match.end() + closing.end()
        else:
            end = match.end()
        chars[match.start() : end] = [
            "\n" if char == "\n" else " " for char in source[match.start() : end]
        ]
        offset = end
    return "".join(chars)

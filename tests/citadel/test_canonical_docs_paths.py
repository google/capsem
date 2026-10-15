"""The retired-root guard must distinguish its approved web owner."""

from tests.citadel.test_web_benchmark_boundary import _legacy_references


def test_canonical_docs_paths_are_owned_and_retired_roots_still_fail() -> None:
    assert not _legacy_references({"example.md": "web/docs/src/content/guide.md"})
    assert _legacy_references({"example.md": "docs/guide.md"})
    assert _legacy_references({"example.md": "../docs/guide.md"})

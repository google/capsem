"""Only the exact native release may publish its dependency-ordered clients."""

from pathlib import Path

import pytest
from capsem_builder.gate.config import load
from capsem_builder.release.tools import client_publishers
from capsem_builder.release.tools.client_package_archives import Package
from capsem_builder.release.tools.client_publishers import (
    require_credentials,
    require_source_context,
    validate_order,
)

SOURCE = "a" * 40


@pytest.mark.parametrize("missing", [None, "PYPI_TOKEN", "NPM_TOKEN", "CARGO_REGISTRY_TOKEN"])
def test_missing_publisher_is_reported_by_name_without_secret_values(missing: str | None) -> None:
    environment = dict.fromkeys(("PYPI_TOKEN", "NPM_TOKEN", "CARGO_REGISTRY_TOKEN"), "test-secret-value")
    if missing:
        environment[missing] = ""
        with pytest.raises(ValueError, match=missing) as error:
            require_credentials(environment)
        assert "test-secret-value" not in str(error.value)
    else:
        require_credentials(environment)


@pytest.mark.parametrize("mutation", [None, "source", "ref", "repository", "head", "tag", "dirty"])
def test_only_clean_exact_immutable_source_context_is_accepted(mutation: str | None) -> None:
    environment = {"GITHUB_SHA": SOURCE, "GITHUB_REF": f"refs/tags/capsem-source-{SOURCE}", "GITHUB_REPOSITORY": "google/capsem"}
    head, tag, dirty = SOURCE, SOURCE, False
    if mutation in {"source", "ref", "repository"}:
        environment[{"source": "GITHUB_SHA", "ref": "GITHUB_REF", "repository": "GITHUB_REPOSITORY"}[mutation]] = "wrong"
    elif mutation == "head":
        head = "b" * 40
    elif mutation == "tag":
        tag = "b" * 40
    elif mutation == "dirty":
        dirty = True
    if mutation:
        with pytest.raises(ValueError):
            require_source_context(SOURCE, environment, head, tag, dirty)
    else:
        require_source_context(SOURCE, environment, head, tag, dirty)


@pytest.mark.parametrize("mutation", [None, "reverse", "missing", "version", "duplicate"])
def test_a_consumer_never_publishes_ahead_of_its_exact_dependency(mutation: str | None) -> None:
    sdk = Package("npm", "@capsem/sdk", "0.7.1", "sdk/typescript", (Path("sdk.tgz"),))
    mcp = Package("npm", "@capsem/mcp", "0.7.1", "mcp/typescript", (Path("mcp.tgz"),), ((sdk.name, sdk.version),))
    packages = [sdk, mcp]
    if mutation == "reverse":
        packages.reverse()
    elif mutation == "missing":
        packages.remove(sdk)
    elif mutation == "version":
        packages[0] = Package(sdk.registry, sdk.name, "0.7.0", sdk.project, sdk.archives)
    elif mutation == "duplicate":
        packages.append(sdk)
    if mutation:
        with pytest.raises(ValueError):
            validate_order(tuple(packages))
    else:
        validate_order(tuple(packages))


@pytest.mark.parametrize("matches", [True, False])
def test_cargo_cannot_upload_an_archive_other_than_the_verified_candidate(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, matches: bool) -> None:
    archive = tmp_path / "prepared" / "sdk.crate"
    archive.parent.mkdir()
    archive.write_bytes(b"qualified")
    target = tmp_path / "cargo" / "package" / archive.name
    target.parent.mkdir(parents=True)
    package = Package("crates", "capsem-sdk", "0.7.1", "sdk/rust", (archive,))
    commands = []

    def run(_config, argv, environment):
        commands.append(argv)
        if argv[1] == "package":
            assert "--no-verify" not in argv
            assert "CARGO_REGISTRY_TOKEN" not in environment
            target.write_bytes(archive.read_bytes() if matches else b"different")
        else:
            assert argv[1] == "publish"
            assert environment["CARGO_REGISTRY_TOKEN"] == "test-credential"

    monkeypatch.setattr(client_publishers, "_run", run)
    monkeypatch.setattr(client_publishers, "cargo_metadata", lambda _: {"target_directory": str(target.parent.parent)})
    config = load(Path(__file__).resolve().parents[3]).model_copy(update={"root": tmp_path})
    arguments = (config, package, [archive], {"CARGO_REGISTRY_TOKEN": "test-credential"})
    if matches:
        client_publishers._upload(*arguments)
        assert len(commands) == 2
    else:
        with pytest.raises(ValueError, match="before upload"):
            client_publishers._upload(*arguments)
        assert len(commands) == 1

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


@pytest.mark.parametrize("missing", [None, "PYPI_TOKEN"])
def test_missing_publisher_is_reported_by_name_without_secret_values(missing: str | None) -> None:
    environment = {"PYPI_TOKEN": "test-secret-value"}
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


def test_trusted_npm_upload_preserves_oidc_and_discards_registry_tokens(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    package = Package("npm", "@capsem/sdk", "0.7.1", "sdk/typescript", (tmp_path / "sdk.tgz",))
    commands = []
    def run(_config, argv, environment):
        commands.append((argv, environment))
    monkeypatch.setattr(client_publishers, "_run", run)
    environment = {"GITHUB_ACTIONS": "true", "ACTIONS_ID_TOKEN_REQUEST_URL": "https://example.invalid/oidc",
                   "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "ephemeral", "PYPI_TOKEN": "private",
                   "NPM_TOKEN": "private", "CARGO_REGISTRY_TOKEN": "private", "NODE_AUTH_TOKEN": "private"}
    config = load(Path(__file__).resolve().parents[3])
    client_publishers._upload(config, package, list(package.archives), environment)
    assert len(commands) == 1
    argv, child = commands[0]
    assert argv[:2] == ["npm", "publish"]
    assert "--provenance" in argv and "--ignore-scripts" in argv
    assert child["ACTIONS_ID_TOKEN_REQUEST_TOKEN"] == "ephemeral"
    assert not {"PYPI_TOKEN", "NPM_TOKEN", "CARGO_REGISTRY_TOKEN", "NODE_AUTH_TOKEN"} & child.keys()


def test_cargo_is_deferred_before_any_registry_access(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    package = Package("crates", "capsem-sdk", "0.7.1", "sdk/rust", (tmp_path / "sdk.crate",))
    def forbidden(*_args):
        pytest.fail("Deferred Cargo publication must not access a registry or run a command")
    monkeypatch.setattr(client_publishers, "inspect", forbidden)
    monkeypatch.setattr(client_publishers, "_run", forbidden)
    client_publishers.publish(load(Path(__file__).resolve().parents[3]), (package,), {"PYPI_TOKEN": "fixture"})


@pytest.mark.parametrize("missing", ["GITHUB_ACTIONS", "ACTIONS_ID_TOKEN_REQUEST_URL", "ACTIONS_ID_TOKEN_REQUEST_TOKEN"])
def test_npm_upload_requires_trusted_ci_context(tmp_path: Path, missing: str) -> None:
    package = Package("npm", "@capsem/sdk", "0.7.1", "sdk/typescript", (tmp_path / "sdk.tgz",))
    environment = {"GITHUB_ACTIONS": "true", "ACTIONS_ID_TOKEN_REQUEST_URL": "https://example.invalid/oidc",
                   "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "ephemeral"}
    environment.pop(missing)
    with pytest.raises(ValueError, match="OIDC"):
        client_publishers._upload(load(Path(__file__).resolve().parents[3]), package, list(package.archives), environment)

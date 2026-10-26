"""Pinned external tools are one verified cache primitive."""

from __future__ import annotations

import hashlib
from pathlib import Path

import pytest
from capsem_builder.cache.contract import CacheScope, PruneStrategy
from capsem_builder.cache.models import CachePolicy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.tools import CachedToolPolicy, ToolDistribution, materialize

PAYLOAD = b"verified executable"


def _paths(tmp_path: Path) -> CachePaths:
    return CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(
            version=1,
            root=Path("cache"),
            authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
            stages={
                "toolchain": StagePolicy(
                    path=Path("target/toolchain"),
                    description="test tools",
                    scope=CacheScope.DISK,
                    warm_size_bytes=1,
                    max_size_bytes=2,
                    prune_strategy=PruneStrategy.LRU,
                    maximum_age_hours=1,
                )
            },
        ),
    )


def _tool(sha256: str | None = None) -> CachedToolPolicy:
    return CachedToolPolicy(
        name="scanner",
        version="1.2.3",
        cache_stage="toolchain",
        download_timeout_seconds=10,
        distributions=(
            ToolDistribution(
                system="Linux",
                machine="x86_64",
                url="https://example.test/scanner",
                sha256=sha256 or hashlib.sha256(PAYLOAD).hexdigest(),
            ),
        ),
    )


def test_materialize_verifies_once_then_reuses_exact_binary(tmp_path: Path) -> None:
    calls: list[str] = []

    def download(url: str, destination: Path, _timeout: int) -> None:
        calls.append(url)
        destination.write_bytes(PAYLOAD)

    first = materialize(
        _paths(tmp_path), _tool(), system="Linux", machine="x86_64", download=download
    )
    second = materialize(
        _paths(tmp_path), _tool(), system="Linux", machine="x86_64", download=download
    )

    assert not first.cache_hit
    assert second.cache_hit
    assert first.path == second.path
    assert first.path.read_bytes() == PAYLOAD
    assert first.path.stat().st_mode & 0o777 == 0o555
    assert calls == ["https://example.test/scanner"]


def test_materialize_never_publishes_wrong_digest(tmp_path: Path) -> None:
    def download(_url: str, destination: Path, _timeout: int) -> None:
        destination.write_bytes(b"tampered")

    with pytest.raises(ValueError, match="checksum mismatch"):
        materialize(
            _paths(tmp_path),
            _tool(),
            system="Linux",
            machine="x86_64",
            download=download,
        )

    assert [path for path in (tmp_path / "cache").rglob("scanner") if path.is_file()] == []


def test_tool_policy_rejects_duplicate_platform_distributions() -> None:
    distribution = _tool().distributions[0]

    with pytest.raises(ValueError, match="unique by system and machine"):
        CachedToolPolicy.model_validate(
            _tool().model_dump() | {"distributions": (distribution, distribution)}
        )


def _archive_policy(data: bytes, *, member: str = "scanner", digest: str | None = None):
    distribution = _tool().distributions[0].model_dump() | {
        "archive_sha256": digest or hashlib.sha256(data).hexdigest(),
        "archive_member": member,
    }
    return CachedToolPolicy.model_validate(_tool().model_dump() | {"distributions": [distribution]})


def _archive(*, kind: bytes | None = None, payload: bytes = PAYLOAD) -> bytes:
    import io
    import tarfile

    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as package:
        member = tarfile.TarInfo("scanner")
        if kind is not None:
            member.type = kind
            member.linkname = "elsewhere"
            package.addfile(member)
        else:
            member.size = len(payload)
            package.addfile(member, io.BytesIO(payload))
    return output.getvalue()


@pytest.mark.parametrize("failure", [None, "archive", "executable", "symlink", "hardlink"])
def test_archive_tool_checks_both_pins_and_regular_member(tmp_path, failure):
    import tarfile

    kind = {"symlink": tarfile.SYMTYPE, "hardlink": tarfile.LNKTYPE}.get(failure)
    data = _archive(kind=kind, payload=b"tampered" if failure == "executable" else PAYLOAD)
    policy = _archive_policy(data, digest="0" * 64 if failure == "archive" else None)
    calls = []

    def download(_url, destination, _timeout):
        calls.append(destination)
        destination.write_bytes(data)

    def resolve():
        return materialize(
            _paths(tmp_path), policy, system="Linux", machine="x86_64", download=download
        )

    if failure:
        with pytest.raises(ValueError, match=r"checksum mismatch|regular file"):
            resolve()
        assert not [path for path in (tmp_path / "cache").rglob("scanner") if path.is_file()]
    else:
        result = resolve()
        assert result.path.read_bytes() == PAYLOAD
        assert resolve().cache_hit
        assert len(calls) == 1
    assert not list((tmp_path / "cache").rglob(".*"))


@pytest.mark.parametrize(
    "member", ["../scanner", "/scanner", "a/../../scanner", "a\\scanner", "", "./scanner"]
)
def test_archive_policy_rejects_unsafe_member(member):
    with pytest.raises(ValueError, match="safe relative path"):
        _archive_policy(b"archive", member=member)


def test_archive_policy_requires_both_pin_and_member():
    with pytest.raises(ValueError, match="supplied together"):
        ToolDistribution.model_validate(
            _tool().distributions[0].model_dump() | {"archive_member": "scanner"}
        )

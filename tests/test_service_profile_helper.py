"""Contracts for the integration-test service helper."""

from pathlib import Path

import pytest

from tests.helpers import service as service_helper


def test_materialize_test_profiles_rejects_empty_generated_catalog(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    generated = tmp_path / "generated" / "profiles"
    generated.mkdir(parents=True)
    monkeypatch.setattr(service_helper, "PROFILES_DIR", generated)

    with pytest.raises(RuntimeError, match="contains no profile.toml"):
        service_helper.materialize_test_profiles(tmp_path / "run")


def test_materialize_test_profiles_copies_real_generated_profiles(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    generated = tmp_path / "generated" / "profiles"
    (generated / "code").mkdir(parents=True)
    (generated / "code" / "profile.toml").write_text('id = "code"\n')
    monkeypatch.setattr(service_helper, "PROFILES_DIR", generated)

    copied = service_helper.materialize_test_profiles(tmp_path / "run")

    assert (copied / "code" / "profile.toml").read_text() == 'id = "code"\n'

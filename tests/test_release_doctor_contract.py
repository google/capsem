"""Release doctor contract tests."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(
        i for i, line in enumerate(lines) if line == name or line.startswith(f"{name} ")
    )
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def test_smoke_runs_full_doctor_without_fast_escape_hatch() -> None:
    block = _recipe_block("smoke:")

    assert "{{cli_binary}} doctor" in block
    assert "doctor --fast" not in block
    assert "{{cli_binary}} doctor --fast" not in block


def test_guest_network_doctor_is_hermetic_by_default() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "CAPSEM_RUN_PUBLIC_NETWORK_SMOKE" not in source
    assert "google.com" not in source
    assert "api.openai.com" not in source
    assert "api.anthropic.com" not in source
    assert "cdn.elie.net" not in source


def test_doctor_session_validation_starts_hermetic_upstream() -> None:
    source = (PROJECT_ROOT / "scripts" / "doctor_session_test.py").read_text()

    assert "capsem-debug-upstream" in source
    assert "CAPSEM_BENCH_MITM_LOCAL_BASE_URL" in source
    assert "[binary, \"run\", \"capsem-doctor\"]" in source


def test_guest_init_exports_ca_bundle_for_runtime_and_login_shells() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    expected = {
        "SSL_CERT_FILE": "/etc/ssl/certs/ca-certificates.crt",
        "REQUESTS_CA_BUNDLE": "/etc/ssl/certs/ca-certificates.crt",
        "NODE_EXTRA_CA_CERTS": "/etc/ssl/certs/ca-certificates.crt",
    }

    runtime_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[0]
    profile_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[1]

    for key, value in expected.items():
        export = f"export {key}={value}"
        assert export in runtime_block
        assert export in profile_block

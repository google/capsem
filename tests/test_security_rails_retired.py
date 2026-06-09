from pathlib import Path


PROJECT_ROOT = Path(__file__).parent.parent


def _text(path):
    return path.read_text(errors="ignore")


def test_retired_policy_v2_and_mcp_decision_rails_stay_absent():
    live_roots = [
        PROJECT_ROOT / "crates",
        PROJECT_ROOT / "config",
    ]
    banned_symbols = [
        "LocalMcpDecisionProvider",
        "McpPolicy",
        "legacy_decision",
        "policy_v2_http_hook",
        "evaluate_model_request_policy",
        "evaluate_model_response_policy",
    ]
    offenders = []
    for root in live_roots:
        for path in root.rglob("*"):
            if path.is_dir() or path.suffix not in {".rs", ".toml", ".yaml", ".yml"}:
                continue
            text = _text(path)
            for symbol in banned_symbols:
                if symbol in text:
                    offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {symbol}")

    assert offenders == []


def test_policy_v2_and_domain_policy_source_files_stay_deleted():
    deleted_paths = [
        "crates/capsem-core/src/net/mitm_proxy/policy_v2_model.rs",
        "crates/capsem-core/src/net/mitm_proxy/policy_v2_http_hook.rs",
        "crates/capsem-core/src/net/domain_policy.rs",
        "crates/capsem-network-engine/src/domain_policy.rs",
        "crates/capsem-network-engine/src/http_policy.rs",
        "crates/capsem-network-engine/src/mcp_security.rs",
        "crates/capsem-network-engine/src/model_security.rs",
    ]
    existing = [path for path in deleted_paths if (PROJECT_ROOT / path).exists()]
    assert existing == []


def test_old_policy_authoring_is_not_live_configuration():
    live_config = [
        PROJECT_ROOT / "config",
    ]
    offenders = []
    for root in live_config:
        for path in root.rglob("*"):
            if path.is_dir() or path.suffix not in {".toml", ".yaml", ".yml"}:
                continue
            text = _text(path)
            for old_prefix in ("[policy.http", "[policy.mcp", "[policy.model"):
                if old_prefix in text:
                    offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {old_prefix}")

    assert offenders == []

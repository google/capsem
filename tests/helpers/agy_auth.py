"""The existing synthetic AGY consumer-session setup, shared by native proofs.

Only a fresh test workload home receives these synthetic values. This does
not exercise browser enrollment, a host keyring, refresh or vendor access.
"""

import textwrap


def synthetic_home_setup() -> str:
    """Python fragment with existing HOME, WORKSPACE, NONCE and Path/json names."""
    return textwrap.dedent('''\
token_path = Path(HOME + "/.gemini/antigravity-cli/antigravity-oauth-token")
token_path.parent.mkdir(parents=True, exist_ok=True)
token_path.write_text(json.dumps({
    "token": {
        "access_token": "capsem_test_agy_access_" + NONCE,
        "token_type": "Bearer",
        "refresh_token": "capsem_test_agy_refresh_" + NONCE,
        "expiry": "2099-01-01T00:00:00Z"
    },
    "auth_method": "consumer"
}), encoding="utf-8")
token_path.chmod(0o600)
settings_path = Path(HOME + "/.gemini/antigravity-cli/settings.json")
agy_model_settings = {
    "trustedWorkspaces": [WORKSPACE],
    "telemetry": {"enabled": False},
    "autoUpdate": {"enabled": False}
}
settings_path.write_text(json.dumps(agy_model_settings), encoding="utf-8")
''')

"""Automatic host credential brokering for `CapsemEnvironment.start()` and `exec()`."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any
from urllib.parse import urlsplit

from capsem_integrations import _controller as _ctrl_mod

_CREDENTIAL_REF_PREFIX = "credential:blake3:"
_RUNTIME_CA_BUNDLE = "/etc/ssl/certs/ca-certificates.crt"
_RUNTIME_CA_ENV: dict[str, str] = {
    "SSL_CERT_FILE": _RUNTIME_CA_BUNDLE,
    "REQUESTS_CA_BUNDLE": _RUNTIME_CA_BUNDLE,
    "PIP_CERT": _RUNTIME_CA_BUNDLE,
    "CURL_CA_BUNDLE": _RUNTIME_CA_BUNDLE,
    "GIT_SSL_CAINFO": _RUNTIME_CA_BUNDLE,
    "NODE_EXTRA_CA_CERTS": _RUNTIME_CA_BUNDLE,
    "UV_NATIVE_TLS": "1",
    "TMPDIR": "/var/tmp",
}
_PROVIDER_KEY_SPECS: dict[str, str] = {
    "ANTHROPIC_API_KEY": "anthropic",
    "OPENAI_API_KEY": "openai",
    "GEMINI_API_KEY": "google",
    "GOOGLE_API_KEY": "google",
    "VERTEX_API_KEY": "google",
    "GOOGLE_VERTEX_API_KEY": "google",
    "GOOGLE_OAUTH_ACCESS_TOKEN": "google",
    "CLOUDSDK_AUTH_ACCESS_TOKEN": "google",
}
_AIDER_PROVIDER_MAP: dict[str, str] = {
    "anthropic": "anthropic",
    "openai": "openai",
    "gemini": "google",
    "google": "google",
    "vertex_ai": "google",
}


def truncate_label_value(value: str, max_bytes: int = 255) -> str:
    """Truncate `value` to at most `max_bytes` UTF-8 bytes without splitting codepoints."""
    if callable(fn := getattr(_ctrl_mod, "truncate_label_value", None)):
        return str(fn(value, max_bytes))
    return value.encode("utf-8")[:max_bytes].decode("utf-8", errors="ignore")


def coerce_bool_flag(value: Any, *, default: bool) -> bool:
    """Coerce a boolean or `--ek key=true|false` CLI string into a `bool`."""
    if value is None:
        return default
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return bool(value)
    if isinstance(value, str):
        norm = value.strip().lower()
        if not norm:
            return default
        if norm in ("1", "true", "yes", "on"):
            return True
        if norm in ("0", "false", "no", "off"):
            return False
    raise ValueError(f"Invalid boolean option value: {value!r}")


def should_broker_credential(raw_value: str) -> bool:
    """Return True when `raw_value` should be registered with the Capsem credential broker."""
    if not isinstance(raw_value, str) or not (stripped := raw_value.strip()):
        return False
    return not stripped.startswith(_CREDENTIAL_REF_PREFIX)


async def _inject_credential(controller: Any, provider: str, raw_value: str) -> str:
    inject_fn = getattr(controller, "inject_credential", None)
    if callable(inject_fn):
        resp = await inject_fn(provider, raw_value, storage="memory")
    else:
        hv = getattr(controller, "hypervisor", None) or getattr(controller, "_hypervisor", None)
        creds = getattr(hv, "credentials", None) if hv is not None else None
        if creds is None or not callable(getattr(creds, "inject", None)):
            raise RuntimeError("Capsem controller does not expose hypervisor.credentials.inject")
        resp = await creds.inject(provider, raw_value, storage="memory")
    ref = getattr(resp, "credential_ref", resp)
    if not isinstance(ref, str) or not ref.startswith(_CREDENTIAL_REF_PREFIX):
        raise ValueError(f"Invalid credential reference returned for {provider}: {ref!r}")
    return ref


def _host_is_googleapis(url: str) -> bool:
    host = (urlsplit(url.strip()).hostname or "").strip().lower().rstrip(".")
    return host == "googleapis.com" or host.endswith(".googleapis.com")


def _infer_provider(
    provider: str,
    cleaned: str,
    env: Mapping[str, str] | None = None,
) -> str:
    if cleaned.startswith(("ya29.", "AIza")):
        return "google"
    if cleaned.startswith("sk-ant-"):
        return "anthropic"
    if env is not None:
        base_keys: tuple[str, ...] = ()
        if provider == "openai":
            base_keys = ("OPENAI_API_BASE", "OPENAI_BASE_URL")
        elif provider == "anthropic":
            base_keys = ("ANTHROPIC_BASE_URL", "ANTHROPIC_VERTEX_BASE_URL")
        for key in base_keys:
            if isinstance(url := env.get(key), str) and _host_is_googleapis(url):
                return "google"
    return provider


async def _resolve_cred_ref(
    controller: Any,
    provider: str,
    raw_value: str,
    cache: dict[tuple[str, str], str] | None,
    env: Mapping[str, str] | None = None,
) -> str:
    cleaned = raw_value.strip()
    provider = _infer_provider(provider, cleaned, env)
    cache_key = (provider, cleaned)
    if cache is not None and cache_key in cache:
        return cache[cache_key]
    ref = await _inject_credential(controller, provider, cleaned)
    if cache is not None:
        cache[cache_key] = ref
    return ref


async def broker_env_credentials(
    controller: Any,
    env: Mapping[str, str],
    *,
    enabled: bool = True,
    cache: dict[tuple[str, str], str] | None = None,
) -> dict[str, str]:
    """Rewrite raw provider API keys in `env` into `credential:blake3:<hex>` references."""
    out = dict(env)
    if not enabled:
        return out
    for env_key, provider in _PROVIDER_KEY_SPECS.items():
        raw_value = out.get(env_key)
        if not isinstance(raw_value, str):
            continue
        stripped = raw_value.strip()
        if not stripped:
            continue
        if stripped.startswith(_CREDENTIAL_REF_PREFIX):
            out[env_key] = stripped
            continue
        out[env_key] = await _resolve_cred_ref(controller, provider, stripped, cache, out)
    if isinstance(aider_raw := out.get("AIDER_API_KEY"), str) and aider_raw.strip():
        parts: list[str] = []
        for item in aider_raw.split(","):
            stripped_item = item.strip()
            if not stripped_item:
                continue
            if "=" not in stripped_item:
                raise ValueError("Malformed AIDER_API_KEY entry; expected '<provider>=<key>'.")
            prov_key, secret = stripped_item.split("=", 1)
            norm_prov = prov_key.strip().lower()
            clean_secret = secret.strip()
            prov = _AIDER_PROVIDER_MAP.get(norm_prov)
            if not should_broker_credential(clean_secret):
                if clean_secret:
                    parts.append(f"{prov_key.strip()}={clean_secret}")
                continue
            if prov is None:
                raise ValueError(
                    f"Unsupported AIDER_API_KEY provider {prov_key.strip()!r}; "
                    f"supported providers: {', '.join(sorted(_AIDER_PROVIDER_MAP))}."
                )
            ref = await _resolve_cred_ref(controller, prov, clean_secret, cache, out)
            parts.append(f"{prov_key.strip()}={ref}")
        out["AIDER_API_KEY"] = ",".join(parts)
    return out


__all__ = [
    "_CREDENTIAL_REF_PREFIX",
    "_PROVIDER_KEY_SPECS",
    "_RUNTIME_CA_ENV",
    "broker_env_credentials",
    "coerce_bool_flag",
    "should_broker_credential",
    "truncate_label_value",
]

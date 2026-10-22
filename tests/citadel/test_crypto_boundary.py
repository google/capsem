"""Cryptographic primitives have one owner, including build and test callers."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
OWNER = Path("crates/capsem-crypto")
PROVIDERS = frozenset(
    [
        "sha1",
        "sha2",
        "sha3",
        "blake2",
        "blake2b_simd",
        "blake2s_simd",
        "blake3",
        "xxhash-rust",
        "digest",
        "hmac",
        "ring",
        "aws-lc-rs",
        "aws-lc-sys",
        "openssl",
        "openssl-sys",
        "rcgen",
        "jsonwebtoken",
        "aes",
        "aes-gcm",
        "chacha20poly1305",
        "ed25519-dalek",
        "rsa",
        "p256",
        "p384",
        "k256",
        "rand",
        "rand_core",
        "getrandom",
        "subtle",
        "zeroize",
        "signature",
        "argon2",
        "scrypt",
        "pbkdf2",
    ]
)
CRYPTO_BOUNDARY_RATIONALE = """\
Use capsem-crypto for cryptographic operations. Per-call-site providers have
repeatedly selected slow hashing implementations and debug profiles on the
boot path. Provider selection, acceleration, randomness, constant-time checks,
and key operations belong to one tested owner, just like foundation's Unix
boundary. Protocol identities remain the caller's contract. See AGENTS.md.
"""
NAMES = "|".join(re.escape(name.replace("-", "_")) for name in sorted(PROVIDERS))
RAW_PATH = re.compile(
    rf"\b(?:{NAMES})\s*::|::\s*crypto\b|::\s*new_v4\s*\(|\b(?:EVP_\w+|SHA256|SHA1|HMAC|RAND_bytes)\s*\("
)
IMPORT = re.compile(r"\b(?:use|extern\s+crate)\b[^;]*;", re.DOTALL)
RAW_IMPORT = re.compile(rf"\b(?:{NAMES})\b")


def raw_operations(source: str) -> list[str]:
    # Comments cannot split imports into an invisible dependency. Keep strings:
    # macro-generated provider paths and source inclusion must be visible too.
    source = re.sub(r"/\*.*?\*/", " ", source, flags=re.DOTALL)
    source = re.sub(r"//[^\n]*", "", source)
    # These are owned APIs, not access to a provider. Every other spelling is
    # checked, including grouped imports, absolute paths, and aliases.
    source = re.sub(r"\bcapsem_crypto\s*::\s*blake3\b", "owned_blake3", source)
    source = IMPORT.sub(
        lambda match: (
            ""
            if re.match(r"use\s+capsem_crypto\s*::", match.group())
            else match.group()
        ),
        source,
    )
    imports = [
        m.group()
        for m in IMPORT.finditer(source)
        if (
            RAW_IMPORT.search(m.group())
            or ("rustls" in m.group() and re.search(r"\bcrypto\b", m.group()))
        )
        and not re.match(r"use\s+(?:crate|super|self|capsem_crypto)\s*::", m.group())
    ]
    bespoke = re.findall(r"\bDefaultHasher\b|\b0x(?:cbf29ce484222325|100000001b3)\b", source)
    return imports + [m.group() for m in RAW_PATH.finditer(source)] + bespoke


@pytest.mark.parametrize("source", [
    "let mut hash = std::collections::hash_map::DefaultHasher::new();",
    "let mut hash = 0xcbf29ce484222325_u64; hash = hash.wrapping_mul(0x100000001b3);",
])
def test_content_fingerprints_do_not_reimplement_or_select_a_hash(source: str) -> None:
    assert raw_operations(source), CRYPTO_BOUNDARY_RATIONALE


def provider_dependencies(manifest: dict, workspace: dict | None = None) -> list[str]:
    found = []
    for section, values in manifest.items():
        if not isinstance(values, dict):
            continue
        if section in {"dependencies", "dev-dependencies", "build-dependencies"}:
            for name, value in values.items():
                details = value if isinstance(value, dict) else {}
                if details.get("workspace"):
                    inherited = (workspace or {}).get(name, {})
                    details = inherited if isinstance(inherited, dict) else {}
                package = details.get("package", name)
                if package in PROVIDERS:
                    found.append(name)
        else:
            found.extend(provider_dependencies(values, workspace))
    return sorted(set(found))


def test_crypto_providers_are_owned_only_by_capsem_crypto() -> None:
    configuration = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    workspace = configuration.get("dependencies", {})
    members = [
        member for pattern in configuration["members"] for member in ROOT.glob(pattern)
    ]
    sources = [path for member in members for path in member.rglob("*.rs")]
    manifests = [path for member in members for path in member.rglob("Cargo.toml")]
    assert sources and manifests, CRYPTO_BOUNDARY_RATIONALE
    violations = []
    for path in sources:
        relative = path.relative_to(ROOT)
        if not relative.is_relative_to(OWNER) and raw_operations(path.read_text()):
            violations.append(str(relative))
    for path in manifests:
        relative = path.relative_to(ROOT)
        if not relative.is_relative_to(OWNER):
            raw = provider_dependencies(tomllib.loads(path.read_text()), workspace)
            if raw:
                violations.append(f"{relative}: {', '.join(raw)}")
    assert not violations, (
        CRYPTO_BOUNDARY_RATIONALE + "\n" + "\n".join(sorted(violations))
    )


@pytest.mark.parametrize(
    "source",
    [
        "use sha2::{Digest, Sha256};",
        "use ::ring as provider;",
        "extern crate blake3 as hash;",
        "use {sha2 as renamed};",
        "use {\n /* hide */\n sha2::{Sha256}\n};",
        "sha2 :: Sha256 :: digest(bytes);",
        "::blake3::hash(bytes);",
        "rustls::crypto::aws_lc_rs::default_provider();",
        "use rustls::{crypto::aws_lc_rs as backend};",
        "use rustls::crypto as provider;",
        "use rustls::{crypto as provider};",
        "use rustls as tls; tls::crypto::CryptoProvider::get_default();",
    "Uuid::new_v4();", "use uuid::Uuid as Id; Id::new_v4();",
        'extern "C" { fn EVP_DigestUpdate(context: *mut u8); }',
    ],
)
def test_crypto_guard_rejects_provider_paths_and_import_aliases(source: str) -> None:
    assert raw_operations(source), source


@pytest.mark.parametrize(
    "source",
    [
        'sha2 = "0.10"',
        "sha2.workspace = true",
        '[dependencies.sha2]\nversion = "0.10"',
        'backend = { package = "aws-lc-rs", version = "1" }',
    ],
)
def test_crypto_guard_rejects_renamed_and_table_dependencies(source: str) -> None:
    for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
        text = (
            source.replace("dependencies", kind)
            if source.startswith("[")
            else f"[{kind}]\n{source}"
        )
        assert provider_dependencies(tomllib.loads(text))
        assert provider_dependencies(tomllib.loads("[target.'cfg(unix)'." + text[1:]))


def test_crypto_guard_resolves_renamed_workspace_dependencies() -> None:
    manifest = {"dependencies": {"backend": {"workspace": True}}}
    assert provider_dependencies(
        manifest, {"backend": {"package": "ring", "version": "0.17"}}
    ) == ["backend"]


def test_crypto_guard_accepts_only_owned_hash_calls() -> None:
    assert not raw_operations(
        "use capsem_crypto::Sha256; capsem_crypto::blake3::hash(bytes);"
    )
    assert not raw_operations("// sha2::Sha256\nString::new();")

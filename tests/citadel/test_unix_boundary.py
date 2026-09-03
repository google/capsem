"""Citadel guard for the shared Unix primitive boundary.

Reusable host Unix behavior belongs in ``capsem-foundation::unix``.  The
inventories beside this test are exact, hashed transition debt and reviewed
domain-ABI ownership; neither is an exemption list that may grow unnoticed.
"""

from __future__ import annotations

import hashlib
import re
import tomllib
from collections.abc import Callable
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES = PROJECT_ROOT / "crates"
INVENTORY = Path(__file__).with_name("unix_boundary_debt.toml")

UNIX_BOUNDARY_RATIONALE = """\
Reusable host Unix operations must go through capsem-foundation::unix.

Scattered nix/libc calls have already produced double-close descriptor reuse,
EPERM-as-dead liveness bugs, fork/exec lock leaks, and swallowed cleanup
errors.  Foundation owns validated process identity, errno classification,
descriptor ownership, and race-safe locks.  Specialized kernel ABIs remain
beside their domain, but their exact raw-reference fingerprint is reviewed and
cannot grow silently.  See AGENTS.md, skills/dev-rust-patterns/SKILL.md, and
skills/citadel/SKILL.md.
"""

RAW_REFERENCE = re.compile(
    r"(?:\b(?:nix|libc)::|\b(?:use|extern\s+crate)\s+(?:nix|libc)\b)"
)
DIRECT_DEPENDENCY = re.compile(r"^\s*(nix|libc)\s*=", re.MULTILINE)

# These modules own a kernel or platform ABI rather than reusable Unix policy.
# Eligibility is exact: adding another path requires changing this reviewed
# contract, and the inventory fingerprint separately rejects growth in an
# existing owner.
DOMAIN_ABI_FILES = {
    Path("crates/capsem-agent/src/audit.rs"),
    Path("crates/capsem-agent/src/bin/capsem_sysutil.rs"),
    Path("crates/capsem-agent/src/control_writer.rs"),
    Path("crates/capsem-agent/src/dns_proxy.rs"),
    Path("crates/capsem-agent/src/main.rs"),
    Path("crates/capsem-agent/src/mcp_server.rs"),
    Path("crates/capsem-agent/src/net_proxy.rs"),
    Path("crates/capsem-agent/src/vsock_io.rs"),
    Path("crates/capsem-core/src/auto_snapshot.rs"),
    Path("crates/capsem-core/src/auto_snapshot/sparse_copy.rs"),
    Path("crates/capsem-core/src/hypervisor/apple_vz/machine.rs"),
    Path("crates/capsem-core/src/hypervisor/fuse/file_handles.rs"),
    Path("crates/capsem-core/src/hypervisor/fuse/inode_table.rs"),
    Path("crates/capsem-core/src/hypervisor/fuse/mod.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/memory.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/mod.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/sys.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/vcpu.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_blk/fd_util.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_console.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_fs/mod.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_fs/ops_dir.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_fs/ops_file.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_fs/ops_meta.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_mmio.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_vsock.rs"),
    Path("crates/capsem-core/src/hypervisor/kvm/virtio_vsock/lifecycle.rs"),
}

DOMAIN_ABI_MANIFESTS = {
    Path("crates/capsem-agent/Cargo.toml"),
    Path("crates/capsem-core/Cargo.toml"),
}


def _is_test_source(path: Path, root: Path = PROJECT_ROOT) -> bool:
    relative = path.relative_to(root)
    return (
        "tests" in relative.parts
        or "benches" in relative.parts
        or path.name == "tests.rs"
        or path.name.startswith("test_")
    )


def _code_lines_with_raw_references(path: Path) -> list[str]:
    matched: list[str] = []
    for source_line in path.read_text(encoding="utf-8").splitlines():
        stripped = source_line.lstrip()
        if stripped.startswith("//"):
            continue
        code = source_line.split("//", 1)[0].strip()
        if code and RAW_REFERENCE.search(code):
            matched.append(code)
    return matched


def _fingerprint(lines: list[str]) -> str:
    return hashlib.sha256("\n".join(lines).encode()).hexdigest()


def _raw_rust_references(root: Path = PROJECT_ROOT) -> dict[str, str]:
    references: dict[str, str] = {}
    for path in sorted((root / "crates").rglob("*.rs")):
        relative = path.relative_to(root)
        if _is_test_source(path, root) or relative.parts[:3] == (
            "crates",
            "capsem-foundation",
            "src",
        ):
            continue
        lines = _code_lines_with_raw_references(path)
        if lines:
            references[relative.as_posix()] = _fingerprint(lines)
    return references


def _raw_manifest_dependencies(root: Path = PROJECT_ROOT) -> dict[str, str]:
    references: dict[str, str] = {}
    for path in sorted((root / "crates").glob("*/Cargo.toml")):
        relative = path.relative_to(root)
        if relative == Path("crates/capsem-foundation/Cargo.toml"):
            continue
        dependencies = [
            f"{match.group(1)}={match.group(0).split('=', 1)[1].strip()}"
            for match in DIRECT_DEPENDENCY.finditer(path.read_text(encoding="utf-8"))
        ]
        if dependencies:
            references[relative.as_posix()] = _fingerprint(dependencies)
    return references


def _inventory() -> dict[str, dict[str, dict[str, str]]]:
    return tomllib.loads(INVENTORY.read_text(encoding="utf-8"))


def _eligible_domain_abi(path: str) -> bool:
    return Path(path) in DOMAIN_ABI_FILES


def _assert_exact(
    actual: dict[str, str],
    migration: dict[str, str],
    domain_abi: dict[str, str],
    *,
    eligible_domain_abi: Callable[[str], bool] = _eligible_domain_abi,
) -> None:
    overlap = sorted(set(migration) & set(domain_abi))
    unknown_abi = sorted(path for path in domain_abi if not eligible_domain_abi(path))
    classified = migration | domain_abi
    missing = sorted(set(actual) - set(classified))
    stale = sorted(set(classified) - set(actual))
    changed = sorted(
        path for path in set(actual) & set(classified) if actual[path] != classified[path]
    )
    problems = []
    if overlap:
        problems.append(f"classified twice: {overlap}")
    if unknown_abi:
        problems.append(f"ineligible domain ABI paths: {unknown_abi}")
    if missing:
        problems.append(f"unclassified raw references: {missing}")
    if stale:
        problems.append(f"stale inventory entries: {stale}")
    if changed:
        problems.append(f"changed raw-reference fingerprints: {changed}")
    assert not problems, UNIX_BOUNDARY_RATIONALE + "\n" + "\n".join(problems)


def test_raw_rust_references_match_the_exact_boundary_inventory() -> None:
    inventory = _inventory()
    _assert_exact(
        _raw_rust_references(), inventory["migration"]["rust"], inventory["domain_abi"]["rust"]
    )


def test_direct_dependencies_match_the_exact_boundary_inventory() -> None:
    inventory = _inventory()
    migration = inventory["migration"]["manifest"]
    domain_abi = inventory["domain_abi"]["manifest"]
    unknown_abi = sorted(Path(path) for path in domain_abi if Path(path) not in DOMAIN_ABI_MANIFESTS)
    assert not unknown_abi, UNIX_BOUNDARY_RATIONALE + f"\nineligible ABI manifests: {unknown_abi}"
    _assert_exact(
        _raw_manifest_dependencies(),
        migration,
        domain_abi,
        eligible_domain_abi=lambda path: Path(path) in DOMAIN_ABI_MANIFESTS,
    )


def test_alias_import_is_a_raw_reference(tmp_path: Path) -> None:
    path = tmp_path / "alias.rs"
    path.write_text("use nix as system;\nfn probe() { system::unistd::getpid(); }\n")
    assert _code_lines_with_raw_references(path) == ["use nix as system;"]


def test_new_source_and_changed_abi_fingerprint_are_detected(tmp_path: Path) -> None:
    crate = tmp_path / "crates" / "example" / "src"
    crate.mkdir(parents=True)
    source = crate / "runtime.rs"
    source.write_text("fn uid() -> u32 { unsafe { libc::getuid() } }\n")
    first = _raw_rust_references(tmp_path)
    source.write_text(
        "fn uid() -> u32 { unsafe { libc::getuid() } }\nfn ppid() { unsafe { libc::getppid(); } }\n"
    )
    second = _raw_rust_references(tmp_path)
    assert first.keys() == second.keys()
    assert first["crates/example/src/runtime.rs"] != second["crates/example/src/runtime.rs"]


def test_new_manifest_dependency_is_detected(tmp_path: Path) -> None:
    crate = tmp_path / "crates" / "example"
    crate.mkdir(parents=True)
    manifest = crate / "Cargo.toml"
    manifest.write_text("[dependencies]\nnix = { version = \"0.29\", features = [\"signal\"] }\n")
    assert list(_raw_manifest_dependencies(tmp_path)) == ["crates/example/Cargo.toml"]

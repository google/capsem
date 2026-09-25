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

# A raw reference is a nix/libc path, any import of those crates (aliased,
# grouped or `::`-rooted: `use ::libc as sys;` then `sys::getuid()` must not
# pass), or an `extern "C" { fn ... }` block, which binds a syscall without
# going through either crate. `extern "C" fn` handler definitions are not
# bindings and are not matched.
RAW_REFERENCE = re.compile(
    r"\b(?:nix|libc)::"
    r"|^\s*(?:pub(?:\([^)]*\))?\s+)?(?:use|extern\s+crate)\b.*\b(?:nix|libc)\b"
    r'|\bextern\s+"C"\s*\{'
)
# `libc = ...`, `libc.workspace = true`, `[dependencies.libc]` (any target or
# dependency kind), and a renamed `sys = { package = "libc" }`.
DIRECT_DEPENDENCY = re.compile(
    r"^\s*(?:nix|libc)\s*[=.]"
    r"|^\s*\[(?:target\.[^\]]+\.)?(?:dev-|build-)?dependencies\.(?:nix|libc)\]"
    r'|\bpackage\s*=\s*"(?:nix|libc)"'
)
USE_STATEMENT = re.compile(r"(?:pub(?:\([^)]*\))?\s+)?use\b")

# These modules own a kernel or platform ABI rather than reusable Unix policy.
# Eligibility is exact: adding another path requires changing this reviewed
# contract, and the inventory fingerprint separately rejects growth in an
# existing owner.
DOMAIN_ABI_FILES = {
    Path("crates/capsem-agent/src/audit.rs"),
    Path("crates/capsem-agent/src/bin/capsem_sysutil.rs"),
    Path("crates/capsem-agent/src/control_writer.rs"),
    # Guest control owns PTY signals and snapshot filesystem/block-device ioctls.
    Path("crates/capsem-agent/src/control_reader.rs"),
    Path("crates/capsem-agent/src/main.rs"),
    # The guest owns namespace setup on disposable threads, never host Unix policy.
    Path("crates/capsem-agent/src/port_bridge/setup.rs"),
    # The tun pump owns the tun/ifreq ioctls that name, address and raise tun0.
    Path("crates/capsem-agent/src/tun_pump.rs"),
    # The guest proxy owns SO_ORIGINAL_DST, the netfilter ABI that gives an
    # intercepted private connection back its destination.
    Path("crates/capsem-agent/src/net_proxy.rs"),
    Path("crates/capsem-agent/src/mcp_server.rs"),
    Path("crates/capsem-agent/src/shutdown.rs"),
    Path("crates/capsem-agent/src/terminal_bridge.rs"),
    Path("crates/capsem-agent/src/vsock_io.rs"),
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


def _logical_code_lines(text: str) -> list[str]:
    """Code without `//` comments, each multi-line `use` joined into one line
    so `use {` / `libc as sys,` / `};` is seen whole."""
    lines: list[str] = []
    pending: list[str] = []
    for source_line in text.splitlines():
        code = source_line.split("//", 1)[0].strip()
        if not code:
            continue
        if pending:
            pending.append(code)
            if ";" in code:
                lines.append(" ".join(pending))
                pending = []
        elif USE_STATEMENT.match(code) and ";" not in code:
            pending = [code]
        else:
            lines.append(code)
    lines.extend(pending)
    return lines


def _code_lines_with_raw_references(path: Path) -> list[str]:
    return [
        code
        for code in _logical_code_lines(path.read_text(encoding="utf-8"))
        if RAW_REFERENCE.search(code)
    ]


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
            line.strip()
            for line in path.read_text(encoding="utf-8").splitlines()
            if DIRECT_DEPENDENCY.search(line)
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


def test_evasive_rust_spellings_are_raw_references(tmp_path: Path) -> None:
    evasions = {
        "rooted_alias": "use ::libc as sys;\nfn f() { unsafe { sys::getuid(); } }\n",
        "grouped_alias": "use {\n    std::io,\n    libc as sys,\n};\nfn f() {}\n",
        "pub_crate_alias": "pub(crate) use nix as unix;\n",
        "extern_block": 'unsafe extern "C" {\n    fn getuid() -> u32;\n}\n',
        "plain_extern_block": 'extern "C" { fn getpid() -> i32; }\n',
    }
    for name, source in evasions.items():
        path = tmp_path / f"{name}.rs"
        path.write_text(source)
        assert _code_lines_with_raw_references(path), name
    legitimate = {
        "signal_handler": 'extern "C" fn on_signal(_: i32) {}\n',
        "foundation_use": "use capsem_foundation::unix::fd;\n",
        "lookalike_module": "use crate::net::libcurl_shim;\nuse crate::phoenix::nixie;\n",
        "comment": "// libc::getuid() would be wrong here\nfn f() {}\n",
    }
    for name, source in legitimate.items():
        path = tmp_path / f"{name}.rs"
        path.write_text(source)
        assert not _code_lines_with_raw_references(path), name


def test_evasive_manifest_spellings_are_direct_dependencies(tmp_path: Path) -> None:
    evasions = {
        "workspace": "[dependencies]\nlibc.workspace = true\n",
        "table": "[dependencies.nix]\nversion = \"0.29\"\n",
        "target_table": "[target.'cfg(unix)'.dev-dependencies.libc]\nversion = \"0.2\"\n",
        "renamed": "[dependencies]\nsys = { package = \"libc\", version = \"0.2\" }\n",
    }
    for name, manifest in evasions.items():
        crate = tmp_path / name / "crates" / "example"
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(manifest)
        assert list(_raw_manifest_dependencies(tmp_path / name)) == ["crates/example/Cargo.toml"], name


# -- Inside the boundary -------------------------------------------------------
#
# Foundation is the one crate allowed to reach the kernel, and nix is how it
# does: nix wraps the syscall, retries nothing silently, and returns Errno that
# `unix::errno::io` converts in one place. A raw `libc::` call inside foundation
# bypasses that and gets none of it checked; the outer guard never looked here,
# so raw calls with a nix wrapper accumulated unnoticed. Every raw call (and
# every `extern "C"` binding) in foundation is therefore listed exactly, with
# the reason nix cannot express it.

FOUNDATION_SRC = Path("crates/capsem-foundation/src")
RAW_LIBC_CALL = re.compile(r"\blibc::([A-Za-z_]\w*)\s*\(")
EXTERN_BINDING = re.compile(r"\bfn\s+([A-Za-z_]\w*)\s*\(")

# Calls nix wraps on every platform Capsem builds for. An inventory entry for
# one of these must say what the wrapper cannot express (`nix_gap`), so an
# exemption is never just "it was easier".
NIX_WRAPPED = {
    "close": "nix::unistd::close",
    "connect": "nix::sys::socket::connect",
    "dup": "nix::unistd::dup",
    "fchmod": "nix::sys::stat::fchmod",
    "fcntl": "nix::fcntl::fcntl",
    "fstat": "nix::sys::stat::fstat",
    "fsync": "nix::unistd::fsync",
    "ftruncate": "nix::unistd::ftruncate",
    "ioctl": "nix::ioctl_* macros",
    "kill": "nix::sys::signal::kill",
    "lseek": "nix::unistd::lseek",
    "mkdirat": "nix::sys::stat::mkdirat",
    "open": "nix::fcntl::open",
    "openat": "nix::fcntl::openat",
    "pipe": "nix::unistd::pipe2",
    "prctl": "nix::sys::prctl",
    "read": "nix::unistd::read",
    "readlinkat": "nix::fcntl::readlinkat",
    "recvmsg": "nix::sys::socket::recvmsg",
    "sendmsg": "nix::sys::socket::sendmsg",
    "setsockopt": "nix::sys::socket::setsockopt",
    "symlinkat": "nix::unistd::symlinkat",
    "syncfs": "nix::unistd::syncfs",
    "unlinkat": "nix::unistd::unlinkat",
    "waitpid": "nix::sys::wait::waitpid",
    "write": "nix::unistd::write",
}


def _foundation_raw_calls(root: Path = PROJECT_ROOT) -> dict[str, str]:
    """`<path>::<call>` -> fingerprint of the lines making that raw call."""
    lines_by_call: dict[str, list[str]] = {}
    for path in sorted((root / FOUNDATION_SRC).rglob("*.rs")):
        if _is_test_source(path, root):
            continue
        relative = path.relative_to(root).as_posix()
        in_extern_block = False
        depth = 0
        for code in _logical_code_lines(path.read_text(encoding="utf-8")):
            names = [match.group(1) for match in RAW_LIBC_CALL.finditer(code)]
            if re.search(r'\bextern\s+"C"\s*\{', code):
                in_extern_block, depth = True, 0
            if in_extern_block:
                names += [match.group(1) for match in EXTERN_BINDING.finditer(code)]
                depth += code.count("{") - code.count("}")
                in_extern_block = depth > 0
            for name in names:
                lines_by_call.setdefault(f"{relative}::{name}", []).append(code)
    return {key: _fingerprint(lines) for key, lines in lines_by_call.items()}


def _assert_foundation_inventory(actual: dict[str, str], inventory: dict[str, dict[str, str]]) -> None:
    problems = []
    for key in sorted(set(actual) - set(inventory)):
        call = key.rsplit("::", 1)[1]
        wrapper = NIX_WRAPPED.get(call)
        hint = f" -- use {wrapper}" if wrapper else " -- list it with the reason nix cannot express it"
        problems.append(f"unlisted raw call {key}{hint}")
    for key in sorted(set(inventory) - set(actual)):
        problems.append(f"stale inventory entry {key}")
    for key in sorted(set(actual) & set(inventory)):
        entry = inventory[key]
        call = key.rsplit("::", 1)[1]
        if entry.get("fingerprint") != actual[key]:
            problems.append(f"changed raw call lines {key}: {actual[key]}")
        if len(entry.get("reason", "")) < 30:
            problems.append(f"{key} needs a reason of at least 30 characters")
        if call in NIX_WRAPPED and len(entry.get("nix_gap", "")) < 30:
            problems.append(f"{key}: {NIX_WRAPPED[call]} exists; state the nix_gap it cannot express")
    assert not problems, UNIX_BOUNDARY_RATIONALE + "\n" + "\n".join(problems)


def _foundation_inventory() -> dict[str, dict[str, str]]:
    raw_calls: dict[str, dict[str, str]] = tomllib.loads(INVENTORY.read_text(encoding="utf-8"))["foundation"][
        "raw_calls"
    ]
    return raw_calls


def test_foundation_raw_calls_match_the_exact_inventory() -> None:
    _assert_foundation_inventory(_foundation_raw_calls(), _foundation_inventory())


def test_foundation_inventory_rejects_new_stale_changed_and_unjustified_calls(tmp_path: Path) -> None:
    source = tmp_path / FOUNDATION_SRC / "unix" / "probe.rs"
    source.parent.mkdir(parents=True)
    source.write_text(
        "fn a() { unsafe { libc::fsync(3) }; }\n"
        'unsafe extern "C" {\n    fn sandbox_probe() -> i32;\n}\n'
    )
    actual = _foundation_raw_calls(tmp_path)
    assert set(actual) == {
        f"{FOUNDATION_SRC.as_posix()}/unix/probe.rs::fsync",
        f"{FOUNDATION_SRC.as_posix()}/unix/probe.rs::sandbox_probe",
    }
    fsync_key = f"{FOUNDATION_SRC.as_posix()}/unix/probe.rs::fsync"
    extern_key = f"{FOUNDATION_SRC.as_posix()}/unix/probe.rs::sandbox_probe"
    justified = "reviewed: this binding has no wrapper in nix at all"

    def failure(inventory: dict[str, dict[str, str]], observed: dict[str, str] = actual) -> str:
        try:
            _assert_foundation_inventory(observed, inventory)
        except AssertionError as error:
            return str(error)
        return ""

    assert "use nix::unistd::fsync" in failure({})
    listed = {
        fsync_key: {"fingerprint": actual[fsync_key], "reason": justified},
        extern_key: {"fingerprint": actual[extern_key], "reason": justified},
    }
    assert "state the nix_gap" in failure(listed), "a nix-wrapped call needs its gap stated"
    listed[fsync_key]["nix_gap"] = "hypothetical: the wrapper cannot take this descriptor"
    assert failure(listed) == ""
    assert "stale inventory entry" in failure({**listed, "x.rs::gone": listed[extern_key]})
    # A second call of an already-listed function is new code, not the same entry.
    source.write_text(source.read_text() + "fn b() { unsafe { libc::fsync(4) }; }\n")
    assert "changed raw call lines" in failure(listed, _foundation_raw_calls(tmp_path))

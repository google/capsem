"""Count first-party source with scc and partition inline Rust test code."""

from __future__ import annotations

import json
import re
import subprocess
import tarfile
from pathlib import Path

import tree_sitter_rust
from tree_sitter import Language, Parser

PARSER = Parser(Language(tree_sitter_rust.language()))
SOURCE_EXT = {
    "rs",
    "py",
    "ts",
    "tsx",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "svelte",
    "astro",
    "css",
    "scss",
    "sass",
    "less",
    "html",
    "htm",
    "c",
    "cc",
    "cpp",
    "h",
    "hpp",
    "sh",
    "bash",
    "zsh",
    "fish",
    "swift",
    "m",
    "mm",
    "go",
    "sql",
    "pl",
    "rb",
    "lua",
    "awk",
}

SKIP_PARTS = {
    "vendor",
    "vendored",
    "third_party",
    "third-party",
    "node_modules",
    "dist",
    ".venv",
    "__pycache__",
    "target",
}

COMPONENTS = [
    "hypervisor",
    "clients",
    "vm_guest",
    "vm_host",
    "host_other",
    "tests",
    "citadel_guardrails",
    "benchmarks",
    "build_release_tooling",
    "docs_marketing",
    "other",
]


def is_source(path):
    p = Path(path)
    return (
        p.suffix.lstrip(".") in SOURCE_EXT
        or p.name
        in {
            "Dockerfile",
            "justfile",
            "Justfile",
            "capsem-init",
            "capsem-doctor",
            "capsem-bench",
            "bootstrap",
        }
        or p.name.startswith("Dockerfile.")
        or path.startswith("benchmarks/collectors/")
    )


def excluded(path):
    return bool(set(Path(path).parts) & SKIP_PARTS)


def classify(path, legacy_hypervisor=False):
    parts = Path(path).parts
    name = Path(path).name
    stem = Path(path).stem
    if path.endswith("test_lifecycle_benchmark.py"):
        return "benchmarks", "legacy_pytest"
    if path.startswith("images/diagnostics/") or path in {
        "images/capsem-doctor",
        "guest/artifacts/capsem-doctor",
    }:
        return "tests", "doctor"
    if (
        "citadel" in parts
        or "guards" in parts
        or "contracts" in parts
        or (
            ("tests" in parts or name.startswith("test_"))
            and re.search(r"contract|boundary|guardrail", name)
        )
    ):
        return "citadel_guardrails", "source_guards"
    if (
        set(parts) & {"tests", "__tests__", "test", "fixtures", "__fixtures__", "fuzz"}
        or any(part.endswith("_tests") for part in parts)
        or name in {"tests.rs", "test.rs"}
        or name.startswith(("test_", "tests_"))
        or stem.endswith(("_test", "_tests", ".test", ".spec"))
        or name == "conftest.py"
        or path.startswith(
            (
                "guest/artifacts/capsem_doctor/",
                "guest/artifacts/tests/",
                "crates/capsem-mock-server/",
                "crates/capsem-debug-upstream/",
            )
        )
    ):
        return "tests", "benchmarks_fuzz" if set(parts) & {
            "benches",
            "benchmarks",
            "fuzz",
        } else "tests_helpers_fixtures"
    if (
        set(parts) & {"benches", "benchmarks"}
        or path.startswith(("crates/capsem-bench/", "guest/artifacts/capsem_bench/"))
        or path in {"images/capsem-bench", "guest/artifacts/capsem-bench"}
    ):
        return "benchmarks", "performance_harness"
    if (
        legacy_hypervisor
        and path.startswith("crates/capsem-core/src/vm/")
        and name in {"boot.rs", "machine.rs", "serial.rs", "vsock.rs", "mod.rs"}
    ):
        return "hypervisor", "legacy_apple_vz"
    if (
        path.startswith(("crates/capsem-core/src/hypervisor/", "crates/capsem-hypervisor/"))
        or path == "crates/capsem-core/src/hypervisor.rs"
    ):
        return (
            "hypervisor",
            "apple_vz" if "/apple_vz/" in path else "kvm" if "/kvm/" in path else "shared_fuse",
        )
    if path.startswith("sdk/"):
        return "clients", "sdk_" + parts[1]
    if path.startswith(("mcp/", "crates/capsem-mcp/")):
        return "clients", "mcp_host"
    if path.startswith(("crates/capsem-app/", "web/app/", "frontend/")):
        return "clients", "app"
    if path.startswith("crates/capsem-tui/"):
        return "clients", "tui"
    if path.startswith("crates/capsem/"):
        return "clients", "cli"
    if path.startswith("crates/capsem-tray/"):
        return "clients", "tray"
    if path.startswith(("crates/capsem-agent/", "guest/")):
        return "vm_guest", "guest_agent" if path.startswith("crates/") else "guest_scripts"
    if path == "images/capsem-init":
        return "vm_guest", "legacy_guest_scripts"
    if path.startswith(("crates/capsem-core/src/vm/", "crates/capsem-process/")):
        return "vm_host", "lifecycle"
    if path.startswith("crates/"):
        return "host_other", parts[1]
    if path.startswith(("capsem/", "capsem_python/", "capsem_proxy/")):
        return "host_other", "legacy_python_runtime"
    if path.startswith(("web/docs/", "web/marketing/", "web/diagrams/", "site/", "docs/")):
        return "docs_marketing", parts[1] if len(parts) > 1 else "site"
    if path.startswith(
        ("build_system/", "scripts/", "src/capsem/", "docker/", ".github/", "config/docker/")
    ) or name in {"bootstrap.sh", "justfile", "Justfile"}:
        return "build_release_tooling", "build_release"
    return "other", parts[0]


def generated(data, path):
    head = data[:2500].decode("utf-8", errors="replace")
    return (
        "generated" in Path(path).parts
        or "__generated__" in Path(path).parts
        or bool(
            re.search(
                r'(?im)^\s*(?://|#|/\*|\*|"""|<!--).*?(?:do not edit|@generated|automatically generated|autogenerated|code generated|generated from)',
                head,
            )
        )
    )


def inline_test_spans(data):
    spans = []

    def walk(node):
        attrs = []
        for child in node.named_children:
            if child.type == "attribute_item":
                attrs.append(child)
                continue
            marker = b" ".join(data[a.start_byte : a.end_byte] for a in attrs)
            is_test = (
                re.search(rb"#\[\s*cfg\s*\(\s*test\s*\)\s*\]", marker)
                or re.search(rb"#\[\s*(?:tokio::|async_std::)?test(?:\s*\(|\s*\])", marker)
                or re.search(rb"#\[\s*(?:rstest|quickcheck)(?:\s*\(|\s*\])", marker)
            )
            if is_test:
                spans.append((attrs[0].start_byte, child.end_byte))
            else:
                walk(child)
            attrs = []

    tree = PARSER.parse(data)
    walk(tree.root_node)
    return sorted(spans)


def extract_snapshot(repo, commit, root):
    proc = subprocess.Popen(["git", "-C", str(repo), "archive", commit], stdout=subprocess.PIPE)
    paths = []
    with tarfile.open(fileobj=proc.stdout, mode="r|") as archive:
        for member in archive:
            path = member.name
            if not member.isfile() or excluded(path):
                continue
            assert not Path(path).is_absolute() and ".." not in Path(path).parts
            name = Path(path).name
            if not (
                is_source(path)
                or name
                in {
                    "Cargo.toml",
                    "Cargo.lock",
                    "package.json",
                    "package-lock.json",
                    "pnpm-lock.yaml",
                    "pyproject.toml",
                    "uv.lock",
                    "yarn.lock",
                }
                or path == "config/gate.toml"
                or path.startswith(".github/workflows/")
            ):
                continue
            dest = root / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(archive.extractfile(member).read())
            paths.append(path)
    if proc.wait() != 0:
        raise RuntimeError("git archive failed")
    return paths


def loc(root, paths, scc):
    generated_files = set()
    split_mapping = {}
    for path in paths:
        if not is_source(path):
            continue
        file = root / path
        data = file.read_bytes()
        if generated(data, path):
            generated_files.add(path)
            continue
        if file.suffix == ".rs" and classify(path)[0] not in {"tests", "citadel_guardrails"}:
            spans = inline_test_spans(data)
            if spans:
                snippets = []
                for start, end in spans:
                    snippets.append(data[start:end])
                prod = data
                for start, end in reversed(spans):
                    prod = prod[:start] + prod[end:]
                file.write_bytes(prod)
                synthetic = ".analysis-inline-tests/" + path
                destination = root / synthetic
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(b"\n".join(snippets) + b"\n")
                split_mapping[synthetic] = path
    body = subprocess.check_output(
        [
            str(scc),
            "--no-config",
            "--no-gitignore",
            "--no-ignore",
            "--no-scc-ignore",
            "--no-complexity",
            "--no-cocomo",
            "--by-file",
            "--format",
            "json",
            str(root),
        ],
        text=True,
    )
    counts = json.loads(body)
    records = []
    for language in counts:
        for file in language.get("Files") or []:
            path = Path(file["Location"]).relative_to(root).as_posix()
            if not is_source(path):
                continue
            synthetic = path in split_mapping
            source_path = split_mapping.get(path, path)
            component, subcomponent = (
                ("tests", "inline_rust_tests")
                if synthetic
                else classify(
                    path,
                    legacy_hypervisor=not (root / "crates/capsem-core/src/hypervisor").exists(),
                )
            )
            records.append(
                {
                    "path": source_path + ("#inline-tests" if synthetic else ""),
                    "language": language["Name"],
                    "component": component,
                    "subcomponent": subcomponent,
                    "code_loc": file["Code"],
                    "comment_lines": file["Comment"],
                    "blank_lines": file["Blank"],
                    "physical_lines": file["Lines"],
                    "generated_excluded": int(source_path in generated_files),
                    "synthetic_inline_test_fragment": int(synthetic),
                }
            )
    return records

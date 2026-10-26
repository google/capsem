"""Static test definitions; parameterized cases, platform runs and doctests are not expanded."""

from __future__ import annotations

import ast
import re
from collections import Counter
from pathlib import Path

import tree_sitter_typescript
from tree_sitter import Language, Parser

from .code import PARSER, classify, generated

TS = Parser(Language(tree_sitter_typescript.language_typescript()))
TSX = Parser(Language(tree_sitter_typescript.language_tsx()))


def nodes(root):
    yield root
    for child in root.named_children:
        yield from nodes(child)


def test_system(path, language):
    if language == "rust":
        return "rust", path.split("/")[1] if path.startswith("crates/") else "sdk_rust"
    if path.startswith(("guest/artifacts/diagnostics/", "images/diagnostics/")):
        return "doctor", Path(path).stem
    if path.startswith("tests/citadel/"):
        return "citadel", "source_guards"
    if path.startswith("tests/ironbank/"):
        return "ironbank", "black_box_acceptance"
    if path.startswith("sdk/python/"):
        return "python_sdk", "sdk_python"
    if path.startswith("sdk/typescript/"):
        return "typescript_sdk", "sdk_typescript"
    if path.startswith("mcp/typescript/"):
        return "mcp_typescript", "host_mcp"
    if path.startswith("integrations/inspect-ai/"):
        return "inspect_ai", "integration"
    if path.startswith(("web/app/", "frontend/")):
        return "frontend", "app"
    if path.startswith("build_system/release_site/"):
        return "release_site", "release_website"
    if path.startswith(("build_system/tests/", "tests/build_system/")):
        return "python_build_system", Path(path).parts[2]
    if classify(path)[0] == "citadel_guardrails":
        return "legacy_source_contracts", "source_guards"
    if path.startswith(("tests/capsem-", "tests/images/")):
        return "python_vm_integration", Path(path).parts[1]
    return "python_other" if language == "python" else "javascript_other", str(Path(path).parent)


def python_tests(path, data):
    if not re.match(r"(test.*\.py|.*_test\.py)$", Path(path).name):
        return []
    tree = ast.parse(data, filename=path)
    result = []

    def visit(body, classname=""):
        for node in body:
            if isinstance(node, ast.ClassDef):
                if node.name.startswith("Test") or any(
                    "TestCase" in ast.unparse(base) for base in node.bases
                ):
                    visit(node.body, node.name)
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith(
                "test"
            ):
                decorators = " | ".join(ast.unparse(d) for d in node.decorator_list)
                if re.search(r"\bfixture\b", decorators):
                    continue
                system, subsystem = test_system(path, "python")
                performance = (
                    "pytest.mark.benchmark" in decorators
                    or path.endswith("test_lifecycle_benchmark.py")
                    or "/benchmarks/" in path
                )
                result.append(
                    {
                        "path": path,
                        "line": node.lineno,
                        "language": "python",
                        "system": system,
                        "subsystem": subsystem,
                        "test_name": (classname + "." if classname else "") + node.name,
                        "parameterized": int("parametrize" in decorators),
                        "conditional_or_skipped": int(
                            "skip" in decorators or "xfail" in decorators
                        ),
                        "target_conditions": "",
                        "benchmark": int(performance),
                    }
                )

    visit(tree.body)
    return result


def rust_tests(path, data):
    result = []

    def walk(node, inherited=""):
        attrs = []
        for child in node.named_children:
            if child.type == "attribute_item":
                attrs.append(data[child.start_byte : child.end_byte].decode())
                continue
            if child.type in {"line_comment", "block_comment"}:
                continue
            markers = " ".join(attrs)
            conditions = inherited + " " + " ".join(a for a in attrs if re.search(r"cfg\s*\(", a))
            if child.type == "function_item" and re.search(
                r"#\[\s*(?:(?:tokio|async_std)::)?test(?:\s*\(|\s*\])|#\[\s*(?:rstest|quickcheck|test_case)(?:\s*\(|\s*\])",
                markers,
            ):
                name = child.child_by_field_name("name")
                system, subsystem = test_system(path, "rust")
                result.append(
                    {
                        "path": path,
                        "line": child.start_point.row + 1,
                        "language": "rust",
                        "system": system,
                        "subsystem": subsystem,
                        "test_name": data[name.start_byte : name.end_byte].decode(),
                        "parameterized": int(
                            bool(re.search(r"rstest|quickcheck|test_case", markers))
                        ),
                        "conditional_or_skipped": int(
                            bool(
                                re.search(
                                    r"ignore|target_arch|target_os", conditions + " " + markers
                                )
                            )
                        ),
                        "target_conditions": conditions.strip(),
                        "benchmark": 0,
                    }
                )
            walk(child, conditions)
            attrs = []

    tree = PARSER.parse(data)
    walk(tree.root_node)
    return result


def typescript_tests(path, data):
    if (
        not re.search(r"(?:\.test|\.spec)\.[cm]?[jt]sx?$", path)
        and "__tests__" not in Path(path).parts
    ):
        return []
    parser = TSX if path.endswith(("tsx", "jsx")) else TS
    tree = parser.parse(data)
    result = []

    def root_identifier(node):
        if node.type == "identifier":
            return data[node.start_byte : node.end_byte].decode()
        for field in ["function", "object"]:
            value = node.child_by_field_name(field)
            if value:
                return root_identifier(value)
        return ""

    for node in nodes(tree.root_node):
        if node.type != "call_expression":
            continue
        function = node.child_by_field_name("function")
        if not function or root_identifier(function) not in {"test", "it", "fit", "xit", "xtest"}:
            continue
        args = node.child_by_field_name("arguments")
        if (
            not args
            or not args.named_children
            or args.named_children[0].type not in {"string", "template_string"}
        ):
            continue
        label = args.named_children[0]
        name = data[label.start_byte : label.end_byte].decode()[1:-1]
        callee = data[function.start_byte : function.end_byte].decode()
        # Exclude the builder call it.each('template') before its final call.
        if callee.endswith((".each", ".for")):
            continue
        system, subsystem = test_system(path, "typescript")
        result.append(
            {
                "path": path,
                "line": node.start_point.row + 1,
                "language": "typescript",
                "system": system,
                "subsystem": subsystem,
                "test_name": name,
                "parameterized": int(bool(re.search(r"\.(each|for)\b", callee))),
                "conditional_or_skipped": int(
                    bool(re.search(r"\.(skip|todo|skipIf|runIf)\b|^x(it|test)", callee))
                ),
                "target_conditions": "",
                "benchmark": 0,
            }
        )
    return result


def counts(root, paths):
    totals = Counter()
    for path in paths:
        if set(Path(path).parts) & {"fixtures", "__fixtures__", "fuzz"}:
            continue
        data = (root / path).read_bytes()
        if generated(data, path):
            continue
        if path.endswith(".py"):
            records = python_tests(path, data)
        elif path.endswith(".rs"):
            records = rust_tests(path, data)
        elif path.endswith((".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs")):
            records = typescript_tests(path, data)
        else:
            continue
        totals.update(r["system"] for r in records if not r["benchmark"])
    return dict(totals)

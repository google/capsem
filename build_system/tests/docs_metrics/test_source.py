from capsem_builder.docs_metrics.code import classify, generated, inline_test_spans
from capsem_builder.docs_metrics.dependencies import declarations
from capsem_builder.docs_metrics.test_inventory import python_tests, typescript_tests


def test_inline_rust_tests_are_partitioned_without_counting_test_text_in_strings():
    data = b'fn production() { let x = "#[test] fn fake() {}"; }\n#[cfg(test)]\nmod tests { #[test] fn real() {} }'
    spans = inline_test_spans(data)
    assert len(spans) == 1
    start, end = spans[0]
    assert data[start:end].startswith(b"#[cfg(test)]")
    assert b"production" not in data[start:end]


def test_generated_code_and_source_roles_do_not_double_count():
    assert generated(b"// Generated from OpenAPI. Do not edit.\n", "sdk/client.ts")
    assert classify("crates/capsem-core/src/hypervisor/kvm/tests.rs")[0] == "tests"
    assert classify("tests/citadel/test_db_boundary.py")[0] == "citadel_guardrails"
    assert classify("crates/capsem-bench/src/main.rs")[0] == "benchmarks"


def test_static_definition_counts_ignore_nested_helpers_and_expand_nothing():
    py = b"def test_one():\n    def test_nested(): pass\n"
    assert len(python_tests("tests/test_example.py", py)) == 1
    ts = b'it.each([1,2,3])("one declaration", () => {}); const fake="it(fake)";'
    assert len(typescript_tests("tests/example.test.ts", ts)) == 1


def test_workspace_inheritance_renames_and_internal_edges(tmp_path):
    (tmp_path / "Cargo.toml").write_text(
        '[workspace]\nmembers=["a","b"]\n[workspace.dependencies]\njson={package="serde_json",version="1"}\n'
    )
    for name in ["a", "b"]:
        (tmp_path / name).mkdir()
        (tmp_path / name / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\n')
    with (tmp_path / "a/Cargo.toml").open("a") as f:
        f.write('[dependencies]\njson={workspace=true}\nb={path="../b"}\n')
    paths = ["Cargo.toml", "a/Cargo.toml", "b/Cargo.toml"]
    rows, members, _, _ = declarations(tmp_path, paths)
    assert len(members) == 2
    assert {(r["package"], r["internal"]) for r in rows} == {("serde_json", 0), ("b", 1)}

"""Compiler-produced declarations must survive generation without bodies."""

from __future__ import annotations

import ast
import importlib.util
import subprocess
import sys
from pathlib import Path


def generator():
    specification = importlib.util.spec_from_file_location('sdk_reference', Path(__file__).with_name('generate-sdk-reference.py'))
    assert specification and specification.loader
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def test_rust_inherent_signatures_exclude_blanket_traits() -> None:
    parser = generator().RustDeclarations()
    parser.feed('<pre class="rust item-decl"><code>pub struct VM {}</code></pre>'
                '<h4 class="code-header">pub async fn exec(&amp;self) -&gt; Result&lt;()&gt;</h4>'
                '<h2 id="trait-implementations">Traits</h2>'
                '<h4 class="code-header">fn clone(&amp;self)</h4>')
    assert parser.declarations == ['pub struct VM {}', 'pub async fn exec(&self) -> Result<()>']


def test_python_reference_includes_resources_signatures_and_models() -> None:
    result = generator().python_reference()
    assert 'async def create(' in result
    assert 'class Hypervisor(Client)' in result
    assert 'class VM(Client)' in result
    assert 'class ExecResult(ExecResponse)' in result
    assert '_operations' not in result
    for block in result.split('```python\n')[1:]:
        ast.parse(block.split('```')[0])


def test_python_only_generation_needs_no_rustdoc() -> None:
    result = subprocess.run(
        [sys.executable, str(Path(__file__).with_name('generate-sdk-reference.py')),
         '--languages', 'python', '--check'],
        capture_output=True, text=True, timeout=10, check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_rust_generation_requires_current_rustdoc() -> None:
    result = subprocess.run(
        [sys.executable, str(Path(__file__).with_name('generate-sdk-reference.py')),
         '--languages', 'rust', '--check'],
        capture_output=True, text=True, timeout=10, check=False,
    )
    assert result.returncode == 2
    assert '--rustdoc-dir is required' in result.stderr

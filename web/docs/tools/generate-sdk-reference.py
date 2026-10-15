"""Generate SDK references from Python AST, TypeScript declarations and rustdoc.

Run from any directory through the repository's bounded-command wrapper.
Build TypeScript declarations and Rust documentation first; --check refuses
drift without changing generated Markdown. Package READMEs are never generated.
"""

from __future__ import annotations

import argparse
import ast
import json
import subprocess
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DEST = ROOT / 'web/docs/src/content/docs/sdk'


def fence(language: str, text: str) -> str:
    return f'```{language}\n{text.strip()}\n```\n'


def python_reference() -> str:
    sections = []
    package = ROOT / 'sdk/python/capsem'
    first = {'hypervisor.py': 0, 'vm.py': 1, 'managed.py': 2, 'execution.py': 3, 'registry.py': 4}
    for path in sorted(package.rglob('*.py'), key=lambda path: (first.get(path.name, 10 if 'models' not in path.parts else 20), str(path))):
        if path.name in ('__init__.py', '_client.py') or '_operations' in path.parts:
            continue
        module = '.'.join(path.relative_to(package).with_suffix('').parts)
        tree = ast.parse(path.read_text())
        declarations = []
        for node in tree.body:
            if isinstance(node, ast.ClassDef) and not node.name.startswith('_'):
                if node.name in ('Resource', 'Transport', 'Method', 'MediaType'):
                    continue
                lines = ['class ' + node.name + '(' + ', '.join(ast.unparse(base) for base in node.bases) + '):']
                doc = ast.get_docstring(node)
                if doc:
                    lines.append('    ' + repr(doc))
                attributes = {}
                for assignment in ast.walk(node):
                    if isinstance(assignment, ast.Assign):
                        for target in assignment.targets:
                            if isinstance(target, ast.Attribute) and isinstance(target.value, ast.Name) and target.value.id == 'self' and not target.attr.startswith('_'):
                                attributes[target.attr] = ast.unparse(assignment.value.func) if isinstance(assignment.value, ast.Call) else 'object'
                lines.extend('    ' + name + ': ' + annotation for name, annotation in sorted(attributes.items()))
                for member in node.body:
                    if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        if member.name.startswith('_') and member.name not in ('__init__', '__aenter__', '__aexit__', '__str__'):
                            continue
                        for decorator in member.decorator_list:
                            lines.append('    @' + ast.unparse(decorator))
                        signature = ('async ' if isinstance(member, ast.AsyncFunctionDef) else '') + 'def ' + member.name
                        signature += '(' + ast.unparse(member.args) + ')'
                        if member.returns:
                            signature += ' -> ' + ast.unparse(member.returns)
                        lines.append('    ' + signature + ':')
                        doc = ast.get_docstring(member)
                        if doc:
                            lines.append('        ' + repr(doc))
                        lines.append('        ...')
                    elif isinstance(member, (ast.AnnAssign, ast.Assign)):
                        targets = [member.target] if isinstance(member, ast.AnnAssign) else member.targets
                        if any(isinstance(target, ast.Name) and not target.id.startswith('_') and target.id != 'model_config' for target in targets):
                            lines.append('    ' + ast.unparse(member))
                declarations.append('\n'.join(lines))
        if declarations:
            label = ', '.join(('models.' if module.startswith('models.') else '') + node.name for node in tree.body if isinstance(node, ast.ClassDef) and not node.name.startswith('_') and node.name not in ('Resource', 'Transport', 'Method', 'MediaType'))
            sections.append('## ' + label + '\n\n' + fence('python', '\n\n'.join(declarations)))
    return '\n'.join(sections)


def typescript_reference() -> str:
    data = subprocess.check_output(['node', str(ROOT / 'web/docs/tools/typescript-reference.mjs'), str(ROOT / 'sdk/typescript/dist')], text=True)
    return '\n'.join('## ' + entry['name'] + '\n\n' + fence('ts', entry['declarations']) for entry in json.loads(data))


class RustDeclarations(HTMLParser):
    """Extract rustdoc's public declaration and inherent method signatures."""

    def __init__(self) -> None:
        super().__init__()
        self.declarations: list[str] = []
        self.capture: str | None = None
        self.parts: list[str] = []
        self.inherent = True
        self.doc_depth = 0
        self.doc_parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag == 'h2' and attributes.get('id') in ('trait-implementations', 'synthetic-implementations', 'blanket-implementations'):
            self.inherent = False
        classes = (attributes.get('class') or '').split()
        if tag == 'div' and self.doc_depth:
            self.doc_depth += 1
        elif tag == 'div' and 'docblock' in classes and self.inherent:
            self.doc_depth = 1
            self.doc_parts = []
        if (tag == 'pre' and 'item-decl' in classes) or (tag == 'h4' and 'code-header' in classes and self.inherent):
            self.capture = tag
            self.parts = []

    def handle_data(self, data: str) -> None:
        if self.capture:
            self.parts.append(data)
        if self.doc_depth:
            self.doc_parts.append(data)

    def handle_endtag(self, tag: str) -> None:
        if tag in ('p', 'li', 'pre') and self.doc_depth:
            self.doc_parts.append(' ')
        if tag == 'div' and self.doc_depth:
            self.doc_depth -= 1
            if not self.doc_depth:
                self.declarations.append('// ' + ' '.join(''.join(self.doc_parts).split()))
        if self.capture == tag:
            self.declarations.append(''.join(self.parts).replace(' ⓘ', ''))
            self.capture = None


def rust_reference(directory: Path) -> str:
    sections = []
    seen: set[str] = set()
    for package in ('capsem_sdk', 'capsem_api'):
        first = {'struct.Hypervisor.html': 0, 'struct.VM.html': 1, 'struct.ManagedSession.html': 2}
        paths = sorted((directory / package).rglob('*.html'), key=lambda path: (first.get(path.name, 10), str(path)))
        if not paths:
            raise ValueError(f'build current rustdoc for {package} before generation')
        for path in paths:
            if path.name.split('.')[0] not in ('struct', 'enum', 'trait', 'type', 'fn', 'constant'):
                continue
            # rustdoc may emit reexports at more than one location.
            parser = RustDeclarations()
            parser.feed(path.read_text())
            key = '\n'.join(parser.declarations)
            if not key or key in seen:
                continue
            seen.add(key)
            name = package + '::' + '::'.join(path.relative_to(directory / package).parts[:-1])
            name += ('::' if not name.endswith('::') else '') + path.name.split('.')[1]
            sections.append('## ' + name + '\n\n' + fence('rust', '\n\n'.join(parser.declarations)))
    return '\n'.join(sections)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustdoc-dir', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    bodies = {'python': python_reference(), 'typescript': typescript_reference(), 'rust': rust_reference(args.rustdoc_dir)}
    changed = []
    for language, body in bodies.items():
        title = {'python': 'Python', 'typescript': 'TypeScript', 'rust': 'Rust'}[language]
        text = f'---\ntitle: {title} SDK reference\ndescription: What are the public {title} SDK interfaces and their contracts?\n---\n\n'
        text += '<!-- Generated by web/docs/tools/generate-sdk-reference.py. Do not edit. -->\n\n'
        text += f'Public signatures and types from the {title} SDK, including resource helpers and model contracts.\n'
        text += 'Start with [Getting started with the SDK](/sdk/); see [Errors and recovery](/sdk/errors/) for connection lifetime, deadlines, and retries.\n\n'
        text += body
        path = DEST / (language + '.md')
        if not path.exists() or path.read_text() != text:
            changed.append(str(path.relative_to(ROOT)))
            if not args.check:
                path.write_text(text)
    for path in changed:
        print(path)
    return int(args.check and bool(changed))


if __name__ == '__main__':
    raise SystemExit(main())

// Read compiler declarations rather than guessing TypeScript signatures.
import ts from '../../../sdk/typescript/node_modules/typescript/lib/typescript.js';
import {readdirSync, readFileSync} from 'node:fs';
import {join, relative} from 'node:path';

const root = process.argv[2];
const result = [];
function walk(directory) {
  for (const entry of readdirSync(directory, {withFileTypes: true})) {
    const file = join(directory, entry.name);
    if (entry.isDirectory()) { walk(file); continue; }
    if (!entry.name.endsWith('.d.ts') || entry.name === 'index.d.ts') continue;
    if (['client.d.ts'].includes(entry.name)) continue;
    const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true);
    const symbols = [];
    for (const node of source.statements) {
      if (!node.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword)) continue;
      if (entry.name === 'execution.d.ts' && node.name?.getText(source) !== 'decodeExecOutput') continue;
      const printer = ts.createPrinter();
      let declaration = node;
      if (ts.isClassDeclaration(node)) {
        const members = node.members.filter(member =>
          !member.modifiers?.some(modifier => [ts.SyntaxKind.PrivateKeyword, ts.SyntaxKind.ProtectedKeyword].includes(modifier.kind)) &&
          !ts.isPrivateIdentifier(member.name ?? ts.factory.createIdentifier('_')));
        declaration = ts.factory.updateClassDeclaration(node, node.modifiers, node.name, node.typeParameters, node.heritageClauses, members);
      }
      symbols.push(printer.printNode(ts.EmitHint.Unspecified, declaration, source));
    }
    if (symbols.length) result.push({name: relative(root, file).replace(/\.d\.ts$/, ''), declarations: symbols.join('\n\n')});
  }
}
walk(root);
const priority = (name) => ({hypervisor: 0, vm: 1, managed: 2, options: 3, resources: 4}[name] ?? (name.startsWith('models/') ? 20 : name.startsWith('operations/') ? 30 : 10));
process.stdout.write(JSON.stringify(result.sort((a, b) => priority(a.name) - priority(b.name) || a.name.localeCompare(b.name))));

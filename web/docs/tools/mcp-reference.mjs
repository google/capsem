// Read tools/list from the real server without invoking tools or contacting Capsem.
import {readFileSync, writeFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createServer} from '../../../mcp/typescript/dist/server.js';

const require = createRequire(new URL('../../../mcp/typescript/package.json', import.meta.url));
const {Client} = await import(require.resolve('@modelcontextprotocol/sdk/client/index.js'));
const {InMemoryTransport} = await import(require.resolve('@modelcontextprotocol/sdk/inMemory.js'));
const START = '<!-- MCP-TOOLS:START -->';
const END = '<!-- MCP-TOOLS:END -->';

export async function listHostTools() {
  const server = createServer({gatewayUrl: 'http://127.0.0.1:1', token: 'documentation-only', timeoutMs: 1000});
  const client = new Client({name: 'capsem-reference', version: '1'});
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  try {
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    const tools = [];
    let cursor;
    do {
      const page = await client.listTools(cursor ? {cursor} : {});
      tools.push(...page.tools);
      cursor = page.nextCursor;
    } while (cursor);
    if (!tools.length) throw new Error('MCP server returned no tools');
    return tools.sort((left, right) => left.name.localeCompare(right.name, 'en'));
  } finally {
    await client.close();
    await server.close();
  }
}

export function updateReference(page, tools) {
  if (page.split(START).length !== 2 || page.split(END).length !== 2 || page.indexOf(START) >= page.indexOf(END)) {
    throw new Error('Expected one ordered pair of MCP tool generation markers');
  }
  const schema = (label, value) => `<details>\n<summary>${label}</summary>\n\n\`\`\`json\n${JSON.stringify(value, null, 2)}\n\`\`\`\n\n</details>`;
  const entries = tools.map(tool => [
    `### \`${tool.name}\``,
    tool.description ?? '',
    schema('Input schema', tool.inputSchema),
    ...(tool.outputSchema ? [schema('Output schema', tool.outputSchema)] : []),
    ...(tool.annotations ? [schema('Tool annotations', tool.annotations)] : []),
  ].filter(Boolean).join('\n\n'));
  const body = `\n\n<!-- Generated from @capsem/mcp tools/list. Do not edit this block. -->\n\n${entries.join('\n\n')}\n\n`;
  return page.slice(0, page.indexOf(START) + START.length) + body + page.slice(page.indexOf(END));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const path = new URL('../src/content/docs/reference/mcp.md', import.meta.url);
  const before = readFileSync(path, 'utf8');
  const after = updateReference(before, await listHostTools());
  if (before !== after) {
    console.log('web/docs/src/content/docs/reference/mcp.md');
    if (process.argv.includes('--check')) process.exitCode = 1;
    else writeFileSync(path, after);
  }
}

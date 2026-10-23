import assert from 'node:assert/strict';
import test from 'node:test';
import {listHostTools, updateReference} from './mcp-reference.mjs';

test('reference includes every live tool and its complete declared schemas', async () => {
  const tools = await listHostTools();
  assert(tools.some(tool => tool.name === 'capsem_exec'));
  const page = updateReference('manual before\n<!-- MCP-TOOLS:START -->\nold\n<!-- MCP-TOOLS:END -->\nmanual after', tools);
  assert(page.startsWith('manual before\n'));
  assert(page.endsWith('\nmanual after'));
  for (const tool of tools) {
    assert(page.includes(`### \`${tool.name}\``));
    assert(page.includes(JSON.stringify(tool.inputSchema, null, 2)));
    if (tool.outputSchema) assert(page.includes(JSON.stringify(tool.outputSchema, null, 2)));
  }
  assert.equal(updateReference(page, tools), page);
});

test('missing or duplicated generation markers fail instead of overwriting prose', () => {
  assert.throws(() => updateReference('manual only', []), /marker/i);
  assert.throws(() => updateReference('<!-- MCP-TOOLS:START --><!-- MCP-TOOLS:START --><!-- MCP-TOOLS:END -->', []), /marker/i);
});

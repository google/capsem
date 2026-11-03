"""Programs executed outside the checkout by public registry consumers."""

PYTHON = '''\
import asyncio
import importlib.metadata as metadata
import os
from pathlib import Path
import capsem
from capsem import Hypervisor

assert metadata.version("capsem") == os.environ["CLIENT_VERSION"]
assert metadata.version("inspect-capsem-sandbox") == os.environ["INSPECT_VERSION"]
assert Path(capsem.__file__).resolve().is_relative_to(Path(os.sys.prefix))
entries = metadata.entry_points(group="inspect_ai", name="capsem")
assert len(entries) == 1
entries[0].load()
async def main():
    async with Hypervisor() as client:
        assert (await client.info()).gateway_version == os.environ["CLIENT_VERSION"]
asyncio.run(main())
print("PUBLIC_PYTHON_INSPECT_OK")
'''

NODE = '''\
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {Hypervisor} from '@capsem/sdk';

const sdk = new Hypervisor();
try {assert.equal((await sdk.info()).gateway_version, process.env.CLIENT_VERSION);}
finally {sdk.close();}
const require = createRequire(resolve('node_modules/@capsem/mcp/package.json'));
const {Client} = await import(pathToFileURL(require.resolve('@modelcontextprotocol/sdk/client/index.js')));
const {StdioClientTransport} = await import(pathToFileURL(require.resolve('@modelcontextprotocol/sdk/client/stdio.js')));
for (const name of ['sdk', 'mcp']) {
  const metadata = JSON.parse(readFileSync(`node_modules/@capsem/${name}/package.json`, 'utf8'));
  assert.equal(metadata.version, process.env.CLIENT_VERSION);
}
const run = process.env.CAPSEM_RUN_DIR;
const port = readFileSync(`${run}/gateway.port`, 'utf8').trim();
const transport = new StdioClientTransport({command: process.execPath,
  args: [resolve('node_modules/@capsem/mcp/dist/cli.js'), '--gateway-url', `http://127.0.0.1:${port}`,
         '--token-file', `${run}/gateway.token`, '--timeout-ms', '5000'],
  env: {PATH: process.env.PATH, HOME: process.env.HOME}, stderr: 'pipe'});
const client = new Client({name: 'public-registry-acceptance', version: '1'});
try {
  await client.connect(transport);
  assert.equal(client.getServerVersion()?.version, process.env.CLIENT_VERSION);
  assert((await client.listTools()).tools.some(tool => tool.name === 'capsem_status'));
  const result = await client.callTool({name: 'capsem_status', arguments: {}});
  assert(!result.isError);
  assert.equal(result.structuredContent.gateway_version, process.env.CLIENT_VERSION);
} finally {await client.close(); await transport.close();}
console.log('PUBLIC_NODE_MCP_OK');
'''

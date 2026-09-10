import {expect, it} from 'vitest';
import {ZodError} from 'zod';
import * as generated from '../src/operations/index.js';
import {MediaType, Transport} from '../src/transport.js';
import {routes, sample} from './contract.js';
import {gateway} from './gateway.js';

// Contract fixtures intentionally pass raw wire data through the real validators.
const operations: Record<string, (transport: Transport, ...parameters: never[]) => Promise<unknown>> = generated;

for (const {path, method, operation} of routes) {
  const content = operation.responses['200']?.content;
  if (!content) throw new Error('Operation has no successful response');
  const binary = MediaType.BINARY in content;
  it.each(binary ? ['minimal', 'complete', 'error'] : ['minimal', 'complete', 'error', 'invalid-json', 'invalid-shape'])(
    `${operation.operationId} executes its HTTP contract: %s`, async outcome => {
      const complete = outcome !== 'minimal';
      const schema = content[MediaType.JSON]?.schema;
      const expected = binary ? new Uint8Array([0, 255]) : sample(schema ?? {}, complete);
      const parameters: Record<string, unknown> = {};
      let expectedPath = path;
      const expectedQuery: Record<string, string> = {};
      for (const parameter of operation.parameters ?? []) {
        if (!parameter.required && !complete) continue;
        const value = parameter.name === 'id' ? 'a/b .. café' : sample(parameter.schema, complete);
        parameters[parameter.name] = value;
        if (parameter.in === 'path') expectedPath = expectedPath.replace(`{${parameter.name}}`, encodeURIComponent(String(value)).replaceAll('.', '%2E'));
        else expectedQuery[parameter.name] = Array.isArray(value) ? value.join(',') : String(value);
      }
      let expectedBody: Uint8Array | string = '';
      let contentType: string | undefined;
      if (operation.requestBody) {
        const bodyContent = operation.requestBody.content;
        if (MediaType.BINARY in bodyContent) {
          parameters.body = expectedBody = new Uint8Array([0, 255]);
          contentType = MediaType.BINARY;
        } else {
          parameters.body = sample(bodyContent[MediaType.JSON]?.schema ?? {}, complete);
          expectedBody = JSON.stringify(parameters.body);
          contentType = MediaType.JSON;
        }
      }
      await gateway((_, response) => {
        if (outcome === 'error') response.writeHead(403).end('denied');
        else if (outcome === 'invalid-json') response.end('{');
        else if (outcome === 'invalid-shape') response.end('null');
        else response.end(binary ? expected : JSON.stringify(expected));
      }, async (url, received) => {
        const transport = new Transport(url, 'secret');
        const call = operations[operation.operationId];
        if (!call) throw new Error('No generated operation');
        try {
          const promise = operation.parameters?.length || operation.requestBody
            ? call(transport, parameters as never) : call(transport);
          if (outcome === 'error') await expect(promise).rejects.toMatchObject({status: 403, body: 'denied'});
          else if (outcome === 'invalid-json') await expect(promise).rejects.toBeInstanceOf(SyntaxError);
          else if (outcome === 'invalid-shape') await expect(promise).rejects.toBeInstanceOf(ZodError);
          else expect(await promise).toEqual(expected);
          if (outcome === 'complete') {
            const args: unknown[] = operation.parameters?.length || operation.requestBody ? [parameters] : [];
            args.push({signal: AbortSignal.abort()});
            await expect(call(transport, ...args as never[])).rejects.toMatchObject({name: 'AbortError'});
          }
          expect(received).toHaveLength(1);
          const request = received[0];
          expect(request?.method).toBe(method.toUpperCase());
          expect(request?.url.split('?')[0]).toBe(expectedPath);
          expect(Object.fromEntries(new URL(request?.url ?? '/', url).searchParams)).toEqual(expectedQuery);
          expect(request?.body).toEqual(Buffer.from(expectedBody));
          expect(request?.headers.authorization).toBe('Bearer secret');
          expect(request?.headers.accept).toBe(binary ? MediaType.BINARY : MediaType.JSON);
          if (contentType) expect(request?.headers['content-type']).toBe(contentType);
        } finally {
          transport.close();
        }
      });
    },
  );
}

it('rejects invalid operation parameters before network IO', async () => {
  const transport = new Transport('http://127.0.0.1:1', 'secret');
  try {
    await expect(generated.getVmInfo(transport, {id: 12} as never)).rejects.toBeInstanceOf(ZodError);
    await expect(generated.getHypervisorLogs(transport, {name: 'unknown'} as never)).rejects.toBeInstanceOf(ZodError);
    await expect(generated.getVmChanges(transport, {id: 'vm', checkpoint: 'cp-10', limit: -1})).rejects.toBeInstanceOf(ZodError);
    await expect(generated.execVm(transport, {id: 'vm', body: {command: true}} as never)).rejects.toBeInstanceOf(ZodError);
  } finally {
    transport.close();
  }
});

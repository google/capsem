import {once} from 'node:events';
import {createServer, type IncomingHttpHeaders, type ServerResponse} from 'node:http';
import {buffer} from 'node:stream/consumers';

export interface Request {method: string; url: string; headers: IncomingHttpHeaders; body: Buffer}

export async function gateway(
  handle: (request: Request, response: ServerResponse) => void,
  run: (url: string, received: Request[]) => Promise<void>,
): Promise<void> {
  const received: Request[] = [];
  const server = createServer((request, response) => {
    void buffer(request).then(body => {
      const snapshot = {method: request.method ?? '', url: request.url ?? '', headers: request.headers, body};
      received.push(snapshot);
      handle(snapshot, response);
    }).catch((error: unknown) => response.destroy(error instanceof Error ? error : new Error(String(error))));
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  try {
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('Missing TCP address');
    await run(`http://127.0.0.1:${address.port}`, received);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
}

import type {ServerResponse} from 'node:http';
import {routes, sample, schemas} from './contract.js';
import type {Request} from './gateway.js';

export class FacadeGateway {
  names = ['chosen'];
  containerStates = ['running'];
  readonly files = new Map<string, Buffer>();

  handle(request: Request, response: ServerResponse): void {
    const url = new URL(request.url, 'http://localhost');
    const route = routes.find(route => route.method.toUpperCase() === request.method
      && new RegExp(`^${route.path.replace(/\{[^}]+\}/g, '[^/]+')}$`).test(url.pathname));
    if (!route) {response.writeHead(404).end('missing'); return;}
    const operation = route.operation;
    const status = operation.responses['200'] ? '200' : '202';
    const schema = operation.responses[status]?.content['application/json']?.schema;
    response.statusCode = Number(status);
    let value: unknown = sample(schema ?? schemas.UploadResponse ?? {});
    if (operation.operationId === 'listVms') value = {
      sandboxes: this.names.map(name => ({...sample(schemas.SandboxInfo ?? {}) as object, id: 'vm-0', name})),
    };
    if (operation.operationId === 'listProfiles') value = {
      profiles: [{...sample(schemas.ProfileSummary ?? {}) as object, id: 'code', name: 'Code'}],
    };
    if (operation.operationId === 'createVm' || operation.operationId === 'forkVm') {
      const body = JSON.parse(request.body.toString()) as {name: string};
      value = {...value as object, id: operation.operationId === 'createVm' ? 'vm-0' : 'fork-0', name: body.name ?? 'generated'};
    }
    if (operation.operationId === 'createNetwork') {
      value = {...value as object, name: (JSON.parse(request.body.toString()) as {name: string}).name};
    }
    if (operation.operationId === 'createVmExposure') {
      const body = JSON.parse(request.body.toString()) as {
        access: string; guest_port: number; host_port: number; target: string;
      };
      value = {...value as object, ...body,
        id: body.access === 'http_preview' ? 'preview-id' : '49152',
        host_port: body.access === 'http_preview' ? null : body.host_port || 49152};
    }
    if (operation.operationId === 'getVmContainer') {
      const state = this.containerStates.length > 1 ? this.containerStates.shift() : this.containerStates[0];
      value = {...value as object, image: 'docker://busybox:latest', state: state ?? 'running'};
    }
    if (operation.operationId === 'uploadVmFile') this.files.set(url.searchParams.get('path') ?? '', request.body);
    if (operation.operationId === 'downloadVmFile') {
      const data = this.files.get(url.searchParams.get('path') ?? '');
      if (data) response.end(data);
      else response.writeHead(404).end('missing');
    } else response.end(JSON.stringify(value));
  }
}

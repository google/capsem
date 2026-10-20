// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProxyHeartbeatResponse} from "../models/ProxyHeartbeatResponse.js";
import type {ProxyLeaseRequest} from "../models/ProxyLeaseRequest.js";
import {ProxyHeartbeatResponseSchema} from "../validation/ProxyHeartbeatResponse.js";
import {ProxyLeaseRequestSchema} from "../validation/ProxyLeaseRequest.js";

export async function heartbeatProxy(
  transport: Transport,
  parameters: {
    "id": string;
    "body": ProxyLeaseRequest;
  },
  options: CallOptions = {},
): Promise<ProxyHeartbeatResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => ProxyLeaseRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/proxies/{id}/heartbeat", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ProxyHeartbeatResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

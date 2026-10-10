// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProxyLeaseRequest} from "../models/ProxyLeaseRequest.js";
import type {StopProxyResponse} from "../models/StopProxyResponse.js";
import {ProxyLeaseRequestSchema} from "../validation/ProxyLeaseRequest.js";
import {StopProxyResponseSchema} from "../validation/StopProxyResponse.js";

export async function stopProxy(
  transport: Transport,
  parameters: {
    "id": string;
    "body": ProxyLeaseRequest;
  },
  options: CallOptions = {},
): Promise<StopProxyResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => ProxyLeaseRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/proxies/{id}/stop", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => StopProxyResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

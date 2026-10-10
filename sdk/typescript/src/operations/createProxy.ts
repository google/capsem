// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {CreateProxyRequest} from "../models/CreateProxyRequest.js";
import type {CreateProxyResponse} from "../models/CreateProxyResponse.js";
import {CreateProxyRequestSchema} from "../validation/CreateProxyRequest.js";
import {CreateProxyResponseSchema} from "../validation/CreateProxyResponse.js";

export async function createProxy(
  transport: Transport,
  parameters: {
    "body": CreateProxyRequest;
  },
  options: CallOptions = {},
): Promise<CreateProxyResponse> {
  const input = z.object({
  "body": z.lazy(() => CreateProxyRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/proxies", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => CreateProxyResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

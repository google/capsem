// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SecurityAskRequest} from "../models/SecurityAskRequest.js";
import type {SecurityAskResolveRequest} from "../models/SecurityAskResolveRequest.js";
import {SecurityAskRequestSchema} from "../validation/SecurityAskRequest.js";
import {SecurityAskResolveRequestSchema} from "../validation/SecurityAskResolveRequest.js";

export async function resolveSecurityAsk(
  transport: Transport,
  parameters: {
    "id": string;
    "ask_id": string;
    "body": SecurityAskResolveRequest;
  },
  options: CallOptions = {},
): Promise<SecurityAskRequest> {
  const input = z.object({
  "id": z.string(),
  "ask_id": z.string(),
  "body": z.lazy(() => SecurityAskResolveRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/security/asks/{ask_id}/resolve", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "ask_id": input["ask_id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => SecurityAskRequestSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

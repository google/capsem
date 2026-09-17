// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {PurgeRequest} from "../models/PurgeRequest.js";
import type {PurgeResponse} from "../models/PurgeResponse.js";
import {PurgeRequestSchema} from "../validation/PurgeRequest.js";
import {PurgeResponseSchema} from "../validation/PurgeResponse.js";

export async function purgeVms(
  transport: Transport,
  parameters: {
    "body": PurgeRequest;
  },
  options: CallOptions = {},
): Promise<PurgeResponse> {
  const input = z.object({
  "body": z.lazy(() => PurgeRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/purge", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => PurgeResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

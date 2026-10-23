// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {BrowserAcknowledgeRequest} from "../models/BrowserAcknowledgeRequest.js";
import type {BrowserAcknowledgeResponse} from "../models/BrowserAcknowledgeResponse.js";
import {BrowserAcknowledgeRequestSchema} from "../validation/BrowserAcknowledgeRequest.js";
import {BrowserAcknowledgeResponseSchema} from "../validation/BrowserAcknowledgeResponse.js";

export async function acknowledgeBrowserRequest(
  transport: Transport,
  parameters: {
    "id": string;
    "body": BrowserAcknowledgeRequest;
  },
  options: CallOptions = {},
): Promise<BrowserAcknowledgeResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => BrowserAcknowledgeRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/browser/acknowledge", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => BrowserAcknowledgeResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

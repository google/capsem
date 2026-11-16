// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {CreateAgentAccessRequest} from "../models/CreateAgentAccessRequest.js";
import type {CreateAgentAccessResponse} from "../models/CreateAgentAccessResponse.js";
import {CreateAgentAccessRequestSchema} from "../validation/CreateAgentAccessRequest.js";
import {CreateAgentAccessResponseSchema} from "../validation/CreateAgentAccessResponse.js";

export async function createAgentAccess(
  transport: Transport,
  parameters: {
    "body": CreateAgentAccessRequest;
  },
  options: CallOptions = {},
): Promise<CreateAgentAccessResponse> {
  const input = z.object({
  "body": z.lazy(() => CreateAgentAccessRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/agent-access", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => CreateAgentAccessResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

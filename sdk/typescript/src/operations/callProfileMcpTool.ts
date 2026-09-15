// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {Value} from "../models/Value.js";
import {ValueSchema} from "../validation/Value.js";

export async function callProfileMcpTool(
  transport: Transport,
  parameters: {
    "profile_id": string;
    "server_id": string;
    "tool_id": string;
    "body": Value;
  },
  options: CallOptions = {},
): Promise<Value> {
  const input = z.object({
  "profile_id": z.string(),
  "server_id": z.string(),
  "tool_id": z.string(),
  "body": z.lazy(() => ValueSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"], "server_id": input["server_id"], "tool_id": input["tool_id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ValueSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentCurrentResponse} from "../models/EnvironmentCurrentResponse.js";
import {EnvironmentCurrentResponseSchema} from "../validation/EnvironmentCurrentResponse.js";

export async function getEnvironmentInfo(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<EnvironmentCurrentResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => EnvironmentCurrentResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

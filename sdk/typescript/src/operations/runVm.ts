// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ExecResponse} from "../models/ExecResponse.js";
import type {RunRequest} from "../models/RunRequest.js";
import {ExecResponseSchema} from "../validation/ExecResponse.js";
import {RunRequestSchema} from "../validation/RunRequest.js";

export async function runVm(
  transport: Transport,
  parameters: {
    "body": RunRequest;
  },
  options: CallOptions = {},
): Promise<ExecResponse> {
  const input = z.object({
  "body": z.lazy(() => RunRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/run", {
    signal: options.signal, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ExecResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RestartResponse} from "../models/RestartResponse.js";
import {RestartResponseSchema} from "../validation/RestartResponse.js";

export async function restartHypervisor(
  transport: Transport,
  options: CallOptions = {},
): Promise<RestartResponse> {
  const payload = await transport.request(Method.POST, "/restart", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => RestartResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

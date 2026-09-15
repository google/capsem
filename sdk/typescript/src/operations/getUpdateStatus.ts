// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {UpdateStatusResponse} from "../models/UpdateStatusResponse.js";
import {UpdateStatusResponseSchema} from "../validation/UpdateStatusResponse.js";

export async function getUpdateStatus(
  transport: Transport,
  options: CallOptions = {},
): Promise<UpdateStatusResponse> {
  const payload = await transport.request(Method.GET, "/update/status", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => UpdateStatusResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

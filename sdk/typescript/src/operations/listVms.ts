// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ListResponse} from "../models/ListResponse.js";
import {ListResponseSchema} from "../validation/ListResponse.js";

export async function listVms(
  transport: Transport,
  options: CallOptions = {},
): Promise<ListResponse> {
  const payload = await transport.request(Method.GET, "/vms/list", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => ListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {NetworkListResponse} from "../models/NetworkListResponse.js";
import {NetworkListResponseSchema} from "../validation/NetworkListResponse.js";

export async function listNetworks(
  transport: Transport,
  options: CallOptions = {},
): Promise<NetworkListResponse> {
  const payload = await transport.request(Method.GET, "/networks", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => NetworkListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
